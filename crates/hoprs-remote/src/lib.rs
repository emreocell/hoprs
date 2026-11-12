//! Protocol-agnostic remote filesystem abstraction.

use std::time::SystemTime;

use async_trait::async_trait;
use hoprs_core::{RemoteEntry, RemotePath, Result};

mod connection;
#[cfg(feature = "testing")]
mod fault;
mod ftp;
mod io_err;
mod local;
#[cfg(feature = "testing")]
mod mem;
#[cfg(feature = "testing")]
pub use fault::{FaultFs, Op};
#[cfg(feature = "testing")]
pub use mem::MemFs;
mod sftp;
#[cfg(feature = "testing")]
pub mod testing;

pub use connection::Connection;
pub use ftp::{CertProblem, FtpConfig, FtpPrompter, FtpSecurity, TransferMode};
#[cfg(feature = "testing")]
pub use ftp::{FtpPromptCalls, ScriptedFtpPrompter};
pub use sftp::{SftpConfig, SftpFs};

pub use local::LocalFs;

pub type BoxAsyncRead = Box<dyn tokio::io::AsyncRead + Send + Unpin>;
pub type BoxAsyncWrite = Box<dyn tokio::io::AsyncWrite + Send + Unpin>;

/// A remote filesystem (SFTP, FTP, in-memory, ...).
///
/// Contract (every backend, and every test double, must follow it; the
/// transfer engine relies on each point):
///
/// - **Symlinks are never followed** by `stat` or `list`: a symlink is
///   reported as `EntryKind::Symlink` (SFTP must use `lstat`, not `stat`).
///   `list` never includes `.` or `..`.
/// - **Errors:** a missing path fails with `ErrorKind::NotFound`; an
///   existing path of the wrong kind (`list` on a file, `open_*` on a
///   directory, ...) fails with `ErrorKind::Io`. A lost connection is
///   `ErrorKind::Network`.
/// - **`open_read(path, offset)`** streams the file from `offset`. At or
///   past the end of the file the stream is empty (reads return 0 bytes);
///   that is not an error.
/// - **`open_write(path, 0)`** creates or truncates the file *before it
///   returns*: once it returns `Ok`, the old contents are gone even if
///   nothing is ever written. (FTP must issue `STOR` inside `open_write`,
///   not lazily on the first write.)
/// - **`open_write(path, offset > 0)`** continues the file from `offset`.
///   The engine only ever passes an `offset` equal to the current size of
///   the destination; a backend that cannot truncate (e.g. FTP `APPE`) is
///   correct in that case. A larger `offset` (or a missing file) fails with
///   `ErrorKind::Io`. Any drift is caught by the engine's final size check.
/// - **The writer's `shutdown()`** does not return `Ok` until all written
///   data has been acknowledged by the destination (for SFTP, every
///   pipelined write request; for FTP, the transfer-complete reply). The
///   engine treats a successful `shutdown()` as "the bytes are there".
/// - `mkdir` on an existing directory succeeds; it creates only the leaf
///   (a missing parent fails with `ErrorKind::NotFound`).
#[async_trait]
pub trait RemoteFs: Send + Sync {
    async fn list(&self, path: &RemotePath) -> Result<Vec<RemoteEntry>>;
    async fn stat(&self, path: &RemotePath) -> Result<RemoteEntry>;
    async fn open_read(&self, path: &RemotePath, offset: u64) -> Result<BoxAsyncRead>;
    async fn open_write(&self, path: &RemotePath, offset: u64) -> Result<BoxAsyncWrite>;
    async fn mkdir(&self, path: &RemotePath) -> Result<()>;
    async fn rename(&self, from: &RemotePath, to: &RemotePath) -> Result<()>;
    async fn remove_file(&self, path: &RemotePath) -> Result<()>;
    async fn remove_dir(&self, path: &RemotePath) -> Result<()>;
    async fn chmod(&self, path: &RemotePath, mode: u32) -> Result<()>;

    /// Best-effort modification time update; backends that cannot do it keep
    /// the default no-op.
    async fn set_modified(&self, _path: &RemotePath, _mtime: SystemTime) -> Result<()> {
        Ok(())
    }
}
