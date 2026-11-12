//! Interactive PTY shell channel.
//!
//! The output pump never blocks on the consumer: a slow reader cannot stall the connection
//! (see `MAX_PENDING_OUTPUT`).

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use hoprs_core::{Error, ErrorKind, Result};
use russh::client::Msg;
use russh::{ChannelMsg, ChannelReadHalf, ChannelWriteHalf};
use tokio::sync::{Notify, mpsc};
use tokio::time::Instant;

use crate::session::{SshSession, map_russh_error};

/// Capacity of the output queue handed to the consumer.
const OUTPUT_QUEUE: usize = 256;

/// Most output that may wait for a slow consumer. russh delivers channel data from the single
/// session task, so the pump must never block on the consumer (a stalled terminal tab would
/// freeze SFTP, keepalives and every other channel). While the queue is full the pump keeps
/// reading and coalesces `Data`; past this cap the channel is closed and the excess dropped.
/// A terminal that is not drained for that long is torn down rather than stalling the connection.
const MAX_PENDING_OUTPUT: usize = 16 * 1024 * 1024;

/// How long after an `Eof` the server may take to send `Close` before we close the channel.
const EOF_CLOSE_TIMEOUT: Duration = Duration::from_secs(5);

/// How long the server may take to answer the PTY and shell requests.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

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
    stop: Arc<Notify>,
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
        // Do not wait for the server's reply: the shell is finished from our side.
        self.stop.notify_one();
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
    let wait = async {
        loop {
            match reader.wait().await {
                Some(ChannelMsg::Success) => return Ok(()),
                Some(ChannelMsg::Failure) => {
                    return Err(Error::new(
                        ErrorKind::Protocol,
                        "Server refused the PTY/shell request",
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
    };
    tokio::time::timeout(REQUEST_TIMEOUT, wait)
        .await
        .unwrap_or_else(|_| Err(Error::new(ErrorKind::Network, "Shell request timed out")))
}

/// Output waiting for the consumer, in order. Consecutive `Data` are coalesced.
#[derive(Default)]
struct Pending {
    items: VecDeque<ShellOutput>,
    bytes: usize,
}

impl Pending {
    /// Queues `data`; returns false (dropping it) when the cap would be exceeded.
    fn push_data(&mut self, data: Vec<u8>) -> bool {
        if self.bytes + data.len() > MAX_PENDING_OUTPUT {
            return false;
        }
        self.bytes += data.len();
        match self.items.back_mut() {
            Some(ShellOutput::Data(last)) => last.extend_from_slice(&data),
            _ => self.items.push_back(ShellOutput::Data(data)),
        }
        true
    }

    fn pop(&mut self) -> Option<ShellOutput> {
        let item = self.items.pop_front()?;
        if let ShellOutput::Data(d) = &item {
            self.bytes -= d.len();
        }
        Some(item)
    }
}

/// Forwards channel messages to the consumer without ever blocking on it: it selects between
/// reading from russh and delivering queued output.
async fn pump(
    reader: ChannelReadHalf,
    writer: Arc<ChannelWriteHalf<Msg>>,
    tx: mpsc::Sender<ShellOutput>,
    closed: Arc<AtomicBool>,
    stop: Arc<Notify>,
) {
    let mut reader = Some(reader);
    let mut pending = Pending::default();
    let mut eof_at: Option<Instant> = None;
    loop {
        let reading = reader.is_some();
        tokio::select! {
            permit = tx.reserve(), if !pending.items.is_empty() => {
                let Ok(permit) = permit else { break };
                let item = pending.pop().expect("queue is not empty");
                let last = item == ShellOutput::Closed;
                permit.send(item);
                if last {
                    break;
                }
            }
            msg = async { reader.as_mut().expect("reading").wait().await }, if reading => {
                let mut end = false;
                match msg {
                    Some(ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. }) => {
                        if !pending.push_data(data.to_vec()) {
                            let _ = writer.close().await;
                            end = true;
                        }
                    }
                    Some(ChannelMsg::ExitStatus { exit_status }) => {
                        pending.items.push_back(ShellOutput::Exit(Some(exit_status)));
                    }
                    Some(ChannelMsg::ExitSignal { .. }) => {
                        pending.items.push_back(ShellOutput::Exit(None));
                    }
                    Some(ChannelMsg::Eof) => {
                        eof_at.get_or_insert_with(|| Instant::now() + EOF_CLOSE_TIMEOUT);
                    }
                    Some(ChannelMsg::Close) | None => end = true,
                    Some(_) => {}
                }
                if end {
                    finish(&mut reader, &mut pending, &closed);
                }
            }
            () = async { tokio::time::sleep_until(eof_at.expect("armed")).await }, if reading && eof_at.is_some() => {
                let _ = writer.close().await;
                finish(&mut reader, &mut pending, &closed);
            }
            () = stop.notified(), if reading => {
                finish(&mut reader, &mut pending, &closed);
            }
        }
    }
    closed.store(true, Ordering::SeqCst);
}

/// Marks the shell closed, queues `Closed` after everything else and stops reading, so russh
/// is never left waiting on this channel.
fn finish(reader: &mut Option<ChannelReadHalf>, pending: &mut Pending, closed: &AtomicBool) {
    closed.store(true, Ordering::SeqCst);
    *reader = None;
    pending.items.push_back(ShellOutput::Closed);
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
        let writer = Arc::new(writer);
        let stop = Arc::new(Notify::new());
        tokio::spawn(pump(
            reader,
            writer.clone(),
            tx,
            closed.clone(),
            stop.clone(),
        ));
        Ok(ShellChannel {
            writer,
            closed,
            stop,
            rx,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_coalesces_data_and_keeps_order() {
        let mut p = Pending::default();
        assert!(p.push_data(b"ab".to_vec()));
        assert!(p.push_data(b"cd".to_vec()));
        p.items.push_back(ShellOutput::Exit(Some(1)));
        assert!(p.push_data(b"ef".to_vec()));
        assert_eq!(p.bytes, 6);
        assert_eq!(p.pop(), Some(ShellOutput::Data(b"abcd".to_vec())));
        assert_eq!(p.pop(), Some(ShellOutput::Exit(Some(1))));
        assert_eq!(p.pop(), Some(ShellOutput::Data(b"ef".to_vec())));
        assert_eq!(p.bytes, 0);
        assert_eq!(p.pop(), None);
    }

    #[test]
    fn pending_refuses_data_past_the_cap() {
        let mut p = Pending::default();
        assert!(p.push_data(vec![0; MAX_PENDING_OUTPUT]));
        assert!(!p.push_data(vec![0]));
        assert_eq!(p.bytes, MAX_PENDING_OUTPUT);
    }
}
