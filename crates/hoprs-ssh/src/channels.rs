//! Channel budget: how many channels one SSH connection may have open at once.
//!
//! OpenSSH refuses more than `MaxSessions` (10 by default) open session channels per
//! connection; past it, a terminal would fail with an obscure open failure, or the transfers
//! would starve it. So [`SshSession`](crate::SshSession) counts its channels against
//! [`SshConfig::max_channels`](crate::SshConfig::max_channels) (default
//! [`DEFAULT_MAX_CHANNELS`] = 10).
//!
//! **What counts.** Every channel opened through
//! [`SshSession::open_channel`](crate::SshSession::open_channel) holds one slot for as long
//! as its [`ChannelSlot`] lives: the SFTP metadata channel, each SFTP stream channel and each
//! PTY ([`SshSession::open_shell`](crate::SshSession::open_shell)). A shell gives its slot
//! back as soon as it ends (closed by us, by the server, or by a lost connection), without
//! waiting for the server's reply to our close.
//!
//! **Reservation for terminals (ruling R2).** SFTP opens at most
//! [`sftp_stream_channels`]`(max_channels, configured)` =
//! `min(configured, max_channels - 1 metadata - 2 reserved)` stream channels (at least 1),
//! so with the default 10 the transfers use at most 1 + 7 slots and at least
//! [`PTY_RESERVE`] = 2 stay free for terminals. Below 4 the reservation shrinks: the one
//! stream channel SFTP needs comes first.
//!
//! **Over the limit.** Opening a channel past `max_channels` fails at once, without asking
//! the server, with `ErrorKind::Protocol`, "Too many channels on this connection (limit N)".
//! A channel open the server refuses (its own `MaxSessions` is lower than ours) gives the
//! same error, with the server's answer as the detail. The refused channel holds no slot.
//!
//! **Reconnects.** The count belongs to one connection (one generation of the session).
//! A new connection starts from zero; slots of the dead connection are not counted
//! anymore, and dropping them later does not free slots of the new one.

use std::sync::{Arc, Mutex, PoisonError};

use hoprs_core::{Error, ErrorKind, Result};

/// OpenSSH's default `MaxSessions`.
pub const DEFAULT_MAX_CHANNELS: usize = 10;

/// Slots the SFTP stream channels leave free for terminals.
pub const PTY_RESERVE: usize = 2;

/// The SFTP metadata channel.
const METADATA_CHANNELS: usize = 1;

const LIMIT_MESSAGE: &str = "Too many channels on this connection";

/// The error for a channel past `limit` (ours or the server's).
pub(crate) fn too_many_channels(limit: usize) -> Error {
    Error::new(
        ErrorKind::Protocol,
        format!("{LIMIT_MESSAGE} (limit {limit})"),
    )
}

/// Whether `error` is the "Too many channels" error of a channel open.
pub fn is_channel_limit(error: &Error) -> bool {
    error.kind == ErrorKind::Protocol && error.message.starts_with(LIMIT_MESSAGE)
}

/// How many SFTP stream channels fit under `max_channels` beside the metadata channel and
/// the terminal reserve: `min(configured, max_channels - 1 - 2)`, at least 1.
pub fn sftp_stream_channels(max_channels: usize, configured: usize) -> usize {
    configured
        .min(max_channels.saturating_sub(METADATA_CHANNELS + PTY_RESERVE))
        .max(1)
}

/// The count of open channels of the current connection.
#[derive(Debug)]
pub(crate) struct ChannelBudget {
    limit: usize,
    count: Mutex<Count>,
}

#[derive(Debug, Default)]
struct Count {
    /// The connection the count belongs to; 0 before the first one.
    generation: u64,
    open: usize,
}

impl ChannelBudget {
    /// A budget of `limit` channels (at least 1).
    pub(crate) fn new(limit: usize) -> Arc<Self> {
        Arc::new(Self {
            limit: limit.max(1),
            count: Mutex::new(Count::default()),
        })
    }

    pub(crate) fn limit(&self) -> usize {
        self.limit
    }

    fn count(&self) -> std::sync::MutexGuard<'_, Count> {
        self.count.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Channels counted for the current connection.
    pub(crate) fn open(&self) -> usize {
        self.count().open
    }

    /// A new connection (`generation`) is up: it starts with no channel.
    pub(crate) fn reset(&self, generation: u64) {
        *self.count() = Count {
            generation,
            open: 0,
        };
    }

    /// Takes a slot for a channel of connection `generation`. Fails with "Too many
    /// channels" when all are taken, and with `Network`, "Not connected" when `generation`
    /// is no longer the current connection.
    pub(crate) fn acquire(self: &Arc<Self>, generation: u64) -> Result<ChannelSlot> {
        let mut count = self.count();
        if count.generation != generation {
            return Err(Error::new(ErrorKind::Network, "Not connected"));
        }
        if count.open >= self.limit {
            return Err(too_many_channels(self.limit));
        }
        count.open += 1;
        Ok(ChannelSlot {
            budget: self.clone(),
            generation,
        })
    }
}

/// One channel's place in the budget; dropping it frees the place. See the module docs.
#[derive(Debug)]
pub struct ChannelSlot {
    budget: Arc<ChannelBudget>,
    generation: u64,
}

impl ChannelSlot {
    /// The session generation (connection) the channel was opened on.
    pub fn generation(&self) -> u64 {
        self.generation
    }
}

impl Drop for ChannelSlot {
    fn drop(&mut self) {
        let mut count = self.budget.count();
        // A slot of a dead connection was forgotten by `reset` already.
        if count.generation == self.generation {
            count.open = count.open.saturating_sub(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use hoprs_core::ErrorKind;

    use super::*;

    #[test]
    fn channel_budget_reserves_room_for_a_pty() {
        // Metadata + streams never take the last two slots (ruling R2).
        assert_eq!(sftp_stream_channels(10, 8), 7);
        assert_eq!(sftp_stream_channels(10, 4), 4);
        assert_eq!(sftp_stream_channels(4, 8), 1);
        assert_eq!(sftp_stream_channels(1, 8), 1);

        let budget = ChannelBudget::new(10);
        budget.reset(1);
        let streams = sftp_stream_channels(budget.limit(), 8);
        let mut held: Vec<ChannelSlot> = (0..1 + streams)
            .map(|_| budget.acquire(1).expect("metadata and stream channels"))
            .collect();
        held.push(budget.acquire(1).expect("first PTY"));
        held.push(budget.acquire(1).expect("second PTY"));
        assert_eq!(budget.open(), 10);

        let e = budget.acquire(1).unwrap_err();
        assert_eq!(e.kind, ErrorKind::Protocol);
        assert_eq!(e.message, "Too many channels on this connection (limit 10)");
        assert!(is_channel_limit(&e));
        assert_eq!(budget.open(), 10, "a refused slot is not counted");

        held.pop();
        assert_eq!(budget.open(), 9, "a closed channel frees its slot");
        let _pty = budget.acquire(1).expect("the freed slot");
    }

    #[test]
    fn channel_budget_resets_on_reconnect() {
        let budget = ChannelBudget::new(2);
        budget.reset(1);
        let old = [budget.acquire(1).unwrap(), budget.acquire(1).unwrap()];
        assert!(budget.acquire(1).is_err());
        assert_eq!(old[0].generation(), 1);

        // A new connection starts empty: the dead connection's channels no longer count.
        budget.reset(2);
        assert_eq!(budget.open(), 0);
        let fresh = budget.acquire(2).unwrap();
        // Dropping a channel of the dead connection does not free a slot of the new one.
        drop(old);
        assert_eq!(budget.open(), 1);
        // A slot asked for the old generation after the reset is refused.
        assert_eq!(budget.acquire(1).unwrap_err().kind, ErrorKind::Network);
        drop(fresh);
        assert_eq!(budget.open(), 0);
    }
}
