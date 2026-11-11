//! Protocol-agnostic remote filesystem abstraction.

use std::time::SystemTime;

use async_trait::async_trait;
use hoprs_core::{RemoteEntry, RemotePath, Result};

#[cfg(feature = "testing")]
mod mem;
#[cfg(feature = "testing")]
pub use mem::MemFs;

pub type BoxAsyncRead = Box<dyn tokio::io::AsyncRead + Send + Unpin>;
pub type BoxAsyncWrite = Box<dyn tokio::io::AsyncWrite + Send + Unpin>;

/// A remote filesystem (SFTP, FTP, in-memory, ...).
///
/// Contract:
/// - `list` never includes `.`/`..` and reports symlinks as
///   `EntryKind::Symlink` without following them.
/// - `stat` on a missing path fails with `ErrorKind::NotFound`.
/// - `open_write` with `offset == 0` creates or truncates the file. With
///   `offset > 0` the file must exist with size >= `offset`; it is truncated
///   to `offset` and writing continues from there.
/// - `mkdir` on an existing directory succeeds.
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
