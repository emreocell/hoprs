//! Protocol-neutral view of a live connection, used by the transfer engine's site watcher.

use hoprs_core::{ConnectionState, Error};
use hoprs_ssh::SshSession;
use tokio::sync::watch;

/// A connection that reconnects by itself and reports its state.
pub trait Connection: Send + Sync {
    /// Observable state; the sender side closes when the connection is gone for good.
    fn state(&self) -> watch::Receiver<ConnectionState>;
    /// Increases every time a new underlying connection replaces the previous one.
    fn generation(&self) -> u64;
    /// Why the last connection ended: set before the state leaves `Connected` (and before
    /// each further `Reconnecting` attempt, to the previous attempt's failure). `None`
    /// until a connection ends. A `Disconnected` state carries its own reason.
    fn last_error(&self) -> Option<Error> {
        None
    }
}

impl Connection for SshSession {
    fn state(&self) -> watch::Receiver<ConnectionState> {
        SshSession::state(self)
    }

    fn generation(&self) -> u64 {
        SshSession::generation(self)
    }

    fn last_error(&self) -> Option<Error> {
        SshSession::last_error(self)
    }
}

impl Connection for crate::ftp::session::FtpSession {
    fn state(&self) -> watch::Receiver<ConnectionState> {
        crate::ftp::session::FtpSession::state(self)
    }

    fn generation(&self) -> u64 {
        crate::ftp::session::FtpSession::generation(self)
    }

    fn last_error(&self) -> Option<Error> {
        crate::ftp::session::FtpSession::last_error(self)
    }
}
