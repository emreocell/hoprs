//! [`FtpFs`](super::FtpFs) data streams: downloads (`RETR`) and uploads (`STOR`, `APPE`),
//! each on a transfer connection checked out of the session's pool.
//!
//! A stream owns its [`PooledConn`] until the transfer's final reply has been read. Only a
//! positive final reply sends the connection back to the pool ([`PooledConn::mark_clean`]).
//! After an error, or when the stream is dropped before its end, the connection is closed, so
//! a connection with a reply still pending is never reused. A stream dropped mid-transfer
//! sends a best-effort `ABOR` from a background task first, without waiting for it.

use std::fmt::Display;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll, ready};
use std::time::Duration;

use futures_util::future::BoxFuture;
use hoprs_core::{Error, ErrorKind};
use suppaftp::FtpError;
use suppaftp::tokio::{AsyncRustlsStream, TransferStream};
use tokio::io::{AsyncRead, AsyncWrite, BufWriter, ReadBuf};

use super::errors::{data_channel_lost, map_ftp_error, map_reply, reply_parts};
use super::session::PooledConn;
use crate::io_err::to_io;

/// The data connection of a transfer, as suppaftp hands it out.
pub(crate) type Transfer = TransferStream<AsyncRustlsStream>;

/// Upload buffer (spec §5): data reaches the socket in writes of this size.
const WRITE_BUFFER: usize = 256 * 1024;

/// How long a stream waits for the server.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Timeouts {
    /// Bounds a transfer's final reply ([`super::FtpConfig::command_timeout`]).
    pub(crate) command: Duration,
    /// Bounds a reply that should come at once: a `NOOP`, an `ABOR`
    /// ([`super::FtpConfig::connect_timeout`]).
    pub(crate) reply: Duration,
}

/// The connection that carried the transfer is lost (or the server is gone).
fn lost(context: &str, detail: impl Display) -> Error {
    Error::new(ErrorKind::Network, format!("Connection lost: {context}"))
        .with_detail(detail.to_string())
}

/// Closes `conn` after a best-effort `ABOR` sent from a background task. Never waits.
fn abort_in_background(mut conn: PooledConn, transfer: Transfer, timeout: Duration) {
    match tokio::runtime::Handle::try_current() {
        Ok(runtime) => {
            runtime.spawn(async move {
                // The connection is closed whatever the outcome: never marked clean.
                let _ = tokio::time::timeout(timeout, conn.stream().abort(transfer)).await;
            });
        }
        // No runtime (dropped outside of one): closing the sockets ends the transfer too.
        Err(_) => drop((transfer, conn)),
    }
}

/// Ends the transfer: shuts the data connection down and reads the final reply. A positive
/// reply sends `conn` back to the pool; anything else closes it.
async fn finish(
    mut conn: PooledConn,
    transfer: Transfer,
    upload: bool,
    context: String,
    timeouts: Timeouts,
) -> Result<(), Error> {
    let e = match tokio::time::timeout(timeouts.command, transfer.finish()).await {
        Ok(Ok(())) => {
            conn.mark_clean();
            return Ok(());
        }
        // suppaftp reads the final reply and then reports a failed shutdown of an upload's
        // data connection instead of it. The connection still answering means the server
        // closed the data connection before taking the whole file: it refused the upload
        // (a full disk, say), which is about this file, not the network.
        Ok(Err(FtpError::ConnectionError(io))) if upload => {
            match tokio::time::timeout(timeouts.reply, conn.stream().noop()).await {
                Ok(Ok(())) => {
                    return Err(Error::new(
                        ErrorKind::Io,
                        format!("Upload refused by the server: {context}"),
                    )
                    .with_detail(format!("the server closed the data connection: {io}")));
                }
                _ => lost(&context, io),
            }
        }
        Ok(Err(e)) => map_ftp_error(e, &context),
        Err(_) => lost(&context, "no reply after the end of the transfer"),
    };
    conn.report(&e);
    Err(e)
}

/// Why an upload's data connection failed with `err`: the server's reply when it sent a
/// negative one (a full disk is `451`, `452` or `552`: `Io`), `err` otherwise. Asks with
/// `ABOR`, which reads the replies pending on the connection. `conn` is closed afterwards.
async fn upload_failed(
    mut conn: PooledConn,
    transfer: Transfer,
    err: io::Error,
    context: String,
    timeouts: Timeouts,
) -> Error {
    let e = match tokio::time::timeout(timeouts.reply, conn.stream().abort(transfer)).await {
        Ok(Err(FtpError::UnexpectedResponse(reply))) => {
            let (code, text) = reply_parts(&reply);
            if code >= 400 {
                map_reply(code, &text, &context)
            } else {
                data_channel_lost(&context, &err)
            }
        }
        // `426`/`226`: the server saw the data connection fail too.
        Ok(Ok(())) => data_channel_lost(&context, &err),
        Ok(Err(e)) => map_ftp_error(e, &context),
        Err(_) => lost(&context, err),
    };
    conn.report(&e);
    e
}

/// A download: reads the data connection, then the final reply at its end.
pub(crate) struct FtpReader {
    state: ReadState,
    /// The path, for errors.
    context: String,
    timeouts: Timeouts,
}

// Streams live boxed, almost always in the large variant: boxing it would only add an
// allocation.
#[allow(clippy::large_enum_variant)]
enum ReadState {
    Reading(PooledConn, Transfer),
    /// The data ended; waiting for the final reply.
    Finishing(BoxFuture<'static, Result<(), Error>>),
    Done,
    Failed(Error),
}

impl FtpReader {
    pub(crate) fn new(
        conn: PooledConn,
        transfer: Transfer,
        context: String,
        timeouts: Timeouts,
    ) -> Self {
        Self {
            state: ReadState::Reading(conn, transfer),
            context,
            timeouts,
        }
    }
}

impl AsyncRead for FtpReader {
    /// End of file only once the final reply says the transfer is complete.
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        loop {
            match &mut this.state {
                ReadState::Reading(_, transfer) => {
                    if buf.remaining() == 0 {
                        return Poll::Ready(Ok(()));
                    }
                    let before = buf.filled().len();
                    let failure = match ready!(Pin::new(transfer).poll_read(cx, buf)) {
                        Ok(()) if buf.filled().len() > before => return Poll::Ready(Ok(())),
                        Ok(()) => None,
                        Err(e) => Some(e),
                    };
                    let ReadState::Reading(conn, transfer) =
                        std::mem::replace(&mut this.state, ReadState::Done)
                    else {
                        continue;
                    };
                    this.state = match failure {
                        None => ReadState::Finishing(Box::pin(finish(
                            conn,
                            transfer,
                            false,
                            this.context.clone(),
                            this.timeouts,
                        ))),
                        // The server may be gone: `report` has the session check.
                        Some(e) => {
                            drop(transfer);
                            let e = lost(&this.context, e);
                            conn.report(&e);
                            ReadState::Failed(e)
                        }
                    };
                }
                ReadState::Finishing(reply) => {
                    this.state = match ready!(reply.as_mut().poll(cx)) {
                        Ok(()) => ReadState::Done,
                        Err(e) => ReadState::Failed(e),
                    };
                }
                ReadState::Done => return Poll::Ready(Ok(())),
                ReadState::Failed(e) => return Poll::Ready(Err(to_io(e.clone()))),
            }
        }
    }
}

impl Drop for FtpReader {
    fn drop(&mut self) {
        if let ReadState::Reading(conn, transfer) =
            std::mem::replace(&mut self.state, ReadState::Done)
        {
            abort_in_background(conn, transfer, self.timeouts.reply);
        }
    }
}

/// An upload: writes through a 256 KiB buffer; `shutdown` returns `Ok` only after the
/// server's positive final reply.
pub(crate) struct FtpWriter {
    state: WriteState,
    /// The path, for errors.
    context: String,
    timeouts: Timeouts,
}

// Streams live boxed, almost always in the large variant: boxing it would only add an
// allocation.
#[allow(clippy::large_enum_variant)]
enum WriteState {
    Open(PooledConn, BufWriter<Transfer>),
    /// Waiting for the final reply (`shutdown`), or for the server's reason for a failure.
    Closing(BoxFuture<'static, Result<(), Error>>),
    Done,
    Failed(Error),
}

impl FtpWriter {
    pub(crate) fn new(
        conn: PooledConn,
        transfer: Transfer,
        context: String,
        timeouts: Timeouts,
    ) -> Self {
        Self {
            state: WriteState::Open(conn, BufWriter::with_capacity(WRITE_BUFFER, transfer)),
            context,
            timeouts,
        }
    }

    /// The data connection failed with `err`: ask the server why (see [`upload_failed`]).
    fn fail(&mut self, err: io::Error) {
        if let WriteState::Open(conn, data) = std::mem::replace(&mut self.state, WriteState::Done) {
            let (context, timeouts) = (self.context.clone(), self.timeouts);
            self.state = WriteState::Closing(Box::pin(async move {
                Err(upload_failed(conn, data.into_inner(), err, context, timeouts).await)
            }));
        }
    }

    /// Drives `Closing` to `Done` or `Failed`; `Ready` with the state's outcome otherwise.
    /// Never called in `Open`.
    fn poll_closed(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        loop {
            match &mut self.state {
                WriteState::Closing(reply) => {
                    self.state = match ready!(reply.as_mut().poll(cx)) {
                        Ok(()) => WriteState::Done,
                        Err(e) => WriteState::Failed(e),
                    };
                }
                WriteState::Failed(e) => return Poll::Ready(Err(to_io(e.clone()))),
                WriteState::Done | WriteState::Open(..) => return Poll::Ready(Ok(())),
            }
        }
    }
}

impl AsyncWrite for FtpWriter {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        if let WriteState::Open(_, data) = &mut this.state {
            match ready!(Pin::new(data).poll_write(cx, buf)) {
                Ok(n) => return Poll::Ready(Ok(n)),
                Err(e) => this.fail(e),
            }
        }
        ready!(this.poll_closed(cx))?;
        Poll::Ready(Err(to_io(Error::new(
            ErrorKind::Io,
            format!("Write after the end of the upload: {}", this.context),
        ))))
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if let WriteState::Open(_, data) = &mut this.state {
            match ready!(Pin::new(data).poll_flush(cx)) {
                Ok(()) => return Poll::Ready(Ok(())),
                Err(e) => this.fail(e),
            }
        }
        this.poll_closed(cx)
    }

    /// Flushes the buffer, closes the data connection and reads the final reply.
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if let WriteState::Open(_, data) = &mut this.state {
            match ready!(Pin::new(data).poll_flush(cx)) {
                Ok(()) => {
                    if let WriteState::Open(conn, data) =
                        std::mem::replace(&mut this.state, WriteState::Done)
                    {
                        this.state = WriteState::Closing(Box::pin(finish(
                            conn,
                            data.into_inner(),
                            true,
                            this.context.clone(),
                            this.timeouts,
                        )));
                    }
                }
                Err(e) => this.fail(e),
            }
        }
        this.poll_closed(cx)
    }
}

impl Drop for FtpWriter {
    fn drop(&mut self) {
        if let WriteState::Open(conn, data) = std::mem::replace(&mut self.state, WriteState::Done) {
            abort_in_background(conn, data.into_inner(), self.timeouts.reply);
        }
    }
}
