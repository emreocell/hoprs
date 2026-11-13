//! A saved [`Site`] (plus its secrets) as the connection config of its protocol.
//!
//! Every `Site` field with a counterpart is mapped. The others are not connection settings:
//! `id` keys the session and the transfer engine, `name` is UI only, `ssh.open_as` picks the
//! tabs, `remote_dir`/`local_dir` are where the file panes start, and `has_password` /
//! `has_key_passphrase` only decide which secrets [`SecretBatch`](super::vault::SecretBatch)
//! reads. Config fields with no `Site` counterpart keep their defaults: `known_hosts`
//! (`~/.ssh/known_hosts`, overridable in [`SessionManager`](super::sessions::SessionManager)),
//! keepalive, timeouts, reconnect policy, the agent socket (`SSH_AUTH_SOCK`), FTP
//! `extra_roots` (none) and `transfer_connections` (set by the session manager).

use std::fmt;

use hoprs_core::{FtpMode, Protocol, Site};
use hoprs_remote::{FtpConfig, FtpSecurity, TransferMode};
use hoprs_ssh::{KeySource, SshConfig};
use secrecy::SecretString;

/// The secrets of one site, read from the keyring. `Debug` never prints them.
#[derive(Clone, Default)]
pub struct SiteSecrets {
    pub password: Option<SecretString>,
    pub key_passphrase: Option<SecretString>,
    /// The keyring could not be reached for this site (or an earlier site of the same
    /// batch): the missing secrets fall back to the normal prompts.
    pub keyring_unavailable: bool,
}

impl fmt::Debug for SiteSecrets {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let redact = |s: &Option<SecretString>| s.as_ref().map(|_| "<redacted>");
        f.debug_struct("SiteSecrets")
            .field("password", &redact(&self.password))
            .field("key_passphrase", &redact(&self.key_passphrase))
            .field("keyring_unavailable", &self.keyring_unavailable)
            .finish()
    }
}

/// The SSH config of `site`. Authentication follows the spec's order: the agent (if the
/// site allows it), the site's key file (or the default `~/.ssh/id_*` files when it names
/// none), the stored password, then keyboard-interactive.
pub fn ssh_config(site: &Site, secrets: &SiteSecrets) -> SshConfig {
    let mut config = SshConfig::new(&site.host, site.port, &site.user);
    config.auth.use_agent = site.ssh.use_agent;
    config.auth.key = match &site.ssh.key_file {
        Some(path) => KeySource::File {
            path: path.clone(),
            passphrase: secrets.key_passphrase.clone(),
        },
        None => KeySource::DefaultFiles,
    };
    config.auth.password = secrets.password.clone();
    config.auth.allow_keyboard_interactive = true;
    config
}

/// The FTP config of `site`: security from the protocol, passive or active mode, the PASV
/// NAT workaround, the pinned certificate and the stored password.
///
/// Only meant for the FTP protocols. An SSH site (a caller bug) gets explicit TLS, so a
/// mistake can never send the password in clear text.
pub fn ftp_config(site: &Site, secrets: &SiteSecrets) -> FtpConfig {
    let security = match site.protocol {
        Protocol::Ftp => FtpSecurity::Plain,
        Protocol::FtpsImplicit => FtpSecurity::ImplicitTls,
        Protocol::FtpsExplicit | Protocol::Ssh => FtpSecurity::ExplicitTls,
    };
    let mut config = FtpConfig::new(&site.host, site.port, &site.user, security);
    config.password = secrets.password.clone();
    config.mode = match site.ftp.mode {
        FtpMode::Passive => TransferMode::Passive,
        FtpMode::Active => TransferMode::Active,
    };
    config.pasv_nat_workaround = site.ftp.nat_workaround;
    config.pinned_fingerprint = site.ftp.pinned_fingerprint.clone();
    config
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use secrecy::ExposeSecret;

    use super::*;

    fn secrets(password: Option<&str>, passphrase: Option<&str>) -> SiteSecrets {
        SiteSecrets {
            password: password.map(SecretString::from),
            key_passphrase: passphrase.map(SecretString::from),
            keyring_unavailable: false,
        }
    }

    fn exposed(s: &Option<SecretString>) -> Option<&str> {
        s.as_ref().map(|s| s.expose_secret())
    }

    #[test]
    fn mapping_ssh_and_ftp() {
        // SSH, defaults: agent on, default key files, keyboard-interactive allowed.
        let mut site = Site::new("Prod", "example.com", "deploy", Protocol::Ssh);
        site.port = 2200;
        let c = ssh_config(&site, &SiteSecrets::default());
        assert_eq!(
            (c.host.as_str(), c.port, c.user.as_str()),
            ("example.com", 2200, "deploy")
        );
        assert!(c.auth.use_agent);
        assert!(matches!(c.auth.key, KeySource::DefaultFiles));
        assert!(c.auth.password.is_none());
        assert!(c.auth.allow_keyboard_interactive);
        assert!(c.known_hosts.ends_with(".ssh/known_hosts"));

        // SSH with a key file, its passphrase, a password and no agent.
        site.ssh.use_agent = false;
        site.ssh.key_file = Some(PathBuf::from("/keys/id_ed25519"));
        let c = ssh_config(&site, &secrets(Some("pw"), Some("pp")));
        assert!(!c.auth.use_agent);
        match &c.auth.key {
            KeySource::File { path, passphrase } => {
                assert_eq!(path, &PathBuf::from("/keys/id_ed25519"));
                assert_eq!(exposed(passphrase), Some("pp"));
            }
            other => panic!("expected a key file, got {other:?}"),
        }
        assert_eq!(exposed(&c.auth.password), Some("pw"));
        // Debug never shows them.
        let debug = format!("{c:?}");
        assert!(
            !debug.contains("pw\"") && !debug.contains("pp\""),
            "{debug}"
        );

        // FTP protocols: security, port, user, password.
        let cases = [
            (Protocol::Ftp, FtpSecurity::Plain),
            (Protocol::FtpsExplicit, FtpSecurity::ExplicitTls),
            (Protocol::FtpsImplicit, FtpSecurity::ImplicitTls),
        ];
        for (protocol, security) in cases {
            let site = Site::new("F", "ftp.example.com", "frank", protocol);
            let c = ftp_config(&site, &secrets(Some("fp"), None));
            assert_eq!(c.security, security, "{protocol:?}");
            assert_eq!(c.port, protocol.default_port());
            assert_eq!(
                (c.host.as_str(), c.user.as_str()),
                ("ftp.example.com", "frank")
            );
            assert_eq!(exposed(&c.password), Some("fp"));
            // Defaults: passive with the NAT workaround, no pin.
            assert_eq!(c.mode, TransferMode::Passive);
            assert!(c.pasv_nat_workaround);
            assert!(c.pinned_fingerprint.is_none());
        }

        // Active mode, no NAT workaround, a pin, no password (asked at connect).
        let mut site = Site::new("F", "h", "u", Protocol::FtpsExplicit);
        site.port = 2121;
        site.ftp.mode = FtpMode::Active;
        site.ftp.nat_workaround = false;
        site.ftp.pinned_fingerprint = Some("SHA256:AB".into());
        let c = ftp_config(&site, &SiteSecrets::default());
        assert_eq!(c.port, 2121);
        assert_eq!(c.mode, TransferMode::Active);
        assert!(!c.pasv_nat_workaround);
        assert_eq!(c.pinned_fingerprint.as_deref(), Some("SHA256:AB"));
        assert!(c.password.is_none());

        // A misrouted SSH site never maps to plain FTP.
        let site = Site::new("S", "h", "u", Protocol::Ssh);
        assert_eq!(
            ftp_config(&site, &SiteSecrets::default()).security,
            FtpSecurity::ExplicitTls
        );
    }

    #[test]
    fn site_secrets_debug_is_redacted() {
        let s = format!("{:?}", secrets(Some("hunter2"), Some("fixture-pp")));
        assert!(!s.contains("hunter2") && !s.contains("fixture-pp"), "{s}");
        assert!(s.contains("<redacted>"));
    }
}
