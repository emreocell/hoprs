//! Fault-injecting `RemoteFs` wrapper for tests.
//!
//! Wraps any `RemoteFs` and can fail a chosen operation once, fail the byte
//! stream once after a cumulative number of bytes, and delay every call.

use std::collections::HashMap;
use std::io;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Context, Poll};
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use hoprs_core::{Error, ErrorKind, RemoteEntry, RemotePath, Result};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use crate::{BoxAsyncRead, BoxAsyncWrite, RemoteFs};

/// Operations that `FaultFs::fail_next` can target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Op {
    List,
    Stat,
    OpenRead,
    OpenWrite,
    Mkdir,
}

#[derive(Default)]
struct State {
    ops: usize,
    latency: Duration,
    fail_next: HashMap<Op, ErrorKind>,
    /// Cumulative bytes read from and written to streams.
    bytes: u64,
    /// One-shot stream failure: (threshold, kind).
    fail_after: Option<(u64, ErrorKind)>,
}

type Shared = Arc<Mutex<State>>;

fn lock(s: &Shared) -> MutexGuard<'_, State> {
    s.lock().unwrap_or_else(|e| e.into_inner())
}

/// Maps a core error kind to an `io::ErrorKind` such that
/// `Error::from(io::Error::new(io_kind_for(kind), ..))` recovers `kind` for
/// `Network`, `NotFound`, `Permission` and `Io`. Every other kind maps to
/// `Other`, which converts back to `Io`.
fn io_kind_for(kind: ErrorKind) -> io::ErrorKind {
    match kind {
        ErrorKind::Network => io::ErrorKind::ConnectionReset,
        ErrorKind::NotFound => io::ErrorKind::NotFound,
        ErrorKind::Permission => io::ErrorKind::PermissionDenied,
        _ => io::ErrorKind::Other,
    }
}

/// A `RemoteFs` that forwards to `inner` while injecting faults.
pub struct FaultFs<F: RemoteFs> {
    inner: F,
    state: Shared,
}

impl<F: RemoteFs + Clone> Clone for FaultFs<F> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            state: Arc::clone(&self.state),
        }
    }
}

impl<F: RemoteFs> FaultFs<F> {
    pub fn new(inner: F) -> Self {
        Self {
            inner,
            state: Shared::default(),
        }
    }

    /// Fails once, when the cumulative bytes read or written through streams
    /// (counted from this call) would exceed `bytes`. Data up to the threshold
    /// is still delivered; the next read/write then fails with an `io::Error`
    /// that `Error::from` maps back to `kind` (see `io_kind_for`).
    pub fn fail_io_after(&self, bytes: u64, kind: ErrorKind) {
        let mut st = lock(&self.state);
        st.bytes = 0;
        st.fail_after = Some((bytes, kind));
    }

    /// The next call of `op` fails once with `kind`.
    pub fn fail_next(&self, op: Op, kind: ErrorKind) {
        lock(&self.state).fail_next.insert(op, kind);
    }

    /// Delay applied before every trait operation.
    pub fn set_latency(&self, d: Duration) {
        lock(&self.state).latency = d;
    }

    /// Number of trait calls made so far.
    pub fn ops(&self) -> usize {
        lock(&self.state).ops
    }

    /// Counts the call, applies latency, then consumes a pending `fail_next`.
    async fn enter(&self, op: Option<Op>) -> Result<()> {
        let latency = {
            let mut st = lock(&self.state);
            st.ops += 1;
            st.latency
        };
        if !latency.is_zero() {
            tokio::time::sleep(latency).await;
        }
        if let Some(op) = op {
            let kind = lock(&self.state).fail_next.remove(&op);
            if let Some(kind) = kind {
                return Err(Error::new(kind, format!("injected failure: {op:?}")));
            }
        }
        Ok(())
    }
}

#[async_trait]
impl<F: RemoteFs> RemoteFs for FaultFs<F> {
    async fn list(&self, path: &RemotePath) -> Result<Vec<RemoteEntry>> {
        self.enter(Some(Op::List)).await?;
        self.inner.list(path).await
    }

    async fn stat(&self, path: &RemotePath) -> Result<RemoteEntry> {
        self.enter(Some(Op::Stat)).await?;
        self.inner.stat(path).await
    }

    async fn open_read(&self, path: &RemotePath, offset: u64) -> Result<BoxAsyncRead> {
        self.enter(Some(Op::OpenRead)).await?;
        let inner = self.inner.open_read(path, offset).await?;
        Ok(Box::new(FaultRead {
            inner,
            state: Arc::clone(&self.state),
        }))
    }

    async fn open_write(&self, path: &RemotePath, offset: u64) -> Result<BoxAsyncWrite> {
        self.enter(Some(Op::OpenWrite)).await?;
        let inner = self.inner.open_write(path, offset).await?;
        Ok(Box::new(FaultWrite {
            inner,
            state: Arc::clone(&self.state),
        }))
    }

    async fn mkdir(&self, path: &RemotePath) -> Result<()> {
        self.enter(Some(Op::Mkdir)).await?;
        self.inner.mkdir(path).await
    }

    async fn rename(&self, from: &RemotePath, to: &RemotePath) -> Result<()> {
        self.enter(None).await?;
        self.inner.rename(from, to).await
    }

    async fn remove_file(&self, path: &RemotePath) -> Result<()> {
        self.enter(None).await?;
        self.inner.remove_file(path).await
    }

    async fn remove_dir(&self, path: &RemotePath) -> Result<()> {
        self.enter(None).await?;
        self.inner.remove_dir(path).await
    }

    async fn chmod(&self, path: &RemotePath, mode: u32) -> Result<()> {
        self.enter(None).await?;
        self.inner.chmod(path, mode).await
    }

    async fn set_modified(&self, path: &RemotePath, mtime: SystemTime) -> Result<()> {
        self.enter(None).await?;
        self.inner.set_modified(path, mtime).await
    }
}

/// Decides how many bytes a stream call may move. `Err` means the armed
/// failure fires now (and is disarmed); `Ok(n)` is the cap for this call.
fn allowance(state: &Shared, want: usize) -> io::Result<usize> {
    let mut st = lock(state);
    let Some((limit, kind)) = st.fail_after else {
        return Ok(want);
    };
    if st.bytes >= limit {
        st.fail_after = None;
        return Err(io::Error::new(io_kind_for(kind), "injected I/O failure"));
    }
    let remaining = limit - st.bytes;
    Ok(want.min(usize::try_from(remaining).unwrap_or(usize::MAX)))
}

struct FaultRead {
    inner: BoxAsyncRead,
    state: Shared,
}

impl AsyncRead for FaultRead {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if buf.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        let cap = match allowance(&self.state, buf.remaining()) {
            Ok(n) => n,
            Err(e) => return Poll::Ready(Err(e)),
        };
        let mut sub = ReadBuf::new(buf.initialize_unfilled_to(cap));
        match Pin::new(&mut self.inner).poll_read(cx, &mut sub) {
            Poll::Ready(Ok(())) => {
                let n = sub.filled().len();
                buf.advance(n);
                lock(&self.state).bytes += n as u64;
                Poll::Ready(Ok(()))
            }
            other => other,
        }
    }
}

struct FaultWrite {
    inner: BoxAsyncWrite,
    state: Shared,
}

impl AsyncWrite for FaultWrite {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        if buf.is_empty() {
            return Poll::Ready(Ok(0));
        }
        let cap = match allowance(&self.state, buf.len()) {
            Ok(n) => n,
            Err(e) => return Poll::Ready(Err(e)),
        };
        match Pin::new(&mut self.inner).poll_write(cx, &buf[..cap]) {
            Poll::Ready(Ok(n)) => {
                lock(&self.state).bytes += n as u64;
                Poll::Ready(Ok(n))
            }
            other => other,
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemFs;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn p(s: &str) -> RemotePath {
        RemotePath::new(s).unwrap()
    }

    fn fs_with_hello() -> FaultFs<MemFs> {
        let mem = MemFs::new();
        mem.add_file("/f", b"hello", SystemTime::UNIX_EPOCH);
        FaultFs::new(mem)
    }

    #[tokio::test]
    async fn fail_io_after_errors_once_then_recovers() {
        let fs = fs_with_hello();
        fs.fail_io_after(3, ErrorKind::Network);

        let mut r = fs.open_read(&p("/f"), 0).await.unwrap();
        let mut out = Vec::new();
        let io_err = r.read_to_end(&mut out).await.unwrap_err();
        assert_eq!(Error::from(io_err).kind, ErrorKind::Network);
        assert_eq!(out, b"hel");

        let mut r = fs.open_read(&p("/f"), 0).await.unwrap();
        let mut out = Vec::new();
        r.read_to_end(&mut out).await.unwrap();
        assert_eq!(out, b"hello");
    }

    #[tokio::test]
    async fn fail_io_after_counts_writes_in_the_same_counter() {
        let fs = fs_with_hello();
        fs.fail_io_after(7, ErrorKind::Permission);

        let mut r = fs.open_read(&p("/f"), 0).await.unwrap();
        let mut out = Vec::new();
        r.read_to_end(&mut out).await.unwrap();

        let mut w = fs.open_write(&p("/g"), 0).await.unwrap();
        let io_err = w.write_all(b"abcd").await.unwrap_err();
        assert_eq!(Error::from(io_err).kind, ErrorKind::Permission);
        w.write_all(b"abcd").await.unwrap();
    }

    #[tokio::test]
    async fn fail_io_after_budget_counts_from_arming() {
        let fs = fs_with_hello();
        let mut r = fs.open_read(&p("/f"), 0).await.unwrap();
        let mut out = Vec::new();
        r.read_to_end(&mut out).await.unwrap();
        assert_eq!(out, b"hello");

        fs.fail_io_after(3, ErrorKind::Network);
        let mut r = fs.open_read(&p("/f"), 0).await.unwrap();
        let mut out = Vec::new();
        let io_err = r.read_to_end(&mut out).await.unwrap_err();
        assert_eq!(out, b"hel");
        assert_eq!(Error::from(io_err).kind, ErrorKind::Network);
    }

    #[tokio::test]
    async fn fail_next_fails_once() {
        let fs = fs_with_hello();
        fs.fail_next(Op::Stat, ErrorKind::NotFound);
        let err = fs.stat(&p("/f")).await.unwrap_err();
        assert_eq!(err.kind, ErrorKind::NotFound);
        assert!(fs.stat(&p("/f")).await.is_ok());
        assert_eq!(fs.ops(), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn latency_delays_each_operation() {
        let fs = fs_with_hello();
        fs.set_latency(Duration::from_millis(50));
        let start = tokio::time::Instant::now();
        fs.stat(&p("/f")).await.unwrap();
        assert!(start.elapsed() >= Duration::from_millis(50));
    }
}
