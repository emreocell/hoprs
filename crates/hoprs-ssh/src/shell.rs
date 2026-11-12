//! Interactive PTY shell channel.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use hoprs_core::{Error, ErrorKind, Result};
use russh::client::Msg;
use russh::{ChannelMsg, ChannelReadHalf, ChannelWriteHalf};
use tokio::sync::mpsc;

use crate::session::{SshSession, map_russh_error};

/// Capacity of the output queue. When the consumer is slow the reader task waits for room
/// instead of buffering without bound, which in turn stops draining the SSH channel and lets
/// the channel window throttle the remote side.
const OUTPUT_QUEUE: usize = 256;

/// Terminal request for [`SshSession::open_shell`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PtyRequest {
    pub term: String,
    pub cols: u32,
    pub rows: u32,
}

impl Default for PtyRequest {
    fn default() -> Self {
        Self {
            term: "xterm-256color".to_owned(),
            cols: 80,
            rows: 24,
        }
    }
}

/// What a shell produces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellOutput {
    Data(Vec<u8>),
    Exit(Option<u32>),
    Closed,
}

/// An open shell on a PTY. Dropping it closes the remote channel.
///
/// `output()` ends with exactly one [`ShellOutput::Closed`] (channel closed or connection
/// lost), after which the receiver yields `None`. Reconnecting a shell is up to the caller.
#[derive(Debug)]
pub struct ShellChannel {
    writer: Arc<ChannelWriteHalf<Msg>>,
    closed: Arc<AtomicBool>,
    rx: mpsc::Receiver<ShellOutput>,
}

fn shell_closed() -> Error {
    Error::new(ErrorKind::Network, "Shell closed")
}

impl ShellChannel {
    fn check_open(&self) -> Result<()> {
        if self.closed.load(Ordering::SeqCst) {
            Err(shell_closed())
        } else {
            Ok(())
        }
    }

    /// Sends `data` to the shell's stdin (the terminal input).
    pub async fn write(&self, data: &[u8]) -> Result<()> {
        self.check_open()?;
        self.writer
            .data_bytes(data.to_vec())
            .await
            .map_err(|e| self.map_send(e))
    }

    /// Tells the server the terminal is now `cols` x `rows` characters (window-change).
    pub async fn resize(&self, cols: u32, rows: u32) -> Result<()> {
        self.check_open()?;
        self.writer
            .window_change(cols, rows, 0, 0)
            .await
            .map_err(|e| self.map_send(e))
    }

    /// Asks the server to close the channel; `Closed` then arrives on `output()`.
    /// Closing an already closed shell is not an error.
    pub async fn close(&self) -> Result<()> {
        if self.closed.load(Ordering::SeqCst) {
            return Ok(());
        }
        // A failed send means the channel is already gone, which is what was asked for.
        let _ = self.writer.close().await;
        Ok(())
    }

    /// The shell's output: stdout and stderr merged as `Data`, then `Exit`, then `Closed`.
    pub fn output(&mut self) -> &mut mpsc::Receiver<ShellOutput> {
        &mut self.rx
    }

    fn map_send(&self, e: russh::Error) -> Error {
        match e {
            russh::Error::SendError => shell_closed(),
            other => map_russh_error(other),
        }
    }
}

impl Drop for ShellChannel {
    fn drop(&mut self) {
        if self.closed.load(Ordering::SeqCst) {
            return;
        }
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            let writer = self.writer.clone();
            runtime.spawn(async move {
                let _ = writer.close().await;
            });
        }
    }
}

/// Waits for the reply to a channel request (`Success`/`Failure`), skipping other messages.
async fn expect_reply(reader: &mut ChannelReadHalf, what: &str) -> Result<()> {
    loop {
        match reader.wait().await {
            Some(ChannelMsg::Success) => return Ok(()),
            Some(ChannelMsg::Failure) => {
                return Err(Error::new(
                    ErrorKind::Network,
                    format!("The server refused the {what} request"),
                ));
            }
            Some(ChannelMsg::Close | ChannelMsg::Eof) | None => {
                return Err(Error::new(
                    ErrorKind::Network,
                    format!("Connection closed during the {what} request"),
                ));
            }
            Some(_) => {}
        }
    }
}

async fn pump(mut reader: ChannelReadHalf, tx: mpsc::Sender<ShellOutput>, closed: Arc<AtomicBool>) {
    while let Some(msg) = reader.wait().await {
        let item = match msg {
            ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. } => {
                ShellOutput::Data(data.to_vec())
            }
            ChannelMsg::ExitStatus { exit_status } => ShellOutput::Exit(Some(exit_status)),
            ChannelMsg::ExitSignal { .. } => ShellOutput::Exit(None),
            ChannelMsg::Close => break,
            _ => continue,
        };
        if tx.send(item).await.is_err() {
            // The consumer is gone.
            closed.store(true, Ordering::SeqCst);
            return;
        }
    }
    closed.store(true, Ordering::SeqCst);
    let _ = tx.send(ShellOutput::Closed).await;
}

impl SshSession {
    /// Opens a channel, requests a PTY (`req`, default terminal modes) and a shell, and starts
    /// forwarding the shell's output. While the session is not connected this fails at once
    /// with `Network`, "Not connected".
    pub async fn open_shell(&self, req: PtyRequest) -> Result<ShellChannel> {
        let channel = self.open_channel().await?;
        let (mut reader, writer) = channel.split();
        writer
            .request_pty(true, &req.term, req.cols, req.rows, 0, 0, &[])
            .await
            .map_err(map_russh_error)?;
        expect_reply(&mut reader, "pty").await?;
        writer.request_shell(true).await.map_err(map_russh_error)?;
        expect_reply(&mut reader, "shell").await?;

        let (tx, rx) = mpsc::channel(OUTPUT_QUEUE);
        let closed = Arc::new(AtomicBool::new(false));
        tokio::spawn(pump(reader, tx, closed.clone()));
        Ok(ShellChannel {
            writer: Arc::new(writer),
            closed,
            rx,
        })
    }
}
