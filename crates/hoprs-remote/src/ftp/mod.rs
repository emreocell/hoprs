//! FTP and FTPS backend: [`FtpFs`].

mod config;
pub(crate) mod errors;
mod list;
mod prompt;
pub(crate) mod session;
mod tls;

use std::sync::Arc;
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
use errors::{map_ftp_error, map_reply};
use list::{
    ListedEntry, format_timeval, is_dot_line, parse_list_line, parse_mlsd_line, parse_timeval,
};
use session::FtpSession;

/// `RemoteFs` over FTP or FTPS. Cheap to clone; clones share one [`FtpSession`].
///
/// Listings and metadata commands run on the session's control connection, one at a time.
/// Paths are always sent absolute; the working directory is never changed.
///
/// - `list` uses `MLSD` when `FEAT` announces it, `LIST -a` otherwise (unix `ls -l` format).
///   `.`, `..`, FIFOs, sockets and devices are left out; symlinks are reported, not followed.
/// - `stat` lists the parent directory and looks the name up (`/` is a directory without
///   asking). A file's exact modification time comes from `MDTM` when the listing has none.
/// - Servers answer most failures with a bare `550` (vsftpd: "Delete operation failed."), so
///   an ambiguous failure is explained with a `stat`: a missing path is `NotFound`, the wrong
///   kind of entry `Io`.
/// - `set_modified` uses `MFMT` when `FEAT` announces it, and is a no-op otherwise.
///
/// **Do not cancel these futures midway.** A command dropped before its reply leaves the
/// control connection in an unknown state, so the session discards it and reconnects (see
/// `FtpSession::with_control`): the user would see a reconnection.
#[derive(Clone)]
pub struct FtpFs {
    session: FtpSession,
}

impl FtpFs {
    /// Connects and logs in (see [`FtpSession::connect`]).
    pub async fn connect(cfg: FtpConfig, prompter: Arc<dyn FtpPrompter>) -> Result<FtpFs> {
        let session = FtpSession::connect(cfg, prompter).await?;
        Ok(FtpFs { session })
    }

    /// Closes every connection and stops reconnecting.
    pub async fn disconnect(&self) {
        self.session.disconnect().await;
    }

    /// The raw lines of `dir`'s listing, and whether they came from `MLSD`.
    async fn listing(&self, dir: &RemotePath) -> Result<(Vec<String>, bool)> {
        let arg = arg(dir)?.to_owned();
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

    /// The parsed entries of `dir`, special files included.
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
        match self.stat(path).await {
            Err(e) if e.kind == ErrorKind::NotFound => not_found(path, &err),
            Err(e) if is_definite(&e) => e,
            _ => err,
        }
    }

    /// Whether `path` is missing (`stat` says `NotFound`). Other `stat` errors are returned.
    async fn missing(&self, path: &RemotePath) -> Result<bool> {
        match self.stat(path).await {
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
                return Err(match self.stat(path).await {
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
            let st = self.stat(path).await?;
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
        let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
            return Ok(RemoteEntry {
                name: "/".to_owned(),
                path: RemotePath::root(),
                kind: EntryKind::Dir,
                size: 0,
                modified: None,
                mode: None,
            });
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
            let up = self.stat(&parent).await?;
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

    async fn open_read(&self, path: &RemotePath, _offset: u64) -> Result<BoxAsyncRead> {
        Err(Error::new(
            ErrorKind::Protocol,
            format!("FTP downloads are not implemented yet: {path}"),
        ))
    }

    async fn open_write(&self, path: &RemotePath, _offset: u64) -> Result<BoxAsyncWrite> {
        Err(Error::new(
            ErrorKind::Protocol,
            format!("FTP uploads are not implemented yet: {path}"),
        ))
    }

    async fn mkdir(&self, path: &RemotePath) -> Result<()> {
        let (code, text) = self.reply(format!("MKD {}", arg(path)?), path).await?;
        if code < 400 {
            return Ok(());
        }
        let err = map_reply(code, &text, path.as_str());
        match self.stat(path).await {
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
        match self.stat(path).await {
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
        match self.stat(path).await {
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
