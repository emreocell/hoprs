//! Connection configuration. `Debug` never prints secrets.

use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use hoprs_core::ReconnectPolicy;
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
    /// `None` uses `SSH_AUTH_SOCK`; on Windows, when that is unset too, the OpenSSH agent
    /// pipe `\\.\pipe\openssh-ssh-agent`.
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
    /// Channels this connection may have open at once (SFTP and terminals); see
    /// [`crate::channels`]. Defaults to OpenSSH's `MaxSessions`, 10. Values below 1 count
    /// as 1.
    pub max_channels: usize,
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
            max_channels: crate::channels::DEFAULT_MAX_CHANNELS,
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
            .field("max_channels", &self.max_channels)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(c.max_channels, 10);
    }
}
