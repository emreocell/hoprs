//! FTP connection configuration. `Debug` never prints the password.

use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use hoprs_core::ReconnectPolicy;
use secrecy::SecretString;

/// How the control and data connections are protected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FtpSecurity {
    /// No TLS at all.
    Plain,
    /// `AUTH TLS` after the greeting (the usual FTPS).
    ExplicitTls,
    /// TLS from the first byte (usually port 990).
    ImplicitTls,
}

impl FtpSecurity {
    /// Label shown to the user.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Plain => "ftp (unencrypted)",
            Self::ExplicitTls => "ftps (explicit)",
            Self::ImplicitTls => "ftps (implicit)",
        }
    }
}

/// Who opens the data connections.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferMode {
    /// The client connects to the address the server announces (`PASV`).
    Passive,
    /// The server connects back to the client (`PORT`/`EPRT`).
    Active,
}

/// Everything needed to open an FTP connection.
#[derive(Clone)]
pub struct FtpConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    /// `None` asks [`crate::FtpPrompter::password`] before connecting.
    pub password: Option<SecretString>,
    pub security: FtpSecurity,
    pub mode: TransferMode,
    /// In passive mode, connect data channels to the control connection's host instead of
    /// a private or unreachable address announced by the server (NAT).
    pub pasv_nat_workaround: bool,
    /// Maximum number of extra connections used for transfers (1..=8).
    pub transfer_connections: usize,
    /// A `NOOP` is sent on the control connection after this much idle time.
    pub keepalive_interval: Duration,
    /// Bounds TCP connect, greeting, login and `FEAT`; time spent in prompts does not count.
    pub connect_timeout: Duration,
    pub reconnect: ReconnectPolicy,
    /// SHA-256 fingerprint of a certificate the user accepted (FTPS).
    pub pinned_fingerprint: Option<String>,
    /// Extra trusted CA certificates (PEM files), besides the system roots (FTPS).
    pub extra_roots: Vec<PathBuf>,
}

impl FtpConfig {
    /// Passive mode with the NAT workaround, 4 transfer connections, 30 s keepalive,
    /// 15 s connect timeout, default reconnect policy, no pin, no extra roots.
    pub fn new(
        host: impl Into<String>,
        port: u16,
        user: impl Into<String>,
        security: FtpSecurity,
    ) -> Self {
        Self {
            host: host.into(),
            port,
            user: user.into(),
            password: None,
            security,
            mode: TransferMode::Passive,
            pasv_nat_workaround: true,
            transfer_connections: 4,
            keepalive_interval: Duration::from_secs(30),
            connect_timeout: Duration::from_secs(15),
            reconnect: ReconnectPolicy::default(),
            pinned_fingerprint: None,
            extra_roots: Vec::new(),
        }
    }
}

impl fmt::Debug for FtpConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FtpConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("user", &self.user)
            .field("password", &self.password.as_ref().map(|_| "<redacted>"))
            .field("security", &self.security)
            .field("mode", &self.mode)
            .field("pasv_nat_workaround", &self.pasv_nat_workaround)
            .field("transfer_connections", &self.transfer_connections)
            .field("keepalive_interval", &self.keepalive_interval)
            .field("connect_timeout", &self.connect_timeout)
            .field("reconnect", &self.reconnect)
            .field("pinned_fingerprint", &self.pinned_fingerprint)
            .field("extra_roots", &self.extra_roots)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels() {
        assert_eq!(FtpSecurity::Plain.label(), "ftp (unencrypted)");
        assert_eq!(FtpSecurity::ExplicitTls.label(), "ftps (explicit)");
        assert_eq!(FtpSecurity::ImplicitTls.label(), "ftps (implicit)");
    }

    #[test]
    fn defaults() {
        let c = FtpConfig::new("h", 21, "u", FtpSecurity::ExplicitTls);
        assert_eq!(c.mode, TransferMode::Passive);
        assert!(c.pasv_nat_workaround);
        assert_eq!(c.transfer_connections, 4);
        assert_eq!(c.keepalive_interval, Duration::from_secs(30));
        assert_eq!(c.connect_timeout, Duration::from_secs(15));
        assert_eq!(c.reconnect, ReconnectPolicy::default());
        assert!(c.password.is_none() && c.pinned_fingerprint.is_none());
        assert!(c.extra_roots.is_empty());
    }

    #[test]
    fn debug_redacts_password() {
        let mut c = FtpConfig::new("h", 21, "u", FtpSecurity::Plain);
        c.password = Some(SecretString::from("hunter2"));
        let s = format!("{c:?}");
        assert!(!s.contains("hunter2") && s.contains("<redacted>"), "{s}");
    }
}
