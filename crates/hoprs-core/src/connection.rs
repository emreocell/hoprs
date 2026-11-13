//! Protocol-neutral connection state and reconnection policy.

use std::time::Duration;

use crate::Error;

/// Observable connection state (SSH, FTP, ...).
#[derive(Clone, Debug, PartialEq)]
pub enum ConnectionState {
    /// First connection in progress: a handle that was started, not yet connected (also a
    /// retry of a first connection that failed).
    Connecting,
    Connected,
    /// Connection lost; `attempt` (from 1) is the reconnection attempt in progress.
    Reconnecting {
        attempt: u32,
    },
    /// A connection attempt (first or reconnection) is held by a prompt (host key,
    /// certificate, password, ...) until the user answers. Not connected; when the prompt
    /// closes the state goes back to the `Connecting` or `Reconnecting` it replaced.
    WaitingForUser,
    /// Gave up, failed for good (host key, authentication) or closed by the user.
    Disconnected(Error),
}

/// Exponential backoff for reconnection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconnectPolicy {
    pub max_attempts: u32,
    pub base_delay: Duration,
    pub max_delay: Duration,
}

impl Default for ReconnectPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            base_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(16),
        }
    }
}

impl ReconnectPolicy {
    /// Delay before each attempt: doubles from `base_delay`, capped at `max_delay`.
    pub fn delays(&self) -> Vec<Duration> {
        let mut delay = self.base_delay;
        (0..self.max_attempts)
            .map(|_| {
                let current = delay.min(self.max_delay);
                delay = delay.saturating_mul(2).min(self.max_delay);
                current
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_backoff() {
        let d: Vec<u64> = ReconnectPolicy::default()
            .delays()
            .iter()
            .map(|d| d.as_secs())
            .collect();
        assert_eq!(d, vec![1, 2, 4, 8, 16]);
    }

    #[test]
    fn backoff_saturates() {
        let p = ReconnectPolicy {
            max_attempts: 7,
            base_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(10),
        };
        assert_eq!(p.delays().last(), Some(&Duration::from_secs(10)));
        assert_eq!(p.delays().len(), 7);
    }
}
