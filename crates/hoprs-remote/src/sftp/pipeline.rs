//! Pipelined SFTP streams: up to `MAX_IN_FLIGHT` requests of `CHUNK` bytes
//! outstanding per file, exposed as `AsyncRead` / `AsyncWrite`.
//!
//! The adapters run over the `SftpIo` trait so they can be tested without a
//! network; the real implementation wraps an open russh-sftp file handle.

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, ready};

use async_trait::async_trait;
use futures_util::StreamExt;
use futures_util::stream::{FuturesOrdered, FuturesUnordered};
use hoprs_core::{Error, Result};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use crate::io_err::to_io;

/// Maximum number of requests outstanding per stream.
pub const MAX_IN_FLIGHT: usize = 64;
/// Size of each read/write request.
pub const CHUNK: u32 = 32 * 1024;

/// Positioned I/O on one open remote file.
#[async_trait]
pub trait SftpIo: Send + Sync + 'static {
    /// Reads up to `len` bytes at `offset`. An empty result means EOF; a
    /// shorter, non-empty result is allowed anywhere.
    async fn read(&self, offset: u64, len: u32) -> Result<Vec<u8>>;
    /// Writes all of `data` at `offset`; `Ok` means the server acknowledged it.
    async fn write(&self, offset: u64, data: Vec<u8>) -> Result<()>;
    /// Closes the file handle.
    async fn close(&self) -> Result<()>;
}

type BoxFut<O> = Pin<Box<dyn Future<Output = O> + Send>>;

/// A read request's outcome, tagged with the range it asked for.
type ReadReply = (u64, u32, Result<Vec<u8>>);

enum ReadState {
    Active,
    Eof,
    Failed(Error),
}

/// Streams a remote file from an offset with pipelined read requests.
///
/// Requests cover consecutive `CHUNK`-sized ranges and are delivered strictly
/// in order. A short reply is followed by a request for the rest of its range
/// before anything past it is delivered. An empty reply is EOF: requests
/// still in flight are dropped. After an error every read fails again.
pub struct PipelinedReader<T: SftpIo> {
    io: Arc<T>,
    /// Offset of the next request to issue.
    next: u64,
    in_flight: FuturesOrdered<BoxFut<ReadReply>>,
    /// Received bytes not yet handed to the consumer, from `pos`.
    buf: Vec<u8>,
    pos: usize,
    state: ReadState,
}

impl<T: SftpIo> PipelinedReader<T> {
    pub fn new(io: Arc<T>, offset: u64) -> Self {
        Self {
            io,
            next: offset,
            in_flight: FuturesOrdered::new(),
            buf: Vec::new(),
            pos: 0,
            state: ReadState::Active,
        }
    }

    fn request(&self, offset: u64, len: u32) -> BoxFut<ReadReply> {
        let io = Arc::clone(&self.io);
        Box::pin(async move {
            let res = io.read(offset, len).await;
            (offset, len, res)
        })
    }

    fn fill(&mut self) {
        while self.in_flight.len() < MAX_IN_FLIGHT {
            let Some(after) = self.next.checked_add(u64::from(CHUNK)) else {
                break;
            };
            let req = self.request(self.next, CHUNK);
            self.in_flight.push_back(req);
            self.next = after;
        }
    }

    fn fail(&mut self, err: Error) {
        self.in_flight = FuturesOrdered::new();
        self.state = ReadState::Failed(err);
    }
}

impl<T: SftpIo> AsyncRead for PipelinedReader<T> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        out: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = &mut *self;
        if out.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        loop {
            if this.pos < this.buf.len() {
                let n = out.remaining().min(this.buf.len() - this.pos);
                out.put_slice(&this.buf[this.pos..this.pos + n]);
                this.pos += n;
                return Poll::Ready(Ok(()));
            }
            match &this.state {
                ReadState::Active => {}
                ReadState::Eof => return Poll::Ready(Ok(())),
                ReadState::Failed(e) => return Poll::Ready(Err(to_io(e.clone()))),
            }
            this.fill();
            let Some((offset, len, res)) = ready!(this.in_flight.poll_next_unpin(cx)) else {
                // Only reachable when offsets ran out at u64::MAX.
                this.state = ReadState::Eof;
                continue;
            };
            match res {
                Err(e) => this.fail(e),
                Ok(data) if data.is_empty() => {
                    this.in_flight = FuturesOrdered::new();
                    this.state = ReadState::Eof;
                }
                Ok(mut data) => {
                    data.truncate(len as usize);
                    let got = data.len() as u32;
                    if got < len {
                        let rest = this.request(offset + u64::from(got), len - got);
                        this.in_flight.push_front(rest);
                    }
                    this.buf = data;
                    this.pos = 0;
                }
            }
        }
    }
}

/// Writes a remote file from an offset with pipelined write requests.
///
/// Bytes are gathered into `CHUNK`-sized requests that are sent without
/// waiting for the reply, at most `MAX_IN_FLIGHT` at a time. The first failed
/// request makes every later call fail. `poll_shutdown` sends the tail, waits
/// for every acknowledgement and only then closes the handle (exactly once);
/// after a write error it fails without closing.
pub struct PipelinedWriter<T: SftpIo> {
    io: Arc<T>,
    /// Offset of the next request to dispatch.
    next: u64,
    buf: Vec<u8>,
    in_flight: FuturesUnordered<BoxFut<Result<()>>>,
    error: Option<Error>,
    closing: Option<BoxFut<Result<()>>>,
    /// Set once `close` was called; holds its outcome when known.
    closed: Option<Option<Result<()>>>,
}

impl<T: SftpIo> PipelinedWriter<T> {
    pub fn new(io: Arc<T>, offset: u64) -> Self {
        Self {
            io,
            next: offset,
            buf: Vec::with_capacity(CHUNK as usize),
            in_flight: FuturesUnordered::new(),
            error: None,
            closing: None,
            closed: None,
        }
    }

    fn fail(&mut self, err: Error) -> io::Error {
        self.in_flight = FuturesUnordered::new();
        self.error = Some(err.clone());
        to_io(err)
    }

    fn check(&self) -> io::Result<()> {
        match &self.error {
            Some(e) => Err(to_io(e.clone())),
            None => Ok(()),
        }
    }

    /// Sends the buffered bytes as one request. The caller ensures there is
    /// room in the pipeline.
    fn dispatch(&mut self) {
        let data = std::mem::replace(&mut self.buf, Vec::with_capacity(CHUNK as usize));
        let offset = self.next;
        self.next += data.len() as u64;
        let io = Arc::clone(&self.io);
        self.in_flight
            .push(Box::pin(async move { io.write(offset, data).await }));
    }

    /// Collects acknowledgements without blocking; also polls freshly
    /// dispatched requests so they are sent.
    fn poll_acks(&mut self, cx: &mut Context<'_>) -> io::Result<()> {
        while let Poll::Ready(Some(res)) = self.in_flight.poll_next_unpin(cx) {
            if let Err(e) = res {
                return Err(self.fail(e));
            }
        }
        Ok(())
    }

    /// Dispatches any buffered bytes and waits for every acknowledgement.
    fn poll_drain(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.check()?;
        loop {
            if !self.buf.is_empty() && self.in_flight.len() < MAX_IN_FLIGHT {
                self.dispatch();
            }
            match ready!(self.in_flight.poll_next_unpin(cx)) {
                Some(Ok(())) => {}
                Some(Err(e)) => return Poll::Ready(Err(self.fail(e))),
                None if self.buf.is_empty() => return Poll::Ready(Ok(())),
                None => {}
            }
        }
    }
}

impl<T: SftpIo> AsyncWrite for PipelinedWriter<T> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = &mut *self;
        this.check()?;
        if this.closed.is_some() {
            return Poll::Ready(Err(io::Error::other("write after shutdown")));
        }
        if data.is_empty() {
            return Poll::Ready(Ok(0));
        }
        this.poll_acks(cx)?;
        if this.buf.len() == CHUNK as usize {
            if this.in_flight.len() >= MAX_IN_FLIGHT {
                // `poll_acks` saw the full pipeline pending and registered
                // the waker.
                return Poll::Pending;
            }
            this.dispatch();
        }
        let n = data.len().min(CHUNK as usize - this.buf.len());
        this.buf.extend_from_slice(&data[..n]);
        if this.buf.len() == CHUNK as usize && this.in_flight.len() < MAX_IN_FLIGHT {
            this.dispatch();
        }
        this.poll_acks(cx)?;
        Poll::Ready(Ok(n))
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.poll_drain(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = &mut *self;
        if this.closed.is_none() {
            ready!(this.poll_drain(cx))?;
            let io = Arc::clone(&this.io);
            this.closing = Some(Box::pin(async move { io.close().await }));
            this.closed = Some(None);
        }
        if let Some(fut) = this.closing.as_mut() {
            let res = ready!(fut.as_mut().poll(cx));
            this.closing = None;
            this.closed = Some(Some(res));
        }
        match &this.closed {
            Some(Some(Err(e))) => Poll::Ready(Err(to_io(e.clone()))),
            _ => Poll::Ready(Ok(())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hoprs_core::ErrorKind;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const BIG: usize = 3 * 1024 * 1024 + 123;
    const CH: usize = CHUNK as usize;

    fn xorshift(mut x: u64) -> u64 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    }

    /// Deterministic pseudo-random bytes.
    fn data(len: usize, seed: u64) -> Vec<u8> {
        let mut x = seed | 1;
        (0..len)
            .map(|_| {
                x = xorshift(x);
                x as u8
            })
            .collect()
    }

    #[derive(Clone, Copy)]
    enum Delay {
        None,
        Fixed(Duration),
        /// Per-request pseudo-random delay in `0..max_us` microseconds,
        /// derived from the request number so it is reproducible.
        Random {
            seed: u64,
            max_us: u64,
        },
    }

    /// In-memory `SftpIo` that records concurrency and can inject faults.
    struct FakeIo {
        src: Vec<u8>,
        dst: Mutex<Vec<u8>>,
        delay: Delay,
        /// Cap on the bytes returned by one read (forces short reads).
        max_reply: usize,
        /// Fail the n-th request (1-based, reads and writes) with this kind.
        fail_at: Option<(usize, ErrorKind)>,
        requests: AtomicUsize,
        in_flight: AtomicUsize,
        max_in_flight: AtomicUsize,
        closes: AtomicUsize,
    }

    impl FakeIo {
        fn new(src: Vec<u8>) -> Self {
            Self {
                src,
                dst: Mutex::new(Vec::new()),
                delay: Delay::Random {
                    seed: 0x9E37_79B9_7F4A_7C15,
                    max_us: 2000,
                },
                max_reply: usize::MAX,
                fail_at: None,
                requests: AtomicUsize::new(0),
                in_flight: AtomicUsize::new(0),
                max_in_flight: AtomicUsize::new(0),
                closes: AtomicUsize::new(0),
            }
        }

        fn delay(mut self, d: Delay) -> Self {
            self.delay = d;
            self
        }

        fn max_reply(mut self, n: usize) -> Self {
            self.max_reply = n;
            self
        }

        fn fail_at(mut self, n: usize, kind: ErrorKind) -> Self {
            self.fail_at = Some((n, kind));
            self
        }

        fn max_seen(&self) -> usize {
            self.max_in_flight.load(Ordering::SeqCst)
        }

        fn closes(&self) -> usize {
            self.closes.load(Ordering::SeqCst)
        }

        fn dst(&self) -> Vec<u8> {
            self.dst.lock().unwrap().clone()
        }

        /// Counts the request in flight, waits its delay and returns whether
        /// it must fail. The caller must call `leave` afterwards.
        async fn enter(&self) -> Result<()> {
            let n = self.requests.fetch_add(1, Ordering::SeqCst) + 1;
            let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_in_flight.fetch_max(now, Ordering::SeqCst);
            let d = match self.delay {
                Delay::None => Duration::ZERO,
                Delay::Fixed(d) => d,
                Delay::Random { seed, max_us } => Duration::from_micros(
                    xorshift(seed ^ (n as u64).wrapping_mul(0x2545_F491)) % max_us,
                ),
            };
            if !d.is_zero() {
                tokio::time::sleep(d).await;
            }
            match self.fail_at {
                Some((at, kind)) if at == n => {
                    Err(Error::new(kind, format!("injected failure #{n}")))
                }
                _ => Ok(()),
            }
        }

        fn leave(&self) {
            self.in_flight.fetch_sub(1, Ordering::SeqCst);
        }
    }

    #[async_trait]
    impl SftpIo for FakeIo {
        async fn read(&self, offset: u64, len: u32) -> Result<Vec<u8>> {
            let r = self.enter().await;
            self.leave();
            r?;
            let start = usize::try_from(offset).unwrap().min(self.src.len());
            let n = (len as usize).min(self.max_reply);
            let end = start.saturating_add(n).min(self.src.len());
            Ok(self.src[start..end].to_vec())
        }

        async fn write(&self, offset: u64, data: Vec<u8>) -> Result<()> {
            let r = self.enter().await;
            if r.is_ok() {
                let start = usize::try_from(offset).unwrap();
                let mut dst = self.dst.lock().unwrap();
                if dst.len() < start + data.len() {
                    dst.resize(start + data.len(), 0);
                }
                dst[start..start + data.len()].copy_from_slice(&data);
            }
            self.leave();
            r
        }

        async fn close(&self) -> Result<()> {
            self.closes.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    async fn read_all(io: &Arc<FakeIo>, offset: u64) -> Vec<u8> {
        let mut r = PipelinedReader::new(Arc::clone(io), offset);
        let mut out = Vec::new();
        r.read_to_end(&mut out).await.unwrap();
        out
    }

    async fn read_with_buffer(io: &Arc<FakeIo>, size: usize) -> Vec<u8> {
        let mut r = PipelinedReader::new(Arc::clone(io), 0);
        let mut out = Vec::new();
        let mut buf = vec![0u8; size];
        loop {
            let n = r.read(&mut buf).await.unwrap();
            if n == 0 {
                return out;
            }
            out.extend_from_slice(&buf[..n]);
        }
    }

    #[test]
    fn streams_are_send_and_unpin() {
        fn check<S: Send + Unpin>() {}
        check::<PipelinedReader<FakeIo>>();
        check::<PipelinedWriter<FakeIo>>();
    }

    #[tokio::test]
    async fn reads_whole_file_in_order() {
        let src = data(BIG, 1);
        let io = Arc::new(FakeIo::new(src.clone()));
        assert_eq!(read_all(&io, 0).await, src);
    }

    #[tokio::test]
    async fn reads_from_offset() {
        let src = data(BIG, 2);
        let io = Arc::new(FakeIo::new(src.clone()));
        let off = 1_000_003;
        assert_eq!(read_all(&io, off as u64).await, src[off..]);
    }

    #[tokio::test]
    async fn reads_with_small_consumer_buffer() {
        let src = data(BIG, 3);
        let io = Arc::new(FakeIo::new(src.clone()));
        assert_eq!(read_with_buffer(&io, 8 * 1024).await, src);
    }

    #[tokio::test]
    async fn reads_with_large_consumer_buffer() {
        let src = data(BIG, 4);
        let io = Arc::new(FakeIo::new(src.clone()));
        assert_eq!(read_with_buffer(&io, 1024 * 1024).await, src);
    }

    #[tokio::test]
    async fn keeps_64_in_flight() {
        let src = data(2 * MAX_IN_FLIGHT * CH + 5, 5);
        let io = Arc::new(FakeIo::new(src.clone()));
        assert_eq!(read_all(&io, 0).await, src);
        assert_eq!(io.max_seen(), MAX_IN_FLIGHT);
    }

    #[tokio::test]
    async fn short_read_is_completed() {
        let src = data(BIG, 6);
        let io = Arc::new(FakeIo::new(src.clone()).max_reply(10_000));
        assert_eq!(read_all(&io, 0).await, src);
        assert!(io.max_seen() <= MAX_IN_FLIGHT);
    }

    #[tokio::test]
    async fn empty_file_reads_nothing() {
        let io = Arc::new(FakeIo::new(Vec::new()));
        assert!(read_all(&io, 0).await.is_empty());
    }

    #[tokio::test]
    async fn offset_past_eof_reads_nothing() {
        let io = Arc::new(FakeIo::new(data(1000, 7)));
        assert!(read_all(&io, 1000).await.is_empty());
        assert!(read_all(&io, 5000).await.is_empty());
    }

    #[tokio::test]
    async fn read_error_propagates_kind() {
        let src = data(BIG, 8);
        let io = Arc::new(FakeIo::new(src.clone()).fail_at(5, ErrorKind::Network));
        let mut r = PipelinedReader::new(Arc::clone(&io), 0);
        let mut out = Vec::new();
        let err = r.read_to_end(&mut out).await.unwrap_err();
        let core = Error::from(err);
        assert_eq!(core.kind, ErrorKind::Network);
        assert!(core.message.contains("injected failure #5"), "{core:?}");
        // Bytes before the failed request were delivered, nothing past it.
        assert_eq!(out, src[..4 * CH]);
        // The error is sticky.
        let mut buf = [0u8; 16];
        let again = r.read(&mut buf).await.unwrap_err();
        assert_eq!(Error::from(again).kind, ErrorKind::Network);
    }

    async fn write_all(io: &Arc<FakeIo>, offset: u64, bytes: &[u8]) {
        let mut w = PipelinedWriter::new(Arc::clone(io), offset);
        w.write_all(bytes).await.unwrap();
        w.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn writes_whole_file() {
        let src = data(BIG, 9);
        let io = Arc::new(FakeIo::new(Vec::new()));
        write_all(&io, 0, &src).await;
        assert_eq!(io.dst(), src);
        assert_eq!(io.closes(), 1);
    }

    #[tokio::test]
    async fn writes_from_offset() {
        let src = data(100_000, 10);
        let io = Arc::new(FakeIo::new(Vec::new()));
        write_all(&io, 7, &src).await;
        assert_eq!(io.dst()[7..], src);
    }

    #[tokio::test]
    async fn writer_respects_in_flight_limit() {
        let src = data(2 * MAX_IN_FLIGHT * CH + 5, 11);
        let io = Arc::new(FakeIo::new(Vec::new()));
        write_all(&io, 0, &src).await;
        assert_eq!(io.dst(), src);
        assert_eq!(io.max_seen(), MAX_IN_FLIGHT);
    }

    #[tokio::test(start_paused = true)]
    async fn shutdown_waits_for_all_acks() {
        let src = data(BIG, 12);
        let io = Arc::new(FakeIo::new(Vec::new()).delay(Delay::Fixed(Duration::from_millis(50))));
        let mut w = PipelinedWriter::new(Arc::clone(&io), 0);
        w.write_all(&src).await.unwrap();
        w.shutdown().await.unwrap();
        assert_eq!(io.dst(), src);
        assert_eq!(io.closes(), 1);
    }

    #[tokio::test]
    async fn write_error_fails_shutdown() {
        let src = data(BIG, 13);
        let io = Arc::new(FakeIo::new(Vec::new()).fail_at(3, ErrorKind::Permission));
        let mut w = PipelinedWriter::new(Arc::clone(&io), 0);
        let _ = w.write_all(&src).await;
        let err = w.shutdown().await.unwrap_err();
        assert_eq!(Error::from(err).kind, ErrorKind::Permission);
        assert_eq!(io.closes(), 0);
        // Still failing afterwards, and still no close.
        assert!(w.write_all(b"x").await.is_err());
        assert!(w.shutdown().await.is_err());
        assert_eq!(io.closes(), 0);
    }

    #[tokio::test]
    async fn write_error_is_detected_in_shutdown_for_small_files() {
        let io = Arc::new(
            FakeIo::new(Vec::new())
                .delay(Delay::None)
                .fail_at(1, ErrorKind::Network),
        );
        let mut w = PipelinedWriter::new(Arc::clone(&io), 0);
        w.write_all(b"hello").await.unwrap();
        let err = w.shutdown().await.unwrap_err();
        assert_eq!(Error::from(err).kind, ErrorKind::Network);
        assert_eq!(io.closes(), 0);
    }
}
