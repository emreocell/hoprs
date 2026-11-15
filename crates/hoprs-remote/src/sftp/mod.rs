//! SFTP backend: [`SftpFs`].
//!
//! Channels: metadata operations share one SFTP channel (russh-sftp multiplexes concurrent
//! requests on it). File streams use a pool of `stream_channels` SFTP channels, opened on
//! demand and handed out round-robin; several files may share one. Every channel counts
//! against the SSH connection's channel budget (`hoprs_ssh::channels`): the pool is capped
//! so that the metadata channel and the streams leave 2 slots for terminals, and when the
//! budget is full anyway (terminals took the room) a stream reuses an open pool channel.
//! Every channel remembers the session generation it was opened on. Before each operation a channel from an older
//! generation (or one that failed with a connection error) is dropped and reopened. Nothing
//! waits for a reconnection: while the session is not `Connected`, operations fail at once
//! with `ErrorKind::Network`.
//!
//! An open file keeps the channel it was opened on for its whole life, even if the pool
//! later replaces that channel; after a reconnection its requests fail with `Network`.
//! Each file handle is closed exactly once: by the writer's `shutdown`, by the reader on
//! EOF or error, or else in the background when the stream is dropped.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use hoprs_core::{EntryKind, Error, ErrorKind, RemoteEntry, RemotePath, Result};
use hoprs_ssh::{ChannelSlot, SessionState, SshSession, is_channel_limit};
use russh_sftp::client::RawSftpSession;
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::protocol::{FileAttributes, OpenFlags, Packet, Status, StatusCode};
use tokio::sync::{Mutex, watch};

use crate::{BoxAsyncRead, BoxAsyncWrite, RemoteFs};

mod pipeline;

use pipeline::{PipelinedReader, PipelinedWriter, SftpIo};

/// russh-sftp request timeout on stream channels. The clock starts when a request is
/// queued, not when it reaches the wire: with `MAX_IN_FLIGHT` x `CHUNK` (2 MiB) per stream and
/// several streams on one SSH connection, a request can wait behind many MiB of queued data
/// on a slow link. A short timeout would turn slowness into `Network` errors and pause the
/// site in a loop. A dead connection is detected by SSH keepalive instead, and the closed
/// channel then fails every request with `Network`.
const STREAM_REQUEST_TIMEOUT: Duration = Duration::from_secs(600);
/// russh-sftp request timeout on the metadata channel: small requests that never queue
/// behind file data, but a big `list` or a slow server still needs more than the default
/// 10 s. Keepalive covers dead connections here too.
const META_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

const POSIX_RENAME: &str = "posix-rename@openssh.com";
/// How long the server may take to answer the `sftp` subsystem request.
const SUBSYSTEM_TIMEOUT: Duration = Duration::from_secs(10);

const S_IFMT: u32 = 0o170_000;
const S_IFDIR: u32 = 0o040_000;
const S_IFREG: u32 = 0o100_000;
const S_IFLNK: u32 = 0o120_000;

/// SFTP options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SftpConfig {
    /// SFTP channels for file streams, clamped to `1..=8`, then capped by the session's
    /// channel budget ([`SshSession::sftp_stream_channels`]): with the default budget of 10,
    /// at most 6, so the metadata channel and the streams leave 3 channels: two terminals,
    /// and the one PTYs never take (see `hoprs_ssh::channels`).
    pub stream_channels: usize,
}

impl Default for SftpConfig {
    fn default() -> Self {
        Self { stream_channels: 4 }
    }
}

/// `RemoteFs` over SFTP on an [`SshSession`]. Cheap to clone; clones share the channels.
///
/// `stat` and `list` never follow symlinks (`lstat`). `list` leaves out FIFOs, sockets and
/// device files; `stat` on one fails with `ErrorKind::Io`. An entry whose attributes carry no
/// file type is reported as a file.
#[derive(Clone)]
pub struct SftpFs {
    inner: Arc<Inner>,
}

struct Inner {
    session: SshSession,
    state: watch::Receiver<SessionState>,
    meta: Mutex<Option<Arc<SftpChannel>>>,
    streams: Mutex<Vec<Option<Arc<SftpChannel>>>>,
    next_stream: AtomicUsize,
}

/// One SFTP subsystem channel.
struct SftpChannel {
    sftp: RawSftpSession,
    /// Session generation the channel was opened on.
    generation: u64,
    posix_rename: bool,
    /// Set after a connection error on this channel.
    dead: AtomicBool,
    /// File handles opened on this channel that we still track: incremented on open,
    /// decremented once the close was answered (or failed), or when a dropped stream finds
    /// no runtime to close on. In that last case the server handle stays open until the
    /// channel closes.
    open_handles: AtomicUsize,
    /// The channel's place in the session's channel budget, freed with the channel.
    _slot: ChannelSlot,
    /// russh-sftp per-request timeout set on this channel (after `init`). russh-sftp has no
    /// getter, so the tests read the value back from here.
    #[cfg(test)]
    request_timeout: Duration,
}

impl SftpChannel {
    /// Opens an SFTP channel whose requests (after `init`) time out after `request_timeout`.
    async fn open(session: &SshSession, request_timeout: Duration) -> Result<SftpChannel> {
        let (mut channel, slot) = session.open_channel().await?.into_parts();
        let generation = slot.generation();
        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(|e| start_error().with_detail(e.to_string()))?;
        let reply = tokio::time::timeout(SUBSYSTEM_TIMEOUT, async {
            loop {
                match channel.wait().await {
                    Some(russh::ChannelMsg::Success) => return Ok(()),
                    Some(russh::ChannelMsg::Failure) => {
                        return Err(Error::coded(
                            ErrorKind::Protocol,
                            "sftp-subsystem-refused",
                            "The server refused the SFTP subsystem",
                        ));
                    }
                    Some(_) => {}
                    None => return Err(start_error().with_detail("channel closed")),
                }
            }
        })
        .await;
        match reply {
            Ok(result) => result?,
            Err(_) => return Err(start_error().with_detail("subsystem request timed out")),
        }
        let sftp = RawSftpSession::new(channel.into_stream());
        let version = sftp
            .init()
            .await
            .map_err(|e| start_error().with_detail(e.to_string()))?;
        sftp.set_timeout(request_timeout.as_secs());
        let posix_rename = version
            .extensions
            .get(POSIX_RENAME)
            .is_some_and(|v| v == "1");
        Ok(SftpChannel {
            sftp,
            generation,
            posix_rename,
            dead: AtomicBool::new(false),
            open_handles: AtomicUsize::new(0),
            _slot: slot,
            #[cfg(test)]
            request_timeout,
        })
    }

    fn usable(&self, generation: u64) -> bool {
        self.generation == generation && !self.dead.load(Ordering::SeqCst)
    }

    /// Maps a russh-sftp result; a connection error marks the channel dead.
    fn check<T>(&self, result: std::result::Result<T, SftpError>, path: &RemotePath) -> Result<T> {
        result.map_err(|e| {
            let err = map_error(e, path);
            if err.kind == ErrorKind::Network {
                self.dead.store(true, Ordering::SeqCst);
            }
            err
        })
    }

    async fn lstat(&self, path: &RemotePath) -> Result<FileAttributes> {
        let attrs = self.sftp.lstat(path.as_str()).await;
        Ok(self.check(attrs, path)?.attrs)
    }

    async fn setstat(&self, path: &RemotePath, attrs: FileAttributes) -> Result<()> {
        let result = self.sftp.setstat(path.as_str(), attrs).await;
        self.check(result, path).map(|_| ())
    }

    /// `NoSuchFile` also covers `ENOTDIR` on OpenSSH: when `err` is `NotFound` but `path`
    /// exists and is not a directory, the real problem is that `path` is not a directory.
    async fn not_a_dir_or(&self, path: &RemotePath, err: Error) -> Error {
        if err.kind != ErrorKind::NotFound {
            return err;
        }
        let lstat = self.lstat(path).await;
        not_a_dir_error(err, lstat, path)
    }

    /// Opens `path` as an [`SftpFile`] and counts the handle.
    async fn open_file(
        self: &Arc<Self>,
        path: &RemotePath,
        flags: OpenFlags,
    ) -> Result<Arc<SftpFile>> {
        let opened = self
            .sftp
            .open(path.as_str(), flags, FileAttributes::empty())
            .await;
        let handle = self.check(opened, path)?.handle;
        self.open_handles.fetch_add(1, Ordering::SeqCst);
        Ok(Arc::new(SftpFile {
            chan: Arc::clone(self),
            handle,
            path: path.clone(),
            closed: AtomicBool::new(false),
        }))
    }
}

/// An open remote file: positioned I/O for the pipelined streams.
struct SftpFile {
    chan: Arc<SftpChannel>,
    handle: String,
    path: RemotePath,
    /// Set once a close was sent (or is about to be).
    closed: AtomicBool,
}

impl SftpFile {
    /// Sends the close and waits for the reply; the handle counts as closed either way.
    async fn close_handle(
        chan: Arc<SftpChannel>,
        handle: String,
    ) -> std::result::Result<Status, SftpError> {
        let result = chan.sftp.close(handle).await;
        chan.open_handles.fetch_sub(1, Ordering::SeqCst);
        result
    }
}

#[async_trait]
impl SftpIo for SftpFile {
    async fn read(&self, offset: u64, len: u32) -> Result<Vec<u8>> {
        match self.chan.sftp.read(self.handle.as_str(), offset, len).await {
            Ok(data) => Ok(data.data),
            Err(SftpError::Status(s)) if s.status_code == StatusCode::Eof => Ok(Vec::new()),
            Err(e) => self.chan.check(Err(e), &self.path),
        }
    }

    async fn write(&self, offset: u64, data: Vec<u8>) -> Result<()> {
        // Resolves on the server's status reply for this request.
        let result = self
            .chan
            .sftp
            .write(self.handle.as_str(), offset, data)
            .await;
        self.chan.check(result, &self.path).map(|_| ())
    }

    async fn close(&self) -> Result<()> {
        if self.closed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        let result = SftpFile::close_handle(Arc::clone(&self.chan), self.handle.clone()).await;
        self.chan.check(result, &self.path).map(|_| ())
    }
}

impl Drop for SftpFile {
    /// Closes a handle the stream never closed, in the background (never blocks).
    fn drop(&mut self) {
        if self.closed.swap(true, Ordering::SeqCst) {
            return;
        }
        let chan = Arc::clone(&self.chan);
        let handle = std::mem::take(&mut self.handle);
        match tokio::runtime::Handle::try_current() {
            Ok(rt) => {
                rt.spawn(async move {
                    let _ = SftpFile::close_handle(chan, handle).await;
                });
            }
            // No runtime: the handle lives until the channel closes.
            Err(_) => {
                chan.open_handles.fetch_sub(1, Ordering::SeqCst);
            }
        }
    }
}

/// Decision for [`SftpChannel::not_a_dir_or`]: `err` becomes "Not a directory" only when it
/// is `NotFound` and `lstat` found an entry that is not a directory.
fn not_a_dir_error(err: Error, lstat: Result<FileAttributes>, path: &RemotePath) -> Error {
    match lstat {
        Ok(attrs)
            if err.kind == ErrorKind::NotFound && entry_kind(&attrs) != Some(EntryKind::Dir) =>
        {
            Error::coded(
                ErrorKind::Io,
                "fs-not-a-directory",
                format!("Not a directory: {path}"),
            )
            .arg("path", path.to_string())
        }
        _ => err,
    }
}

fn start_error() -> Error {
    Error::coded(
        ErrorKind::Network,
        "sftp-start-failed",
        "Could not start SFTP",
    )
}

/// Maps an SFTP status: the message names `path`, the detail keeps the raw status.
fn map_status(status: &Status, path: &RemotePath) -> Error {
    let error = match status.status_code {
        StatusCode::NoSuchFile => Error::coded(
            ErrorKind::NotFound,
            "fs-no-such-file",
            format!("No such file or directory: {path}"),
        )
        .arg("path", path.to_string()),
        StatusCode::PermissionDenied => Error::coded(
            ErrorKind::Permission,
            "fs-permission-denied",
            format!("Permission denied: {path}"),
        )
        .arg("path", path.to_string()),
        StatusCode::NoConnection | StatusCode::ConnectionLost => Error::coded(
            ErrorKind::Network,
            "sftp-connection-lost",
            format!("SFTP connection lost: {path}"),
        )
        .arg("path", path.to_string()),
        StatusCode::Failure
        | StatusCode::BadMessage
        | StatusCode::OpUnsupported
        | StatusCode::Eof
        | StatusCode::Ok => Error::coded(
            ErrorKind::Io,
            "sftp-operation-failed",
            format!("SFTP operation failed: {path}"),
        )
        .arg("path", path.to_string()),
    };
    error.with_detail(format!(
        "{:?}: {}",
        status.status_code, status.error_message
    ))
}

fn map_error(err: SftpError, path: &RemotePath) -> Error {
    match err {
        SftpError::Status(status) => map_status(&status, path),
        // Channel gone (closed stream, dropped request) or no reply in time.
        SftpError::IO(_) | SftpError::UnexpectedBehavior(_) | SftpError::Timeout => Error::coded(
            ErrorKind::Network,
            "sftp-connection-lost",
            format!("SFTP connection lost: {path}"),
        )
        .arg("path", path.to_string())
        .with_detail(err.to_string()),
        SftpError::Limited(_) | SftpError::UnexpectedPacket => Error::coded(
            ErrorKind::Io,
            "sftp-operation-failed",
            format!("SFTP operation failed: {path}"),
        )
        .arg("path", path.to_string())
        .with_detail(err.to_string()),
    }
}

/// Fails unless `attrs` describe a regular file (no type information counts as one).
fn ensure_regular(attrs: &FileAttributes, path: &RemotePath) -> Result<()> {
    match entry_kind(attrs) {
        Some(EntryKind::File) => Ok(()),
        Some(EntryKind::Dir) => Err(Error::coded(
            ErrorKind::Io,
            "fs-is-a-directory",
            format!("Is a directory: {path}"),
        )
        .arg("path", path.to_string())),
        Some(EntryKind::Symlink) | None => Err(Error::coded(
            ErrorKind::Io,
            "fs-not-a-regular-file",
            format!("Not a regular file: {path}"),
        )
        .arg("path", path.to_string())),
    }
}

/// The file's size, or `Io` if it is shorter than `offset` (a resume there would leave a
/// gap of zero bytes).
fn check_resume_size(attrs: &FileAttributes, offset: u64, path: &RemotePath) -> Result<u64> {
    let size = attrs.size.unwrap_or(0);
    if size < offset {
        return Err(Error::coded(
            ErrorKind::Io,
            "sftp-cannot-resume-size",
            format!("Cannot resume at {offset}: {path} has only {size} bytes"),
        )
        .arg("offset", offset.to_string())
        .arg("path", path.to_string())
        .arg("size", size.to_string()));
    }
    Ok(size)
}

/// File type from the `S_IFMT` bits of the permissions. `None` for FIFOs, sockets and
/// devices. No type information (no permissions, or no type bits) counts as a file.
fn entry_kind(attrs: &FileAttributes) -> Option<EntryKind> {
    match attrs.permissions.map_or(0, |p| p & S_IFMT) {
        S_IFDIR => Some(EntryKind::Dir),
        S_IFLNK => Some(EntryKind::Symlink),
        S_IFREG | 0 => Some(EntryKind::File),
        _ => None,
    }
}

fn make_entry(
    name: String,
    path: RemotePath,
    kind: EntryKind,
    attrs: &FileAttributes,
) -> RemoteEntry {
    RemoteEntry {
        name,
        path,
        kind,
        size: if kind == EntryKind::File {
            attrs.size.unwrap_or(0)
        } else {
            0
        },
        modified: attrs
            .mtime
            .map(|t| UNIX_EPOCH + Duration::from_secs(u64::from(t))),
        mode: attrs.permissions.map(|p| p & 0o7777),
    }
}

impl SftpFs {
    /// Opens the metadata channel (`sftp` subsystem) on `session`.
    pub async fn new(session: SshSession, config: SftpConfig) -> Result<SftpFs> {
        let streams = session.sftp_stream_channels(config.stream_channels.clamp(1, 8));
        let meta = SftpChannel::open(&session, META_REQUEST_TIMEOUT).await?;
        Ok(SftpFs {
            inner: Arc::new(Inner {
                state: session.state(),
                session,
                meta: Mutex::new(Some(Arc::new(meta))),
                streams: Mutex::new(vec![None; streams]),
                next_stream: AtomicUsize::new(0),
            }),
        })
    }

    fn ensure_connected(&self) -> Result<()> {
        if *self.inner.state.borrow() == SessionState::Connected {
            Ok(())
        } else {
            Err(Error::coded(
                ErrorKind::Network,
                "net-not-connected",
                "Not connected",
            ))
        }
    }

    /// The metadata channel, reopened if stale.
    async fn meta(&self) -> Result<Arc<SftpChannel>> {
        self.ensure_connected()?;
        let mut slot = self.inner.meta.lock().await;
        if let Some(chan) = slot.as_ref()
            && chan.usable(self.inner.session.generation())
        {
            return Ok(chan.clone());
        }
        *slot = None;
        let chan = Arc::new(SftpChannel::open(&self.inner.session, META_REQUEST_TIMEOUT).await?);
        *slot = Some(chan.clone());
        Ok(chan)
    }

    /// The next stream channel, round-robin. After a reconnection every pooled channel is
    /// dropped; slots are (re)opened when picked. When the channel budget is full (the
    /// terminals took the room), an open pool channel is shared instead.
    async fn stream_channel(&self) -> Result<Arc<SftpChannel>> {
        self.ensure_connected()?;
        let generation = self.inner.session.generation();
        let mut pool = self.inner.streams.lock().await;
        if pool.iter().flatten().any(|c| c.generation != generation) {
            pool.iter_mut().for_each(|slot| *slot = None);
        }
        let index = self.inner.next_stream.fetch_add(1, Ordering::Relaxed) % pool.len();
        if let Some(chan) = &pool[index]
            && chan.usable(generation)
        {
            return Ok(chan.clone());
        }
        pool[index] = None;
        let chan = match SftpChannel::open(&self.inner.session, STREAM_REQUEST_TIMEOUT).await {
            Ok(chan) => Arc::new(chan),
            Err(e) if is_channel_limit(&e) => {
                return pool
                    .iter()
                    .flatten()
                    .find(|c| c.usable(generation))
                    .cloned()
                    .ok_or(e);
            }
            Err(e) => return Err(e),
        };
        pool[index] = Some(chan.clone());
        Ok(chan)
    }
}

#[async_trait]
impl RemoteFs for SftpFs {
    async fn list(&self, path: &RemotePath) -> Result<Vec<RemoteEntry>> {
        let chan = self.meta().await?;
        let opened = chan.sftp.opendir(path.as_str()).await;
        let handle = match chan.check(opened, path) {
            Ok(h) => h.handle,
            Err(e) => return Err(chan.not_a_dir_or(path, e).await),
        };
        let mut out = Vec::new();
        let result = loop {
            match chan.sftp.readdir(handle.as_str()).await {
                Ok(names) => {
                    for file in names.files {
                        let name = file.filename;
                        // `/` in a name would escape `path`; never valid from a sane server.
                        if name.is_empty() || name == "." || name == ".." || name.contains('/') {
                            continue;
                        }
                        let Some(kind) = entry_kind(&file.attrs) else {
                            continue;
                        };
                        let child = path.join(&name);
                        out.push(make_entry(name, child, kind, &file.attrs));
                    }
                }
                Err(SftpError::Status(s)) if s.status_code == StatusCode::Eof => break Ok(()),
                Err(e) => break chan.check(Err(e), path),
            }
        };
        // The listing is complete either way; a failed close only leaks a server handle.
        let _ = chan.sftp.close(handle).await;
        result.map(|()| out)
    }

    async fn stat(&self, path: &RemotePath) -> Result<RemoteEntry> {
        let chan = self.meta().await?;
        let attrs = chan.lstat(path).await?;
        let Some(kind) = entry_kind(&attrs) else {
            return Err(Error::coded(
                ErrorKind::Io,
                "fs-not-a-regular-file",
                format!("Not a regular file: {path}"),
            )
            .arg("path", path.to_string()));
        };
        let name = path.file_name().unwrap_or("/").to_owned();
        Ok(make_entry(name, path.clone(), kind, &attrs))
    }

    /// Refuses anything but a regular file (checked with `lstat` before opening, so a
    /// FIFO is never opened).
    async fn open_read(&self, path: &RemotePath, offset: u64) -> Result<BoxAsyncRead> {
        let chan = self.stream_channel().await?;
        ensure_regular(&chan.lstat(path).await?, path)?;
        let file = chan.open_file(path, OpenFlags::READ).await?;
        Ok(Box::new(PipelinedReader::new(file, offset)))
    }

    /// Offset 0 creates or truncates (an existing non-regular file is refused). A larger
    /// offset needs an existing regular file at least that long; a longer one is cut back to
    /// `offset` before writing.
    async fn open_write(&self, path: &RemotePath, offset: u64) -> Result<BoxAsyncWrite> {
        let chan = self.stream_channel().await?;
        let file = if offset == 0 {
            match chan.lstat(path).await {
                Ok(attrs) => ensure_regular(&attrs, path)?,
                Err(e) if e.kind == ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
            let flags = OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::TRUNCATE;
            chan.open_file(path, flags).await?
        } else {
            let attrs = match chan.lstat(path).await {
                Ok(attrs) => attrs,
                Err(e) if e.kind == ErrorKind::NotFound => {
                    return Err(Error::coded(
                        ErrorKind::Io,
                        "sftp-cannot-resume-missing",
                        format!("Cannot resume a missing file: {path}"),
                    )
                    .arg("path", path.to_string()));
                }
                Err(e) => return Err(e),
            };
            ensure_regular(&attrs, path)?;
            check_resume_size(&attrs, offset, path)?;
            let file = chan.open_file(path, OpenFlags::WRITE).await?;
            // The file may have changed since `lstat`: decide on what was opened. On error
            // `file` is dropped, which closes the handle.
            let opened = chan.sftp.fstat(file.handle.as_str()).await;
            let attrs = chan.check(opened, path)?.attrs;
            ensure_regular(&attrs, path)?;
            let size = check_resume_size(&attrs, offset, path)?;
            if size > offset {
                let attrs = FileAttributes {
                    size: Some(offset),
                    ..FileAttributes::empty()
                };
                let cut = chan.sftp.fsetstat(file.handle.as_str(), attrs).await;
                chan.check(cut, path)?;
            }
            file
        };
        Ok(Box::new(PipelinedWriter::new(file, offset)))
    }

    async fn mkdir(&self, path: &RemotePath) -> Result<()> {
        let chan = self.meta().await?;
        let created = chan
            .sftp
            .mkdir(path.as_str(), FileAttributes::empty())
            .await;
        match chan.check(created, path) {
            Ok(_) => Ok(()),
            Err(e) => match chan.lstat(path).await {
                Ok(attrs) if entry_kind(&attrs) == Some(EntryKind::Dir) => Ok(()),
                _ => Err(e),
            },
        }
    }

    async fn rename(&self, from: &RemotePath, to: &RemotePath) -> Result<()> {
        let chan = self.meta().await?;
        if !chan.posix_rename {
            let result = chan.sftp.rename(from.as_str(), to.as_str()).await;
            return chan.check(result, from).map(|_| ());
        }
        let mut data = Vec::new();
        for path in [from.as_str(), to.as_str()] {
            let len = u32::try_from(path.len()).map_err(|_| {
                Error::coded(
                    ErrorKind::Io,
                    "sftp-path-too-long",
                    format!("Path too long: {path}"),
                )
                .arg("path", path.to_string())
            })?;
            data.extend_from_slice(&len.to_be_bytes());
            data.extend_from_slice(path.as_bytes());
        }
        let reply = chan.sftp.extended(POSIX_RENAME, data).await;
        match chan.check(reply, from)? {
            Packet::Status(s) if s.status_code == StatusCode::Ok => Ok(()),
            Packet::Status(s) => Err(map_status(&s, from)),
            _ => chan.check(Err(SftpError::UnexpectedPacket), from),
        }
    }

    async fn remove_file(&self, path: &RemotePath) -> Result<()> {
        let chan = self.meta().await?;
        let result = chan.sftp.remove(path.as_str()).await;
        chan.check(result, path).map(|_| ())
    }

    async fn remove_dir(&self, path: &RemotePath) -> Result<()> {
        let chan = self.meta().await?;
        let result = chan.sftp.rmdir(path.as_str()).await;
        match chan.check(result, path) {
            Ok(_) => Ok(()),
            Err(e) => Err(chan.not_a_dir_or(path, e).await),
        }
    }

    async fn chmod(&self, path: &RemotePath, mode: u32) -> Result<()> {
        let chan = self.meta().await?;
        let attrs = FileAttributes {
            permissions: Some(mode),
            ..FileAttributes::empty()
        };
        chan.setstat(path, attrs).await
    }

    async fn set_modified(&self, path: &RemotePath, mtime: SystemTime) -> Result<()> {
        let secs = mtime
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|d| u32::try_from(d.as_secs()).ok())
            .ok_or_else(|| {
                Error::coded(
                    ErrorKind::Io,
                    "sftp-mtime-out-of-range",
                    format!("Modification time out of range for SFTP: {path}"),
                )
                .arg("path", path.to_string())
            })?;
        let chan = self.meta().await?;
        let attrs = FileAttributes {
            atime: Some(secs),
            mtime: Some(secs),
            ..FileAttributes::empty()
        };
        chan.setstat(path, attrs).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(code: StatusCode) -> Status {
        Status {
            id: 7,
            status_code: code,
            error_message: "server says".into(),
            language_tag: "en".into(),
        }
    }

    #[test]
    fn status_mapping() {
        let path = RemotePath::new("/a/b").unwrap();
        for (code, kind) in [
            (StatusCode::NoSuchFile, ErrorKind::NotFound),
            (StatusCode::PermissionDenied, ErrorKind::Permission),
            (StatusCode::Failure, ErrorKind::Io),
            (StatusCode::BadMessage, ErrorKind::Io),
            (StatusCode::OpUnsupported, ErrorKind::Io),
            (StatusCode::NoConnection, ErrorKind::Network),
            (StatusCode::ConnectionLost, ErrorKind::Network),
        ] {
            let e = map_error(SftpError::Status(status(code)), &path);
            assert_eq!(e.kind, kind, "{code:?}");
            assert!(e.message.ends_with(": /a/b"), "{e:?}");
            assert_eq!(
                e.detail.as_deref(),
                Some(format!("{code:?}: server says").as_str())
            );
        }
    }

    #[test]
    fn transport_errors_are_network() {
        let path = RemotePath::new("/x").unwrap();
        for err in [
            SftpError::Timeout,
            SftpError::IO("broken pipe".into()),
            SftpError::UnexpectedBehavior("session closed".into()),
        ] {
            let e = map_error(err, &path);
            assert_eq!(e.kind, ErrorKind::Network, "{e:?}");
            assert!(e.message.contains("/x"), "{e:?}");
        }
        assert_eq!(
            map_error(SftpError::UnexpectedPacket, &path).kind,
            ErrorKind::Io
        );
    }

    #[test]
    fn file_type_from_mode_bits() {
        let attrs = |permissions| FileAttributes {
            permissions,
            ..FileAttributes::empty()
        };
        assert_eq!(entry_kind(&attrs(Some(0o040_755))), Some(EntryKind::Dir));
        assert_eq!(entry_kind(&attrs(Some(0o100_644))), Some(EntryKind::File));
        assert_eq!(
            entry_kind(&attrs(Some(0o120_777))),
            Some(EntryKind::Symlink)
        );
        for special in [0o010_644, 0o140_755, 0o020_600, 0o060_660] {
            assert_eq!(entry_kind(&attrs(Some(special))), None, "{special:o}");
        }
        assert_eq!(entry_kind(&attrs(None)), Some(EntryKind::File));
        assert_eq!(entry_kind(&attrs(Some(0o644))), Some(EntryKind::File));
    }

    #[test]
    fn not_a_dir_decision() {
        let path = RemotePath::new("/a/b").unwrap();
        let not_found = || Error::new(ErrorKind::NotFound, "No such file or directory: /a/b");
        let attrs = |permissions| FileAttributes {
            permissions: Some(permissions),
            ..FileAttributes::empty()
        };
        let missing = Err(not_found());
        let e = not_a_dir_error(not_found(), missing, &path);
        assert_eq!(e.kind, ErrorKind::NotFound, "{e:?}");
        let e = not_a_dir_error(not_found(), Ok(attrs(0o040_755)), &path);
        assert_eq!(e.kind, ErrorKind::NotFound, "{e:?}");
        let e = not_a_dir_error(not_found(), Ok(attrs(0o100_644)), &path);
        assert_eq!(e.kind, ErrorKind::Io, "{e:?}");
        assert_eq!(e.message, "Not a directory: /a/b");
        // Only NotFound is reinterpreted.
        let denied = Error::new(ErrorKind::Permission, "Permission denied: /a/b");
        let e = not_a_dir_error(denied, Ok(attrs(0o100_644)), &path);
        assert_eq!(e.kind, ErrorKind::Permission, "{e:?}");
    }

    #[test]
    fn request_timeouts() {
        assert_eq!(STREAM_REQUEST_TIMEOUT, Duration::from_secs(600));
        assert_eq!(META_REQUEST_TIMEOUT, Duration::from_secs(60));
    }

    #[tokio::test]
    #[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
    async fn channels_use_their_request_timeouts() {
        let (session, _dir) = alice().await;
        let fs = SftpFs::new(session.clone(), SftpConfig::default())
            .await
            .unwrap();
        let meta = fs.meta().await.unwrap();
        assert_eq!(meta.request_timeout, META_REQUEST_TIMEOUT);
        for _ in 0..4 {
            let chan = fs.stream_channel().await.unwrap();
            assert_eq!(chan.request_timeout, STREAM_REQUEST_TIMEOUT);
        }
        session.disconnect().await;
    }

    #[test]
    fn entry_fields() {
        let attrs = FileAttributes {
            size: Some(42),
            permissions: Some(0o104_755),
            mtime: Some(1_000_000),
            ..FileAttributes::empty()
        };
        let path = RemotePath::new("/f").unwrap();
        let e = make_entry("f".into(), path.clone(), EntryKind::File, &attrs);
        assert_eq!(e.mode, Some(0o4755));
        assert_eq!(e.size, 42);
        assert_eq!(
            e.modified,
            Some(UNIX_EPOCH + Duration::from_secs(1_000_000))
        );
        let d = make_entry("f".into(), path, EntryKind::Dir, &attrs);
        assert_eq!(d.size, 0);
    }

    async fn alice() -> (SshSession, tempfile::TempDir) {
        use hoprs_ssh::testing::{fixture, test_config};
        use hoprs_ssh::{KeySource, Prompter, ReconnectPolicy, ScriptedPrompter};
        let dir = tempfile::TempDir::new().unwrap();
        let mut cfg = test_config("alice", dir.path());
        cfg.auth.key = KeySource::File {
            path: fixture("id_ed25519"),
            passphrase: None,
        };
        cfg.reconnect = ReconnectPolicy {
            max_attempts: 5,
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_millis(400),
        };
        let prompter = Arc::new(ScriptedPrompter::new().accept_host(true));
        let session = SshSession::connect(cfg, prompter as Arc<dyn Prompter>)
            .await
            .unwrap();
        (session, dir)
    }

    #[tokio::test]
    #[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
    async fn stream_channels_clamped() {
        let (session, _dir) = alice().await;
        // At most 6 under the default budget of 10: metadata + 6 leaves 3 (two terminals
        // and the spare PTYs never take).
        for (asked, got) in [(0, 1), (1, 1), (4, 4), (6, 6), (7, 6), (8, 6), (100, 6)] {
            let config = SftpConfig {
                stream_channels: asked,
            };
            let fs = SftpFs::new(session.clone(), config).await.unwrap();
            assert_eq!(fs.inner.streams.lock().await.len(), got, "{asked}");
        }
        session.disconnect().await;
    }

    /// The SFTP channels leave room for terminals, and share a stream channel when the
    /// terminals took the rest of the budget (Review Focus 5).
    #[tokio::test]
    #[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
    async fn sftp_channels_leave_room_for_terminals() {
        use hoprs_ssh::PtyRequest;
        let (session, _dir) = alice().await;
        assert_eq!(session.max_channels(), 10);
        let fs = SftpFs::new(session.clone(), SftpConfig { stream_channels: 8 })
            .await
            .unwrap();
        let mut streams = Vec::new();
        for _ in 0..6 {
            streams.push(fs.stream_channel().await.unwrap());
        }
        assert_eq!(session.open_channels(), 7, "metadata + 6 streams");
        // Two terminals fit next to a full SFTP (ruling R2).
        let first = session.open_shell(PtyRequest::default()).await.unwrap();
        let second = session.open_shell(PtyRequest::default()).await.unwrap();
        // The last slot is never a PTY's (the metadata channel may need it again).
        let e = session.open_shell(PtyRequest::default()).await.unwrap_err();
        assert!(hoprs_ssh::is_channel_limit(&e), "{e:?}");
        assert_eq!(session.open_channels(), 9);
        drop((first, second, streams));
        session.disconnect().await;

        // Terminals first: a stream that cannot get its own channel shares one.
        let (session, _dir) = alice().await;
        let fs = SftpFs::new(session.clone(), SftpConfig { stream_channels: 2 })
            .await
            .unwrap();
        let a = fs.stream_channel().await.unwrap();
        let mut shells = Vec::new();
        let refused = loop {
            match session.open_shell(PtyRequest::default()).await {
                Ok(shell) => shells.push(shell),
                Err(e) => break e,
            }
        };
        assert!(hoprs_ssh::is_channel_limit(&refused), "{refused:?}");
        assert_eq!(shells.len(), 7, "PTYs stop one short of the limit");
        // Something else takes the last slot: the budget is full.
        let _last = session.open_channel().await.unwrap();
        assert_eq!(session.open_channels(), session.max_channels());
        let b = fs.stream_channel().await.unwrap();
        assert!(
            Arc::ptr_eq(&a, &b),
            "the full budget shares the open stream"
        );
        assert!(b.lstat(&RemotePath::root()).await.is_ok());
        assert!(fs.list(&RemotePath::root()).await.is_ok());
        drop(shells);
        session.disconnect().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
    async fn stream_pool_round_robin_and_reopen() {
        let (session, _dir) = alice().await;
        let config = SftpConfig { stream_channels: 2 };
        let fs = SftpFs::new(session.clone(), config).await.unwrap();
        let a = fs.stream_channel().await.unwrap();
        let b = fs.stream_channel().await.unwrap();
        assert!(!Arc::ptr_eq(&a, &b));
        assert!(Arc::ptr_eq(&a, &fs.stream_channel().await.unwrap()));
        assert!(Arc::ptr_eq(&b, &fs.stream_channel().await.unwrap()));
        let root = RemotePath::root();
        for chan in [&a, &b] {
            assert_eq!(chan.generation, 1);
            assert!(chan.lstat(&root).await.is_ok());
        }

        let mut rx = session.state();
        hoprs_ssh::testing::kill_ssh_sessions();
        tokio::time::timeout(
            Duration::from_secs(20),
            rx.wait_for(|s| matches!(s, SessionState::Reconnecting { .. })),
        )
        .await
        .unwrap()
        .unwrap();
        tokio::time::timeout(
            Duration::from_secs(20),
            rx.wait_for(|s| *s == SessionState::Connected),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(session.generation(), 2);

        let c = fs.stream_channel().await.unwrap();
        let d = fs.stream_channel().await.unwrap();
        for chan in [&c, &d] {
            assert_eq!(chan.generation, 2);
            assert!(!Arc::ptr_eq(chan, &a) && !Arc::ptr_eq(chan, &b));
            assert!(chan.lstat(&root).await.is_ok());
        }
        assert!(!Arc::ptr_eq(&c, &d));
        // The old channels are gone for good.
        assert_eq!(a.lstat(&root).await.unwrap_err().kind, ErrorKind::Network);
        session.disconnect().await;
    }

    async fn wait_no_open_handles(chan: &SftpChannel) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while chan.open_handles.load(Ordering::SeqCst) != 0 {
            assert!(
                tokio::time::Instant::now() < deadline,
                "{} handles still open",
                chan.open_handles.load(Ordering::SeqCst)
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    #[tokio::test]
    #[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
    async fn every_handle_is_closed_once() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (session, _dir) = alice().await;
        let config = SftpConfig { stream_channels: 1 };
        let fs = SftpFs::new(session.clone(), config).await.unwrap();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = RemotePath::new(&format!("/home/alice/handles-{nanos}")).unwrap();
        let chan = fs.stream_channel().await.unwrap();

        // Shutdown closes the writer's handle.
        let mut w = fs.open_write(&path, 0).await.unwrap();
        assert_eq!(chan.open_handles.load(Ordering::SeqCst), 1);
        w.write_all(&[7u8; 100_000]).await.unwrap();
        w.shutdown().await.unwrap();
        assert_eq!(chan.open_handles.load(Ordering::SeqCst), 0);
        drop(w);

        // Dropping without reading closes it in the background.
        for _ in 0..200 {
            let r = fs.open_read(&path, 0).await.unwrap();
            assert!(chan.open_handles.load(Ordering::SeqCst) >= 1);
            drop(r);
        }
        wait_no_open_handles(&chan).await;

        // Reaching EOF closes it, before the reader is dropped.
        let mut r = fs.open_read(&path, 0).await.unwrap();
        let mut out = Vec::new();
        r.read_to_end(&mut out).await.unwrap();
        assert_eq!(out.len(), 100_000);
        assert_eq!(chan.open_handles.load(Ordering::SeqCst), 0);
        drop(r);

        // A writer dropped without shutdown is closed too.
        let mut w = fs.open_write(&path, 0).await.unwrap();
        w.write_all(b"abc").await.unwrap();
        drop(w);
        wait_no_open_handles(&chan).await;

        fs.remove_file(&path).await.unwrap();
        session.disconnect().await;
    }

    #[test]
    fn status_errors_carry_a_code_and_the_path() {
        let path = RemotePath::new("/a/b").unwrap();
        let denied = map_status(&status(StatusCode::PermissionDenied), &path);
        assert_eq!(denied.code, Some("fs-permission-denied"));
        assert_eq!(denied.args, vec![("path", "/a/b".to_owned())]);
        let lost = map_status(&status(StatusCode::ConnectionLost), &path);
        assert_eq!(lost.code, Some("sftp-connection-lost"));
    }
}
