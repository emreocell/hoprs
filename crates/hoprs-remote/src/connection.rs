//! Protocol-neutral view of a live connection, used by the transfer engine's site watcher.

use hoprs_core::ConnectionState;
use hoprs_ssh::SshSession;
use tokio::sync::watch;

/// A connection that reconnects by itself and reports its state.
pub trait Connection: Send + Sync {
    /// Observable state; the sender side closes when the connection is gone for good.
    fn state(&self) -> watch::Receiver<ConnectionState>;
    /// Increases every time a new underlying connection replaces the previous one.
    fn generation(&self) -> u64;
}

impl Connection for SshSession {
    fn state(&self) -> watch::Receiver<ConnectionState> {
        SshSession::state(self)
    }

    fn generation(&self) -> u64 {
        SshSession::generation(self)
    }
}
