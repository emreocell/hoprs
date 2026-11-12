//! Connection configuration. `Debug` never prints secrets.

use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use secrecy::SecretString;

/// Where the private key comes from.
#[derive(Clone)]
pub enum KeySource {
    None,
    /// `~/.ssh/id_ed25519`, `id_ecdsa`, `id_rsa` (those that exist).
    DefaultFiles,
    File {
        path: PathBuf,
        passphrase: Option<SecretString>,
    },
}

impl fmt::Debug for KeySource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => f.write_str("None"),
            Self::DefaultFiles => f.write_str("DefaultFiles"),
            Self::File { path, passphrase } => f
                .debug_struct("File")
                .field("path", path)
                .field("passphrase", &redact(passphrase))
                .finish(),
        }
    }
}

fn redact(secret: &Option<SecretString>) -> Option<&'static str> {
    secret.as_ref().map(|_| "<redacted>")
}

/// Authentication options, tried in order: agent, key, password, keyboard-interactive.
#[derive(Clone)]
pub struct AuthConfig {
    pub use_agent: bool,
    /// `None` uses `SSH_AUTH_SOCK`.
    pub agent_socket: Option<PathBuf>,
    pub key: KeySource,
    pub password: Option<SecretString>,
    pub allow_keyboard_interactive: bool,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            use_agent: true,
            agent_socket: None,
            key: KeySource::DefaultFiles,
            password: None,
            allow_keyboard_interactive: true,
        }
    }
}

impl fmt::Debug for AuthConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuthConfig")
            .field("use_agent", &self.use_agent)
            .field("agent_socket", &self.agent_socket)
            .field("key", &self.key)
            .field("password", &redact(&self.password))
            .field(
                "allow_keyboard_interactive",
                &self.allow_keyboard_interactive,
            )
            .finish()
    }
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

/// Everything needed to open an SSH connection.
#[derive(Clone)]
pub struct SshConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: AuthConfig,
    pub known_hosts: PathBuf,
    /// A keepalive is sent after this much time without data from the server.
    pub keepalive_interval: Duration,
    /// Unanswered keepalives tolerated. The connection is declared lost once one more goes
    /// unanswered, so a hung connection is detected within
    /// `keepalive_interval x (keepalive_max + 1)` of the last data received (120 s with the
    /// defaults 30 s x 3). Values below 1 count as 1: keepalive never waits forever.
    pub keepalive_max: u32,
    pub reconnect: ReconnectPolicy,
    pub connect_timeout: Duration,
}

impl SshConfig {
    pub fn new(host: impl Into<String>, port: u16, user: impl Into<String>) -> Self {
        let known_hosts = std::env::home_dir()
            .map(|h| h.join(".ssh").join("known_hosts"))
            .unwrap_or_else(|| PathBuf::from(".ssh/known_hosts"));
        Self {
            host: host.into(),
            port,
            user: user.into(),
            auth: AuthConfig::default(),
            known_hosts,
            keepalive_interval: Duration::from_secs(30),
            keepalive_max: 3,
            reconnect: ReconnectPolicy::default(),
            connect_timeout: Duration::from_secs(15),
        }
    }
}

impl fmt::Debug for SshConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SshConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("user", &self.user)
            .field("auth", &self.auth)
            .field("known_hosts", &self.known_hosts)
            .field("keepalive_interval", &self.keepalive_interval)
            .field("keepalive_max", &self.keepalive_max)
            .field("reconnect", &self.reconnect)
            .field("connect_timeout", &self.connect_timeout)
            .finish()
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

    #[test]
    fn debug_redacts_secrets() {
        let mut c = SshConfig::new("h", 22, "u");
        c.auth.password = Some(SecretString::from("hunter2"));
        c.auth.key = KeySource::File {
            path: "/k".into(),
            passphrase: Some(SecretString::from("fixture-pp")),
        };
        let s = format!("{c:?}");
        assert!(!s.contains("hunter2") && !s.contains("fixture-pp") && s.contains("<redacted>"));
    }

    #[test]
    fn defaults() {
        let c = SshConfig::new("h", 22, "u");
        assert_eq!(c.keepalive_interval, Duration::from_secs(30));
        assert!(c.known_hosts.ends_with(".ssh/known_hosts"));
    }
}
