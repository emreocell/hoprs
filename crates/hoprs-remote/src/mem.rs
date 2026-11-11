//! In-memory `RemoteFs` for tests.
//!
//! Error mapping: missing path -> `NotFound`; wrong node kind, non-empty
//! `remove_dir`, `open_write` with an invalid offset (including a missing
//! file) -> `Io`; writes under a denied prefix -> `Permission`.

use std::collections::BTreeMap;
use std::io;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Context, Poll};
use std::time::SystemTime;

use async_trait::async_trait;
use hoprs_core::{EntryKind, Error, ErrorKind, RemoteEntry, RemotePath, Result};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use crate::{BoxAsyncRead, BoxAsyncWrite, RemoteFs};

enum Node {
    File {
        data: Vec<u8>,
        mtime: SystemTime,
        mode: u32,
    },
    Dir {
        mtime: SystemTime,
    },
    Symlink {
        #[allow(dead_code)]
        target: String,
    },
}

struct State {
    nodes: BTreeMap<RemotePath, Node>,
    denied: Vec<RemotePath>,
}

impl State {
    fn is_denied(&self, path: &RemotePath) -> bool {
        let mut cur = Some(path.clone());
        while let Some(c) = cur {
            if self.denied.contains(&c) {
                return true;
            }
            cur = c.parent();
        }
        false
    }

    fn check_write(&self, path: &RemotePath) -> Result<()> {
        if self.is_denied(path) {
            return Err(Error::new(
                ErrorKind::Permission,
                format!("permission denied: {path}"),
            ));
        }
        Ok(())
    }

    fn ensure_dirs(&mut self, path: &RemotePath, mtime: SystemTime) {
        let mut cur = Some(path.clone());
        while let Some(c) = cur {
            self.nodes.entry(c.clone()).or_insert(Node::Dir { mtime });
            cur = c.parent();
        }
    }

    fn entry(&self, path: &RemotePath) -> Option<RemoteEntry> {
        let node = self.nodes.get(path)?;
        let name = path.file_name().unwrap_or("/").to_owned();
        let (kind, size, modified, mode) = match node {
            Node::File { data, mtime, mode } => (
                EntryKind::File,
                data.len() as u64,
                Some(*mtime),
                Some(*mode),
            ),
            Node::Dir { mtime } => (EntryKind::Dir, 0, Some(*mtime), None),
            Node::Symlink { .. } => (EntryKind::Symlink, 0, None, None),
        };
        Some(RemoteEntry {
            name,
            path: path.clone(),
            kind,
            size,
            modified,
            mode,
        })
    }

    fn require(&self, path: &RemotePath) -> Result<&Node> {
        self.nodes.get(path).ok_or_else(|| not_found(path))
    }
}

fn not_found(path: &RemotePath) -> Error {
    Error::new(ErrorKind::NotFound, format!("not found: {path}"))
}

fn io_err(msg: String) -> Error {
    Error::new(ErrorKind::Io, msg)
}

/// In-memory filesystem; cheap to clone, clones share state.
#[derive(Clone)]
pub struct MemFs {
    state: Arc<Mutex<State>>,
}

impl Default for MemFs {
    fn default() -> Self {
        Self::new()
    }
}

fn rp(s: &str) -> RemotePath {
    RemotePath::new(s).expect("valid test path")
}

impl MemFs {
    /// Creates a filesystem containing only the root directory.
    pub fn new() -> MemFs {
        let mut nodes = BTreeMap::new();
        nodes.insert(
            RemotePath::root(),
            Node::Dir {
                mtime: SystemTime::UNIX_EPOCH,
            },
        );
        MemFs {
            state: Arc::new(Mutex::new(State {
                nodes,
                denied: Vec::new(),
            })),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap()
    }

    /// Adds a file, creating parent directories.
    pub fn add_file(&self, path: &str, data: &[u8], mtime: SystemTime) {
        let path = rp(path);
        let mut st = self.lock();
        if let Some(parent) = path.parent() {
            st.ensure_dirs(&parent, mtime);
        }
        st.nodes.insert(
            path,
            Node::File {
                data: data.to_vec(),
                mtime,
                mode: 0o644,
            },
        );
    }

    /// Adds a directory, creating parent directories.
    pub fn add_dir(&self, path: &str) {
        self.lock().ensure_dirs(&rp(path), SystemTime::UNIX_EPOCH);
    }

    /// Adds a symlink, creating parent directories.
    pub fn add_symlink(&self, path: &str, target: &str) {
        let path = rp(path);
        let mut st = self.lock();
        if let Some(parent) = path.parent() {
            st.ensure_dirs(&parent, SystemTime::UNIX_EPOCH);
        }
        st.nodes.insert(
            path,
            Node::Symlink {
                target: target.to_owned(),
            },
        );
    }

    /// Returns the file contents, or `None` if the path is not a file.
    pub fn read_file(&self, path: &str) -> Option<Vec<u8>> {
        match self.lock().nodes.get(&rp(path)) {
            Some(Node::File { data, .. }) => Some(data.clone()),
            _ => None,
        }
    }

    /// Makes writes, mkdir, rename and remove under `path` fail with `Permission`.
    pub fn deny_writes_under(&self, path: &str) {
        self.lock().denied.push(rp(path));
    }
}

struct MemReader {
    data: Vec<u8>,
    pos: usize,
}

impl AsyncRead for MemReader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let n = buf.remaining().min(self.data.len() - self.pos);
        let start = self.pos;
        buf.put_slice(&self.data[start..start + n]);
        self.pos += n;
        Poll::Ready(Ok(()))
    }
}

struct MemWriter {
    state: Arc<Mutex<State>>,
    path: RemotePath,
}

impl AsyncWrite for MemWriter {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let mut st = self.state.lock().unwrap();
        match st.nodes.get_mut(&self.path) {
            Some(Node::File { data, .. }) => {
                data.extend_from_slice(buf);
                Poll::Ready(Ok(buf.len()))
            }
            _ => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::NotFound,
                "file removed while open",
            ))),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

#[async_trait]
impl RemoteFs for MemFs {
    async fn list(&self, path: &RemotePath) -> Result<Vec<RemoteEntry>> {
        let st = self.lock();
        match st.require(path)? {
            Node::Dir { .. } => {}
            _ => return Err(io_err(format!("not a directory: {path}"))),
        }
        Ok(st
            .nodes
            .keys()
            .filter(|k| k.parent().as_ref() == Some(path))
            .filter_map(|k| st.entry(k))
            .collect())
    }

    async fn stat(&self, path: &RemotePath) -> Result<RemoteEntry> {
        self.lock().entry(path).ok_or_else(|| not_found(path))
    }

    async fn open_read(&self, path: &RemotePath, offset: u64) -> Result<BoxAsyncRead> {
        let st = self.lock();
        match st.require(path)? {
            Node::File { data, .. } => {
                let pos = usize::try_from(offset)
                    .unwrap_or(usize::MAX)
                    .min(data.len());
                Ok(Box::new(MemReader {
                    data: data.clone(),
                    pos,
                }))
            }
            _ => Err(io_err(format!("not a file: {path}"))),
        }
    }

    async fn open_write(&self, path: &RemotePath, offset: u64) -> Result<BoxAsyncWrite> {
        let mut st = self.lock();
        st.check_write(path)?;
        match st.nodes.get_mut(path) {
            Some(Node::File { data, .. }) => {
                if offset > data.len() as u64 {
                    return Err(io_err(format!("offset {offset} beyond end of {path}")));
                }
                data.truncate(offset as usize);
            }
            Some(_) => return Err(io_err(format!("not a file: {path}"))),
            None => {
                if offset > 0 {
                    return Err(io_err(format!("cannot resume missing file: {path}")));
                }
                let parent = path
                    .parent()
                    .ok_or_else(|| io_err("root is a directory".into()))?;
                match st.nodes.get(&parent) {
                    Some(Node::Dir { .. }) => {}
                    Some(_) => return Err(io_err(format!("not a directory: {parent}"))),
                    None => return Err(not_found(&parent)),
                }
                st.nodes.insert(
                    path.clone(),
                    Node::File {
                        data: Vec::new(),
                        mtime: SystemTime::now(),
                        mode: 0o644,
                    },
                );
            }
        }
        Ok(Box::new(MemWriter {
            state: Arc::clone(&self.state),
            path: path.clone(),
        }))
    }

    async fn mkdir(&self, path: &RemotePath) -> Result<()> {
        let mut st = self.lock();
        st.check_write(path)?;
        match st.nodes.get(path) {
            Some(Node::Dir { .. }) => return Ok(()),
            Some(_) => return Err(io_err(format!("already exists: {path}"))),
            None => {}
        }
        let parent = path.parent().ok_or_else(|| io_err("root exists".into()))?;
        match st.nodes.get(&parent) {
            Some(Node::Dir { .. }) => {}
            Some(_) => return Err(io_err(format!("not a directory: {parent}"))),
            None => return Err(not_found(&parent)),
        }
        st.nodes.insert(
            path.clone(),
            Node::Dir {
                mtime: SystemTime::now(),
            },
        );
        Ok(())
    }

    async fn rename(&self, from: &RemotePath, to: &RemotePath) -> Result<()> {
        let mut st = self.lock();
        st.check_write(from)?;
        st.check_write(to)?;
        let from_is_dir = matches!(st.require(from)?, Node::Dir { .. });
        if from == &RemotePath::root() {
            return Err(io_err("cannot rename root".into()));
        }
        if from == to {
            return Ok(());
        }
        let parent = to.parent().ok_or_else(|| io_err("target is root".into()))?;
        match st.nodes.get(&parent) {
            Some(Node::Dir { .. }) => {}
            Some(_) => return Err(io_err(format!("not a directory: {parent}"))),
            None => return Err(not_found(&parent)),
        }
        let mut anc = to.parent();
        while let Some(a) = anc {
            if &a == from {
                return Err(io_err(format!("cannot move {from} into itself")));
            }
            anc = a.parent();
        }
        if let Some(existing) = st.nodes.get(to)
            && (from_is_dir || matches!(existing, Node::Dir { .. }))
        {
            return Err(io_err(format!("target exists: {to}")));
        }
        let moved: Vec<RemotePath> = st
            .nodes
            .keys()
            .filter(|k| *k == from || is_under(k, from))
            .cloned()
            .collect();
        for old in moved {
            let node = st.nodes.remove(&old).expect("key just listed");
            let suffix = &old.as_str()[from.as_str().len()..];
            let new = if suffix.is_empty() {
                to.clone()
            } else {
                rp(&format!("{}{suffix}", to.as_str()))
            };
            st.nodes.insert(new, node);
        }
        Ok(())
    }

    async fn remove_file(&self, path: &RemotePath) -> Result<()> {
        let mut st = self.lock();
        st.check_write(path)?;
        match st.require(path)? {
            Node::Dir { .. } => Err(io_err(format!("is a directory: {path}"))),
            _ => {
                st.nodes.remove(path);
                Ok(())
            }
        }
    }

    async fn remove_dir(&self, path: &RemotePath) -> Result<()> {
        let mut st = self.lock();
        st.check_write(path)?;
        if !matches!(st.require(path)?, Node::Dir { .. }) {
            return Err(io_err(format!("not a directory: {path}")));
        }
        if path == &RemotePath::root() {
            return Err(io_err("cannot remove root".into()));
        }
        if st.nodes.keys().any(|k| k.parent().as_ref() == Some(path)) {
            return Err(io_err(format!("directory not empty: {path}")));
        }
        st.nodes.remove(path);
        Ok(())
    }

    async fn chmod(&self, path: &RemotePath, mode: u32) -> Result<()> {
        let mut st = self.lock();
        st.check_write(path)?;
        if let Some(Node::File { mode: m, .. }) = st.nodes.get_mut(path) {
            *m = mode;
            Ok(())
        } else if st.nodes.contains_key(path) {
            Ok(())
        } else {
            Err(not_found(path))
        }
    }

    async fn set_modified(&self, path: &RemotePath, mtime: SystemTime) -> Result<()> {
        let mut st = self.lock();
        match st.nodes.get_mut(path) {
            Some(Node::File { mtime: m, .. }) | Some(Node::Dir { mtime: m }) => {
                *m = mtime;
                Ok(())
            }
            Some(Node::Symlink { .. }) => Ok(()),
            None => Err(not_found(path)),
        }
    }
}

fn is_under(path: &RemotePath, ancestor: &RemotePath) -> bool {
    let mut cur = path.parent();
    while let Some(c) = cur {
        if &c == ancestor {
            return true;
        }
        cur = c.parent();
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use hoprs_core::ErrorKind;
    use std::time::UNIX_EPOCH;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn p(s: &str) -> RemotePath {
        RemotePath::new(s).unwrap()
    }

    #[tokio::test]
    async fn list_read_write_and_errors() {
        let fs = MemFs::new();
        fs.add_file("/a/b.txt", b"hello", UNIX_EPOCH);
        let list = fs.list(&p("/a")).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "b.txt");
        assert_eq!(list[0].size, 5);
        let mut s = String::new();
        fs.open_read(&p("/a/b.txt"), 2)
            .await
            .unwrap()
            .read_to_string(&mut s)
            .await
            .unwrap();
        assert_eq!(s, "llo");
        let mut w = fs.open_write(&p("/a/b.txt"), 3).await.unwrap();
        w.write_all(b"p!").await.unwrap();
        assert_eq!(fs.read_file("/a/b.txt").unwrap(), b"help!");
        assert_eq!(
            fs.stat(&p("/nope")).await.unwrap_err().kind,
            ErrorKind::NotFound
        );
        fs.deny_writes_under("/ro");
        assert_eq!(
            fs.mkdir(&p("/ro/x")).await.unwrap_err().kind,
            ErrorKind::Permission
        );
        fs.add_symlink("/a/link", "/a/b.txt");
        assert!(
            fs.list(&p("/a"))
                .await
                .unwrap()
                .iter()
                .any(|e| e.name == "link" && e.kind == EntryKind::Symlink)
        );
        assert_eq!(
            fs.stat(&p("/a/link")).await.unwrap().kind,
            EntryKind::Symlink
        );
    }

    #[tokio::test]
    async fn mkdir_is_idempotent_and_leaf_only() {
        let fs = MemFs::new();
        fs.mkdir(&p("/d")).await.unwrap();
        fs.mkdir(&p("/d")).await.unwrap();
        assert_eq!(fs.stat(&p("/d")).await.unwrap().kind, EntryKind::Dir);
        assert_eq!(
            fs.mkdir(&p("/x/y")).await.unwrap_err().kind,
            ErrorKind::NotFound
        );
    }

    #[tokio::test]
    async fn rename_file_and_dir_subtree() {
        let fs = MemFs::new();
        fs.add_file("/a.txt", b"x", UNIX_EPOCH);
        fs.rename(&p("/a.txt"), &p("/b.txt")).await.unwrap();
        assert!(fs.read_file("/a.txt").is_none());
        assert_eq!(fs.read_file("/b.txt").unwrap(), b"x");
        fs.add_file("/d/e/f.txt", b"y", UNIX_EPOCH);
        fs.rename(&p("/d"), &p("/g")).await.unwrap();
        assert_eq!(fs.read_file("/g/e/f.txt").unwrap(), b"y");
        assert_eq!(
            fs.stat(&p("/d")).await.unwrap_err().kind,
            ErrorKind::NotFound
        );
    }

    #[tokio::test]
    async fn remove_dir_non_empty_fails() {
        let fs = MemFs::new();
        fs.add_file("/d/f", b"", UNIX_EPOCH);
        assert_eq!(
            fs.remove_dir(&p("/d")).await.unwrap_err().kind,
            ErrorKind::Io
        );
        fs.remove_file(&p("/d/f")).await.unwrap();
        fs.remove_dir(&p("/d")).await.unwrap();
    }

    #[tokio::test]
    async fn open_write_zero_creates_empty_file() {
        let fs = MemFs::new();
        let _w = fs.open_write(&p("/new"), 0).await.unwrap();
        assert_eq!(fs.read_file("/new").unwrap(), b"");
        assert_eq!(
            fs.open_write(&p("/new"), 5).await.err().unwrap().kind,
            ErrorKind::Io
        );
    }
}
