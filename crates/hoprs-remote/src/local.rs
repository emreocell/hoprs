//! Local-disk `RemoteFs`.
//!
//! Error mapping follows the `MemFs` contract: missing path -> `NotFound`;
//! wrong node kind, non-empty `remove_dir`, invalid `open_write` offset ->
//! `Io`; OS permission errors -> `Permission`.

use std::fs::Metadata;
use std::io::{self, SeekFrom};
use std::path::PathBuf;
use std::time::SystemTime;

use async_trait::async_trait;
use hoprs_core::{EntryKind, Error, ErrorKind, RemoteEntry, RemotePath, Result};
use tokio::io::AsyncSeekExt;

use crate::{BoxAsyncRead, BoxAsyncWrite, RemoteFs};

/// `RemoteFs` over the local disk. A `RemotePath` `/a/b` maps to
/// `root/a/b`; production uses `/` as the root.
#[derive(Debug, Clone)]
pub struct LocalFs {
    root: PathBuf,
}

fn io_err(msg: String) -> Error {
    Error::new(ErrorKind::Io, msg)
}

fn entry_kind(meta: &Metadata) -> EntryKind {
    let ft = meta.file_type();
    if ft.is_symlink() {
        EntryKind::Symlink
    } else if ft.is_dir() {
        EntryKind::Dir
    } else {
        EntryKind::File
    }
}

#[cfg(unix)]
fn mode_of(meta: &Metadata) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    Some(meta.permissions().mode() & 0o7777)
}

#[cfg(not(unix))]
fn mode_of(_meta: &Metadata) -> Option<u32> {
    None
}

fn make_entry(name: String, path: RemotePath, meta: &Metadata) -> RemoteEntry {
    let kind = entry_kind(meta);
    RemoteEntry {
        name,
        path,
        kind,
        size: if kind == EntryKind::File {
            meta.len()
        } else {
            0
        },
        modified: meta.modified().ok(),
        mode: mode_of(meta),
    }
}

impl LocalFs {
    pub fn new(root: impl Into<PathBuf>) -> LocalFs {
        LocalFs { root: root.into() }
    }

    fn local(&self, path: &RemotePath) -> PathBuf {
        let mut out = self.root.clone();
        out.extend(path.as_str().split('/').filter(|s| !s.is_empty()));
        out
    }

    async fn meta(&self, path: &RemotePath) -> Result<Metadata> {
        Ok(tokio::fs::symlink_metadata(self.local(path)).await?)
    }
}

#[async_trait]
impl RemoteFs for LocalFs {
    async fn list(&self, path: &RemotePath) -> Result<Vec<RemoteEntry>> {
        let mut rd = tokio::fs::read_dir(self.local(path)).await?;
        let mut out = Vec::new();
        while let Some(de) = rd.next_entry().await? {
            let name = de.file_name().to_string_lossy().into_owned();
            let meta = match tokio::fs::symlink_metadata(de.path()).await {
                Ok(m) => m,
                // Removed between readdir and stat.
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e.into()),
            };
            out.push(make_entry(name.clone(), path.join(&name), &meta));
        }
        Ok(out)
    }

    async fn stat(&self, path: &RemotePath) -> Result<RemoteEntry> {
        let meta = self.meta(path).await?;
        let name = path.file_name().unwrap_or("/").to_owned();
        Ok(make_entry(name, path.clone(), &meta))
    }

    async fn open_read(&self, path: &RemotePath, offset: u64) -> Result<BoxAsyncRead> {
        let local = self.local(path);
        if tokio::fs::metadata(&local).await?.is_dir() {
            return Err(io_err(format!("not a file: {path}")));
        }
        let mut file = tokio::fs::File::open(&local).await?;
        if offset > 0 {
            file.seek(SeekFrom::Start(offset)).await?;
        }
        Ok(Box::new(file))
    }

    async fn open_write(&self, path: &RemotePath, offset: u64) -> Result<BoxAsyncWrite> {
        let local = self.local(path);
        match tokio::fs::metadata(&local).await {
            Ok(m) if m.is_dir() => return Err(io_err(format!("not a file: {path}"))),
            Ok(m) if offset > m.len() => {
                return Err(io_err(format!("offset {offset} beyond end of {path}")));
            }
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound && offset > 0 => {
                return Err(io_err(format!("cannot resume missing file: {path}")));
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        let mut opts = tokio::fs::OpenOptions::new();
        opts.write(true);
        if offset == 0 {
            opts.create(true).truncate(true);
        }
        let mut file = opts.open(&local).await?;
        if offset > 0 {
            file.set_len(offset).await?;
            file.seek(SeekFrom::Start(offset)).await?;
        }
        Ok(Box::new(file))
    }

    async fn mkdir(&self, path: &RemotePath) -> Result<()> {
        let local = self.local(path);
        match tokio::fs::create_dir(&local).await {
            Ok(()) => Ok(()),
            Err(e) => match tokio::fs::metadata(&local).await {
                Ok(m) if m.is_dir() => Ok(()),
                _ => Err(e.into()),
            },
        }
    }

    async fn rename(&self, from: &RemotePath, to: &RemotePath) -> Result<()> {
        Ok(tokio::fs::rename(self.local(from), self.local(to)).await?)
    }

    async fn remove_file(&self, path: &RemotePath) -> Result<()> {
        if self.meta(path).await?.is_dir() {
            return Err(io_err(format!("is a directory: {path}")));
        }
        Ok(tokio::fs::remove_file(self.local(path)).await?)
    }

    async fn remove_dir(&self, path: &RemotePath) -> Result<()> {
        if !self.meta(path).await?.is_dir() {
            return Err(io_err(format!("not a directory: {path}")));
        }
        Ok(tokio::fs::remove_dir(self.local(path)).await?)
    }

    #[cfg(unix)]
    async fn chmod(&self, path: &RemotePath, mode: u32) -> Result<()> {
        use std::os::unix::fs::PermissionsExt;
        Ok(
            tokio::fs::set_permissions(self.local(path), std::fs::Permissions::from_mode(mode))
                .await?,
        )
    }

    #[cfg(not(unix))]
    async fn chmod(&self, _path: &RemotePath, _mode: u32) -> Result<()> {
        Ok(())
    }

    async fn set_modified(&self, path: &RemotePath, mtime: SystemTime) -> Result<()> {
        let local = self.local(path);
        tokio::task::spawn_blocking(move || -> io::Result<()> {
            let file = std::fs::File::options().write(true).open(&local);
            // Directories cannot be opened for writing on every platform.
            let file = match file {
                Ok(f) => f,
                Err(_) if local.is_dir() => std::fs::File::open(&local)?,
                Err(e) => return Err(e),
            };
            file.set_modified(mtime)
        })
        .await
        .map_err(|e| io_err(format!("set_modified task failed: {e}")))?
        .map_err(Error::from)
    }
}
