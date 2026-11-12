//! FTP and FTPS backend: [`FtpFs`].

mod config;
pub(crate) mod errors;
mod list;
mod prompt;
pub(crate) mod session;
mod stream;
mod tls;

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

use async_trait::async_trait;
use hoprs_core::{ConnectionState, EntryKind, Error, ErrorKind, RemoteEntry, RemotePath, Result};
use tokio::sync::watch;

pub use config::{FtpConfig, FtpSecurity, TransferMode};
pub use prompt::{CertProblem, FtpPrompter};
#[cfg(feature = "testing")]
pub use prompt::{CertificatePrompt, FtpPromptCalls, ScriptedFtpPrompter};
pub use tls::fingerprint;

use crate::{BoxAsyncRead, BoxAsyncWrite, Connection, RemoteFs};
use errors::{is_control_loss, map_ftp_error, map_reply};
use list::{
    ListedEntry, StatReply, classify_stat_reply, format_timeval, is_dot_line, parse_list_line,
    parse_mlsd_line, parse_timeval, unreadable_listing,
};
use session::{FtpSession, PooledConn};
use stream::{FtpReader, FtpWriter, Timeouts, Transfer};

/// `RemoteFs` over FTP or FTPS. Cheap to clone; clones share one [`FtpSession`].
///
/// Listings and metadata commands run on the session's control connection, one at a time.
/// Paths are always sent absolute; the working directory is never changed.
///
/// - `list` uses `MLSD` when `FEAT` announces it, `LIST -a` otherwise (unix `ls -l` format).
///   `.`, `..`, FIFOs, sockets and devices are left out; symlinks are reported, not followed.
/// - `stat` never lists a directory for a file (rulings R9, R10): `SIZE` first, then
///   `STAT <path>`, which lists the path itself on the control connection (no data
///   connection; `SIZE` follows symlinks, `STAT` does not). A symlink is reported as such; a
///   regular file gets the size from `SIZE`, permissions from the `STAT` line and its exact
///   modification time from `MDTM` (the line's date when `MDTM` fails).
/// - A missing path costs no listing either (ruling R11): `SIZE` says `550` and `STAT` sends
///   a multi-line `213` with nothing in it (vsftpd, under a missing parent too): `NotFound`.
///   Trade-off: under a parent the server cannot search, `STAT` lists nothing as well, so the
///   result is `NotFound` rather than `Permission`. Nothing is overwritten because of it: the
///   `STOR` that follows fails, and is reported as `Permission`. Failures are explained with
///   a `stat` that confirms a missing path with a listing, so they keep `Permission`.
/// - Otherwise (`SIZE` not supported: `500`/`502`; `STAT` refused, or not one entry with that
///   name; a directory or special file) `stat` lists the parent directory and looks the name
///   up: directories, special files, unreadable parents. `/` is a directory without asking.
/// - A `LIST` reply this parser cannot read (an MS-DOS style listing, IIS's default) is a
///   `Protocol` error, "Cannot read the server's listing format", and so is a `stat` whose
///   `SIZE` found the file when the listing does not show it: a listing that cannot be read
///   never makes a directory look empty or an existing file look missing.
/// - Servers answer most failures with a bare `550` (vsftpd: "Delete operation failed."), so
///   an ambiguous failure is explained with a `stat`: a missing path is `NotFound`, the wrong
///   kind of entry `Io`.
/// - `set_modified` uses `MFMT` when `FEAT` announces it, and is a no-op otherwise.
///
/// Transfers run on the session's pool of transfer connections, never on the control
/// connection, in binary mode (see `stream`):
///
/// - Opening never lists a directory (ruling R8): one `SIZE` on the control connection, and
///   a `stat` only to explain a refusal.
/// - `open_read` gets the size with `SIZE`; the server refuses directories and special files
///   (vsftpd: `550`), which `stat` then reports as `Io` (or `NotFound`, `Permission`). At or
///   past the size the stream is empty without a `RETR`, otherwise `REST` (when
///   `offset > 0`) and `RETR`. The stream ends only after the final `226`/`250`.
/// - `open_write` with offset 0 sends `STOR` at once, so the file is created or truncated
///   before it returns. A larger offset must equal the file's `SIZE`, then `APPE` continues
///   it.
/// - Known edge (R8): `SIZE` and `STOR` follow symlinks on the server, so a symlink to a file
///   is read or written like the file. Writing to a FIFO is left to the server (vsftpd
///   refuses it with `553`; a server that opens it would block that transfer). The transfer
///   engine never gets there: `stat` reports the symlink and it refuses the destination.
/// - A write stream's `shutdown` flushes the 256 KiB buffer, closes the data connection and
///   returns `Ok` only after the `226`. A refusal from the server (`451`, `452`, `552`: a full
///   disk) is `Io`; a lost connection is `Network`. A `STOR` refused (`550`, `553`) for a
///   missing file in an existing directory is `Permission`.
/// - Passive mode sends `PASV` (`EPSV` to an IPv6 server). With `pasv_nat_workaround` the
///   announced address is replaced by the control connection's peer. Active mode sends
///   `PORT`/`EPRT` with the control connection's local address.
/// - FTPS data connections resume the control connection's TLS session (rustls' session
///   cache), as vsftpd's default `require_ssl_reuse=YES` demands.
/// - A data connection that keeps a stream waiting for `data_timeout` (no byte received or
///   accepted) fails it with `Network`, "Connection lost: no data for N s", and closes its
///   connection, which frees its pool slot.
/// - A stream dropped before its end closes its connection after a best-effort `ABOR`; only a
///   transfer that ended with a positive reply returns its connection to the pool.
///
/// **Do not cancel these futures midway.** A command dropped before its reply leaves the
/// control connection in an unknown state, so the session discards it and reconnects (see
/// `FtpSession::with_control`): the user would see a reconnection.
#[derive(Clone)]
pub struct FtpFs {
    session: FtpSession,
    /// Directory listings sent so far (tests check that opens send none).
    listings: Arc<AtomicU64>,
}

impl FtpFs {
    /// Connects and logs in (see [`FtpSession::connect`]).
    pub async fn connect(cfg: FtpConfig, prompter: Arc<dyn FtpPrompter>) -> Result<FtpFs> {
        let session = FtpSession::connect(cfg, prompter).await?;
        Ok(FtpFs {
            session,
            listings: Arc::new(AtomicU64::new(0)),
        })
    }

    /// Closes every connection and stops reconnecting.
    pub async fn disconnect(&self) {
        self.session.disconnect().await;
    }

    /// The raw lines of `dir`'s listing, and whether they came from `MLSD`.
    async fn listing(&self, dir: &RemotePath) -> Result<(Vec<String>, bool)> {
        let arg = arg(dir)?.to_owned();
        self.listings.fetch_add(1, Ordering::Relaxed);
        let context = dir.to_string();
        self.session
            .with_control(move |c| {
                Box::pin(async move {
                    let mlsd = c.has_mlsd;
                    let lines = if mlsd {
                        c.stream.mlsd(Some(&arg)).await
                    } else {
                        // `-a`: dot files, and the `.` entry that proves `dir` is a directory.
                        c.stream.list(Some(&format!("-a {arg}"))).await
                    };
                    lines
                        .map(|lines| (lines, mlsd))
                        .map_err(|e| map_ftp_error(e, &context))
                })
            })
            .await
    }

    /// The parsed entries of `dir`, special files included. A `LIST` reply in a format the
    /// parser cannot read is a `Protocol` error, never an empty directory.
    async fn entries(&self, dir: &RemotePath) -> Result<Listing> {
        let (lines, mlsd) = self.listing(dir).await?;
        if mlsd {
            return Ok(Listing {
                entries: lines.iter().filter_map(|l| parse_mlsd_line(l)).collect(),
                is_dir: true,
                mlsd,
            });
        }
        let now = SystemTime::now();
        if unreadable_listing(&lines, now) {
            return Err(listing_format_error(dir, &lines));
        }
        Ok(Listing {
            entries: lines
                .iter()
                .filter_map(|l| parse_list_line(l, now))
                .collect(),
            is_dir: lines.iter().any(|l| is_dot_line(l, now)),
            mlsd,
        })
    }

    /// Sends `command` on the control connection and returns the reply's code and text,
    /// whatever the code. Socket errors are mapped with `path` as context.
    async fn reply(&self, command: String, path: &RemotePath) -> Result<(u32, String)> {
        let context = path.to_string();
        self.session
            .with_control(move |c| {
                Box::pin(async move { session::quote(&mut c.stream, &command, &context).await })
            })
            .await
    }

    /// Sends `{verb} {path}`; a reply of 400 or more is an error, explained with
    /// [`explain`](Self::explain).
    async fn simple(&self, verb: &str, path: &RemotePath) -> Result<()> {
        let (code, text) = self.reply(format!("{verb} {}", arg(path)?), path).await?;
        if code < 400 {
            return Ok(());
        }
        Err(self
            .explain(path, map_reply(code, &text, path.as_str()))
            .await)
    }

    /// Turns an ambiguous failure (`Io` or `NotFound`, the usual bare `550`) on `path` into
    /// `NotFound` when `path` does not exist. When the `stat` itself fails with `Network` or
    /// `Permission` (an unreadable parent), that error is the better answer. Anything else is
    /// returned as is.
    async fn explain(&self, path: &RemotePath, err: Error) -> Error {
        if !matches!(err.kind, ErrorKind::Io | ErrorKind::NotFound) {
            return err;
        }
        match self.stat_exact(path).await {
            Err(e) if e.kind == ErrorKind::NotFound => not_found(path, &err),
            Err(e) if is_definite(&e) => e,
            _ => err,
        }
    }

    /// [`explain`](Self::explain) for a refused `STOR`/`APPE`. A refusal (`550`, `553`) of a
    /// missing file whose parent is a directory is `Permission`: the server could not create
    /// the file there (vsftpd: `553 Could not create file.` in a read-only directory).
    async fn explain_write(&self, path: &RemotePath, err: Error) -> Error {
        let refused = err
            .detail
            .as_deref()
            .is_some_and(|d| d.starts_with("550 ") || d.starts_with("553 "));
        let err = self.explain(path, err).await;
        if !refused || err.kind != ErrorKind::NotFound {
            return err;
        }
        let Some(parent) = path.parent() else {
            return err;
        };
        match self.stat_exact(&parent).await {
            Ok(st) if st.kind == EntryKind::Dir => {
                let e = Error::new(ErrorKind::Permission, format!("Permission denied: {path}"));
                match err.detail {
                    Some(detail) => e.with_detail(detail),
                    None => e,
                }
            }
            _ => err,
        }
    }

    #[cfg(feature = "testing")]
    pub(crate) fn listings(&self) -> u64 {
        self.listings.load(Ordering::Relaxed)
    }

    #[cfg(feature = "testing")]
    pub(crate) fn session(&self) -> &FtpSession {
        &self.session
    }

    /// The size of the regular file `path`, with one `SIZE` on the control connection. vsftpd
    /// answers it with `stat()` (a symlink is followed, nothing is opened) and refuses
    /// directories and special files with `550`. Only a refusal is explained with a `stat`
    /// (a listing of the parent): `NotFound`, `Io` "Is a directory" / "Not a regular file",
    /// or `Permission`. A server without `SIZE` is asked with `stat` alone.
    async fn file_size(&self, path: &RemotePath) -> Result<u64> {
        let (code, text) = self.reply(format!("SIZE {}", arg(path)?), path).await?;
        if code == 213 {
            return parse_size(code, &text, path.as_str());
        }
        if matches!(code, 500 | 502) {
            let st = self.stat_listed(path).await?;
            regular_file(&st, path)?;
            return Ok(st.size);
        }
        let err = map_reply(code, &text, path.as_str());
        if !matches!(err.kind, ErrorKind::Io | ErrorKind::NotFound) {
            return Err(err);
        }
        match self.stat_listed(path).await {
            Err(e) if e.kind == ErrorKind::NotFound => Err(not_found(path, &err)),
            Err(e) => Err(e),
            Ok(st) => {
                regular_file(&st, path)?;
                Err(err)
            }
        }
    }

    /// `stat` from a listing of the parent directory: directories, symlinks, special files,
    /// missing paths and unreadable parents (`Permission`). `path` is not `/`.
    async fn stat_listed(&self, path: &RemotePath) -> Result<RemoteEntry> {
        let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
            return Ok(root_entry());
        };
        let Listing {
            entries,
            is_dir,
            mlsd,
        } = match self.entries(&parent).await {
            Ok(listing) => listing,
            // `MLSD` refused: the parent is missing or not a directory.
            Err(e) if matches!(e.kind, ErrorKind::Io | ErrorKind::NotFound) => {
                return Err(not_found(path, &e));
            }
            Err(e) => return Err(e),
        };
        // Without a `.` entry the listing may be the parent's own line (vsftpd lists a file, or
        // a directory it cannot open, that way): never look the child up in it.
        if !is_dir
            && let [only] = entries.as_slice()
            && Some(only.name.as_str()) == parent.file_name()
        {
            let up = self.stat_exact(&parent).await?;
            match up.kind {
                // The same check as `list`: the directory's own entry, so it is unreadable.
                EntryKind::Dir
                    if only.kind == Some(EntryKind::Dir)
                        && only.mode == up.mode
                        && only.modified == up.modified =>
                {
                    return Err(Error::new(
                        ErrorKind::Permission,
                        format!("Permission denied: {path}"),
                    ));
                }
                // A real child that happens to share the directory's name.
                EntryKind::Dir => {}
                // Nothing lives under a file (SftpFs: `NoSuchFile`). A symlink listed as its
                // own entry points to a file or to a directory that cannot be opened; it is
                // taken as a file.
                EntryKind::File | EntryKind::Symlink => {
                    return Err(Error::new(
                        ErrorKind::NotFound,
                        format!("No such file or directory: {path}"),
                    )
                    .with_detail(format!("{parent} is not a directory")));
                }
            }
        }
        let Some(listed) = entries.into_iter().find(|e| e.name == name) else {
            return Err(Error::new(
                ErrorKind::NotFound,
                format!("No such file or directory: {path}"),
            ));
        };
        let Some(mut entry) = to_entry(&parent, listed) else {
            return Err(Error::new(
                ErrorKind::Io,
                format!("Not a regular file: {path}"),
            ));
        };
        if entry.kind == EntryKind::File && !mlsd {
            // `LIST` dates have minutes at best (only a day for older files).
            let (code, text) = self.reply(format!("MDTM {}", arg(path)?), path).await?;
            if code == 213
                && let Some(t) = parse_timeval(text.trim())
            {
                entry.modified = Some(t);
            }
        }
        Ok(entry)
    }

    /// [`RemoteFs::stat`] for the error paths: a missing path is confirmed with a listing of
    /// the parent, so an unsearchable parent is `Permission` rather than `NotFound`.
    /// Boxed: `stat_with` and `stat_listed` call each other.
    fn stat_exact<'a>(
        &'a self,
        path: &'a RemotePath,
    ) -> futures_util::future::BoxFuture<'a, Result<RemoteEntry>> {
        Box::pin(self.stat_with(path, true))
    }

    /// `stat`. With `confirm_missing`, a path `SIZE` and `STAT` say is missing is looked up in
    /// a listing of its parent too (see the type's docs for the trade-off without it).
    async fn stat_with(&self, path: &RemotePath, confirm_missing: bool) -> Result<RemoteEntry> {
        let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
            return Ok(root_entry());
        };
        let size = format!("SIZE {}", arg(path)?);
        let stat = format!("STAT {path}");
        let mdtm = format!("MDTM {path}");
        let (context, name) = (path.to_string(), name.to_owned());
        let probe = self
            .session
            .with_control(move |c| {
                Box::pin(async move {
                    let (code, text) = session::quote(&mut c.stream, &size, &context).await?;
                    if matches!(code, 500 | 502) {
                        return Ok(Probe::List { sized: false });
                    }
                    if code != 213 && code != 550 {
                        return Ok(Probe::Refused(code, text));
                    }
                    // `SIZE` follows symlinks; `STAT <path>` lists the path itself (ruling R10).
                    let (stat_code, listing) =
                        session::quote(&mut c.stream, &stat, &context).await?;
                    let reply = classify_stat_reply(stat_code, &listing, &name, SystemTime::now());
                    if code == 550 {
                        // Not a regular file (ruling R11): missing, a symlink, or for the
                        // listing to tell.
                        return Ok(match reply {
                            StatReply::Missing => Probe::Missing,
                            StatReply::Entry(e) if e.kind == Some(EntryKind::Symlink) => {
                                Probe::Found(e)
                            }
                            _ => Probe::List { sized: false },
                        });
                    }
                    let StatReply::Entry(mut listed) = reply else {
                        return Ok(Probe::List { sized: true });
                    };
                    match listed.kind {
                        Some(EntryKind::Symlink) => {}
                        Some(EntryKind::File) => {
                            listed.size = parse_size(code, &text, &context)?;
                            let (mdtm_code, stamp) =
                                session::quote(&mut c.stream, &mdtm, &context).await?;
                            if let Some(t) = (mdtm_code == 213)
                                .then(|| parse_timeval(stamp.trim()))
                                .flatten()
                            {
                                listed.modified = Some(t);
                            }
                        }
                        _ => return Ok(Probe::List { sized: true }),
                    }
                    Ok(Probe::Found(listed))
                })
            })
            .await?;
        match probe {
            Probe::Found(listed) => to_entry(&parent, listed).ok_or_else(|| {
                Error::new(ErrorKind::Protocol, format!("Unexpected FTP reply: {path}"))
            }),
            Probe::List { sized } => match self.stat_listed(path).await {
                Err(e) if sized && e.kind == ErrorKind::NotFound => Err(unreadable_listing_error(
                    path,
                    "SIZE found the file, the listing did not",
                )),
                other => other,
            },
            Probe::Missing if confirm_missing => self.stat_listed(path).await,
            Probe::Missing => Err(Error::new(
                ErrorKind::NotFound,
                format!("No such file or directory: {path}"),
            )),
            Probe::Refused(code, text) => Err(map_reply(code, &text, path.as_str())),
        }
    }

    fn timeouts(&self) -> Timeouts {
        Timeouts {
            command: self.session.command_timeout(),
            reply: self.session.reply_timeout(),
            data: self.session.data_timeout(),
        }
    }

    /// Whether `path` is missing (`stat` says `NotFound`). Other `stat` errors are returned.
    async fn missing(&self, path: &RemotePath) -> Result<bool> {
        match self.stat_exact(path).await {
            Ok(_) => Ok(false),
            Err(e) if e.kind == ErrorKind::NotFound => Ok(true),
            Err(e) => Err(e),
        }
    }
}

/// A parsed directory listing.
struct Listing {
    entries: Vec<ListedEntry>,
    /// The listing proves that the path is a directory: `MLSD` succeeded, or `LIST -a` sent a
    /// `.` entry. Without that proof it may be the listing of a file (servers list a file as
    /// its own entry) or of an unreadable directory (vsftpd lists its own entry instead).
    is_dir: bool,
    /// From `MLSD`: modification times are exact.
    mlsd: bool,
}

/// `path` as a command argument. FTP commands are lines: a CR or LF would end the command
/// early (and suppaftp refuses to send it), so such names are refused here.
fn arg(path: &RemotePath) -> Result<&str> {
    let s = path.as_str();
    if s.contains(['\r', '\n']) {
        return Err(Error::new(
            ErrorKind::Io,
            format!(
                "FTP cannot use a name with a line break: {}",
                s.escape_debug()
            ),
        ));
    }
    Ok(s)
}

/// The entry for `/`, a directory that is never asked about.
fn root_entry() -> RemoteEntry {
    RemoteEntry {
        name: "/".to_owned(),
        path: RemotePath::root(),
        kind: EntryKind::Dir,
        size: 0,
        modified: None,
        mode: None,
    }
}

/// What `stat`'s commands on the control connection found out.
enum Probe {
    /// A regular file or a symlink, from `STAT <path>`.
    Found(ListedEntry),
    /// Only a listing of the parent can tell. `sized`: `SIZE` answered `213`, so the path
    /// exists, and a listing that does not show it cannot be read.
    List { sized: bool },
    /// `SIZE` said `550` and `STAT <path>` listed nothing: the path does not exist.
    Missing,
    /// `SIZE` was refused with this reply.
    Refused(u32, String),
}

/// The size in a `213` reply to `SIZE`.
fn parse_size(code: u32, text: &str, path: &str) -> Result<u64> {
    text.trim().parse().map_err(|_| {
        Error::new(ErrorKind::Protocol, format!("Unexpected FTP reply: {path}"))
            .with_detail(format!("{code} {text}"))
    })
}

/// A listing this client cannot read (see [`unreadable_listing`]); the first line is the
/// detail.
fn listing_format_error(dir: &RemotePath, lines: &[String]) -> Error {
    let first = lines.iter().find(|l| !l.trim().is_empty());
    unreadable_listing_error(dir, first.map(String::as_str).unwrap_or(""))
}

fn unreadable_listing_error(path: &RemotePath, detail: &str) -> Error {
    Error::new(
        ErrorKind::Protocol,
        "Cannot read the server's listing format",
    )
    .with_detail(format!("{path}: {}", detail.trim_end()))
}

fn not_found(path: &RemotePath, cause: &Error) -> Error {
    let e = Error::new(
        ErrorKind::NotFound,
        format!("No such file or directory: {path}"),
    );
    match &cause.detail {
        Some(detail) => e.with_detail(detail.clone()),
        None => e,
    }
}

/// A `stat` failure that explains a vague reply better than the reply itself.
fn is_definite(e: &Error) -> bool {
    matches!(e.kind, ErrorKind::Network | ErrorKind::Permission)
}

/// Only regular files are opened: a directory, a symlink (as a listing reports it) or a
/// special file (which `stat` already refuses) is `Io`.
fn regular_file(st: &RemoteEntry, path: &RemotePath) -> Result<()> {
    match st.kind {
        EntryKind::File => Ok(()),
        EntryKind::Dir => Err(Error::new(ErrorKind::Io, format!("Is a directory: {path}"))),
        EntryKind::Symlink => Err(Error::new(
            ErrorKind::Io,
            format!("Not a regular file: {path}"),
        )),
    }
}

/// The transfer command's outcome on `conn` (bounded by `command_timeout`), errors mapped.
/// A refusal read as a clean reply (`550` on `RETR`, ...) leaves `conn` in sync, so with
/// `reusable` it is marked clean and goes back to the pool when the caller drops it. A read
/// that sent `REST` passes `reusable = false`: after a refused `RETR` the server may keep the
/// restart offset for the next transfer. After a socket error or a timeout `conn` is closed,
/// and a lost connection is reported to the session.
fn transfer_opened(
    conn: &mut PooledConn,
    opened: std::result::Result<suppaftp::FtpResult<Transfer>, tokio::time::error::Elapsed>,
    path: &RemotePath,
    reusable: bool,
) -> Result<Transfer> {
    let e = match opened {
        Ok(Ok(transfer)) => return Ok(transfer),
        Ok(Err(e @ suppaftp::FtpError::UnexpectedResponse(_))) => {
            let e = map_ftp_error(e, path.as_str());
            if reusable && !is_control_loss(&e) {
                conn.mark_clean();
            }
            e
        }
        Ok(Err(e)) => map_ftp_error(e, path.as_str()),
        Err(_) => Error::new(ErrorKind::Network, format!("Connection lost: {path}"))
            .with_detail("no reply to the transfer command"),
    };
    conn.report(&e);
    Err(e)
}

fn not_a_dir(path: &RemotePath) -> Error {
    Error::new(ErrorKind::Io, format!("Not a directory: {path}"))
}

/// The entry for `listed` in `dir`; `None` for special files and names that cannot be a
/// single path component.
fn to_entry(dir: &RemotePath, listed: ListedEntry) -> Option<RemoteEntry> {
    let kind = listed.kind?;
    let name = listed.name;
    if name.is_empty() || name == "." || name == ".." || name.contains('/') {
        return None;
    }
    Some(RemoteEntry {
        path: dir.join(&name),
        name,
        kind,
        size: if kind == EntryKind::File {
            listed.size
        } else {
            0
        },
        modified: listed.modified,
        mode: listed.mode,
    })
}

impl std::fmt::Debug for FtpFs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FtpFs")
            .field("session", &self.session)
            .finish()
    }
}

impl Connection for FtpFs {
    fn state(&self) -> watch::Receiver<ConnectionState> {
        self.session.state()
    }

    fn generation(&self) -> u64 {
        self.session.generation()
    }
}

#[async_trait]
impl RemoteFs for FtpFs {
    async fn list(&self, path: &RemotePath) -> Result<Vec<RemoteEntry>> {
        let Listing {
            entries, is_dir, ..
        } = match self.entries(path).await {
            Ok(listing) => listing,
            Err(e) if matches!(e.kind, ErrorKind::Io | ErrorKind::NotFound) => {
                // `MLSD` on a file or a missing path, or a server that refuses `LIST` there.
                return Err(match self.stat_exact(path).await {
                    Err(st) if st.kind == ErrorKind::NotFound => not_found(path, &e),
                    Err(st) if is_definite(&st) => st,
                    Ok(st) if !matches!(st.kind, EntryKind::Dir | EntryKind::Symlink) => {
                        not_a_dir(path)
                    }
                    _ => e,
                });
            }
            Err(e) => return Err(e),
        };
        if !is_dir {
            // Empty, a file's own entry, or an unreadable directory's own entry.
            let st = self.stat_exact(path).await?;
            match st.kind {
                EntryKind::Dir => {
                    if let [only] = entries.as_slice()
                        && Some(only.name.as_str()) == path.file_name()
                        && only.kind == Some(EntryKind::Dir)
                        && only.mode == st.mode
                        && only.modified == st.modified
                    {
                        return Err(Error::new(
                            ErrorKind::Permission,
                            format!("Permission denied: {path}"),
                        ));
                    }
                }
                // A symlink to a file is listed as its own entry, like a file.
                EntryKind::Symlink => {
                    if let [only] = entries.as_slice()
                        && Some(only.name.as_str()) == path.file_name()
                        && only.kind == Some(EntryKind::Symlink)
                    {
                        return Err(not_a_dir(path));
                    }
                }
                EntryKind::File => return Err(not_a_dir(path)),
            }
        }
        Ok(entries
            .into_iter()
            .filter_map(|listed| to_entry(path, listed))
            .collect())
    }

    async fn stat(&self, path: &RemotePath) -> Result<RemoteEntry> {
        self.stat_with(path, false).await
    }

    async fn open_read(&self, path: &RemotePath, offset: u64) -> Result<BoxAsyncRead> {
        let name = arg(path)?.to_owned();
        if offset >= self.file_size(path).await? {
            return Ok(Box::new(tokio::io::empty()));
        }
        let rest = usize::try_from(offset).map_err(|_| {
            Error::new(
                ErrorKind::Io,
                format!("Offset {offset} out of range: {path}"),
            )
        })?;
        let mut conn = self.session.checkout().await?;
        let opened = tokio::time::timeout(self.session.command_timeout(), async {
            let ftp = conn.stream();
            ftp.resume_transfer(rest).await?;
            ftp.retr_as_stream(&name).await
        })
        .await;
        let transfer = transfer_opened(&mut conn, opened, path, rest == 0)?;
        Ok(Box::new(FtpReader::new(
            conn,
            transfer,
            path.to_string(),
            self.timeouts(),
        )))
    }

    async fn open_write(&self, path: &RemotePath, offset: u64) -> Result<BoxAsyncWrite> {
        let name = arg(path)?.to_owned();
        // Offset 0 sends `STOR` straight away (ruling R8); a refusal is explained afterwards.
        // `APPE` only continues a file whose size is exactly `offset`.
        if offset > 0 {
            match self.file_size(path).await {
                Ok(size) if size == offset => {}
                Ok(size) => {
                    return Err(Error::new(
                        ErrorKind::Io,
                        format!("Cannot continue {path} at byte {offset}"),
                    )
                    .with_detail(format!("the file has {size} bytes")));
                }
                Err(e) if e.kind == ErrorKind::NotFound => {
                    return Err(Error::new(
                        ErrorKind::Io,
                        format!("Cannot continue a missing file: {path}"),
                    ));
                }
                Err(e) => return Err(e),
            }
        }
        let mut conn = self.session.checkout().await?;
        // The server creates or truncates the file before its `150`: done once this returns.
        let opened = tokio::time::timeout(self.session.command_timeout(), async {
            let ftp = conn.stream();
            if offset > 0 {
                ftp.append_with_stream(&name).await
            } else {
                ftp.put_with_stream(&name).await
            }
        })
        .await;
        let transfer = match transfer_opened(&mut conn, opened, path, true) {
            Ok(transfer) => transfer,
            Err(e) => {
                drop(conn);
                return Err(self.explain_write(path, e).await);
            }
        };
        Ok(Box::new(FtpWriter::new(
            conn,
            transfer,
            path.to_string(),
            self.timeouts(),
        )))
    }

    async fn mkdir(&self, path: &RemotePath) -> Result<()> {
        let (code, text) = self.reply(format!("MKD {}", arg(path)?), path).await?;
        if code < 400 {
            return Ok(());
        }
        let err = map_reply(code, &text, path.as_str());
        match self.stat_exact(path).await {
            Ok(st) if st.kind == EntryKind::Dir => return Ok(()),
            Ok(_) => return Err(err),
            Err(e) if e.kind != ErrorKind::NotFound => return Err(e),
            Err(_) => {}
        }
        match path.parent() {
            Some(parent) if self.missing(&parent).await? => Err(not_found(&parent, &err)),
            _ => Err(err),
        }
    }

    async fn rename(&self, from: &RemotePath, to: &RemotePath) -> Result<()> {
        let (rnfr, rnto) = (format!("RNFR {}", arg(from)?), format!("RNTO {}", arg(to)?));
        let context = from.to_string();
        let (first, second) = self
            .session
            .with_control(move |c| {
                Box::pin(async move {
                    let first = session::quote(&mut c.stream, &rnfr, &context).await?;
                    if first.0 >= 400 {
                        return Ok((first, None));
                    }
                    let second = session::quote(&mut c.stream, &rnto, &context).await?;
                    Ok((first, Some(second)))
                })
            })
            .await?;
        let (code, text, target) = match second {
            None => (first.0, first.1, from),
            Some((code, _)) if code < 400 => return Ok(()),
            Some((code, text)) => (code, text, to),
        };
        let err = map_reply(code, &text, target.as_str());
        if !matches!(err.kind, ErrorKind::Io | ErrorKind::NotFound) {
            return Err(err);
        }
        if self.missing(from).await? {
            return Err(not_found(from, &err));
        }
        match to.parent() {
            Some(parent) if self.missing(&parent).await? => Err(not_found(&parent, &err)),
            _ => Err(err),
        }
    }

    async fn remove_file(&self, path: &RemotePath) -> Result<()> {
        let (code, text) = self.reply(format!("DELE {}", arg(path)?), path).await?;
        if code < 400 {
            return Ok(());
        }
        let err = map_reply(code, &text, path.as_str());
        if !matches!(err.kind, ErrorKind::Io | ErrorKind::NotFound) {
            return Err(err);
        }
        match self.stat_exact(path).await {
            Err(e) if e.kind == ErrorKind::NotFound => Err(not_found(path, &err)),
            Err(e) if is_definite(&e) => Err(e),
            Ok(st) if st.kind == EntryKind::Dir => {
                Err(Error::new(ErrorKind::Io, format!("Is a directory: {path}"))
                    .with_detail(err.detail.unwrap_or_default()))
            }
            _ => Err(err),
        }
    }

    async fn remove_dir(&self, path: &RemotePath) -> Result<()> {
        let (code, text) = self.reply(format!("RMD {}", arg(path)?), path).await?;
        if code < 400 {
            return Ok(());
        }
        let err = map_reply(code, &text, path.as_str());
        if !matches!(err.kind, ErrorKind::Io | ErrorKind::NotFound) {
            return Err(err);
        }
        match self.stat_exact(path).await {
            Err(e) if e.kind == ErrorKind::NotFound => Err(not_found(path, &err)),
            Err(e) if is_definite(&e) => Err(e),
            Ok(st) if st.kind != EntryKind::Dir => Err(not_a_dir(path)),
            // Most likely not empty.
            Ok(_) => {
                Err(Error::new(ErrorKind::Io, err.message)
                    .with_detail(err.detail.unwrap_or_default()))
            }
            Err(_) => Err(err),
        }
    }

    async fn chmod(&self, path: &RemotePath, mode: u32) -> Result<()> {
        self.simple(&format!("SITE CHMOD {:o}", mode & 0o7777), path)
            .await
    }

    async fn set_modified(&self, path: &RemotePath, mtime: SystemTime) -> Result<()> {
        let command = format_timeval(mtime).map(|stamp| format!("MFMT {stamp} {path}"));
        arg(path)?;
        let context = path.to_string();
        let reply = self
            .session
            .with_control(move |c| {
                Box::pin(async move {
                    if !c.has_mfmt {
                        return Ok(None);
                    }
                    let Some(command) = command else {
                        return Err(Error::new(
                            ErrorKind::Io,
                            format!("Modification time out of range for FTP: {context}"),
                        ));
                    };
                    session::quote(&mut c.stream, &command, &context)
                        .await
                        .map(Some)
                })
            })
            .await?;
        match reply {
            Some((code, text)) if code >= 400 => Err(self
                .explain(path, map_reply(code, &text, path.as_str()))
                .await),
            _ => Ok(()),
        }
    }
}
