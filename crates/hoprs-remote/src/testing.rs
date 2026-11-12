//! Helpers for the FTP Docker integration tests (`docker/compose.yml`). Test-only.

use std::path::{Path, PathBuf};
use std::time::Duration;

use hoprs_core::Result;
use hoprs_ssh::testing::{docker, docker_with_env};
use secrecy::SecretString;

use crate::ftp::session::{Control, open_control, quote};
use crate::ftp::{FtpConfig, FtpPrompter, FtpSecurity};

/// Host port of the `ftp` service (plain FTP and explicit FTPS).
pub const FTP_PORT: u16 = 2121;
/// Host port of the `ftps-implicit` service.
pub const FTPS_IMPLICIT_PORT: u16 = 2990;

/// Config for `frank` on the test containers: the `ftps-implicit` service for
/// [`FtpSecurity::ImplicitTls`], the `ftp` service otherwise. No extra roots, no pin.
/// Keepalive every 2 s, well under vsftpd's `idle_session_timeout=8`; command timeout 10 s.
pub fn ftp_config(security: FtpSecurity) -> FtpConfig {
    let port = match security {
        FtpSecurity::ImplicitTls => FTPS_IMPLICIT_PORT,
        FtpSecurity::Plain | FtpSecurity::ExplicitTls => FTP_PORT,
    };
    let mut cfg = FtpConfig::new("127.0.0.1", port, "frank", security);
    cfg.password = Some(SecretString::from("frank-pass"));
    cfg.keepalive_interval = Duration::from_secs(2);
    cfg.command_timeout = Duration::from_secs(10);
    cfg
}

/// Path of a file under `docker/fixtures/ftps`.
pub fn ftps_fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docker/fixtures/ftps")
        .join(name)
}

/// `SHA256:`-prefixed fingerprint of `server.pem` (from `server.fingerprint`).
pub fn server_fingerprint() -> String {
    let hex = std::fs::read_to_string(ftps_fixture("server.fingerprint"))
        .expect("read server.fingerprint");
    format!("SHA256:{}", hex.trim())
}

/// While alive, the `ftps-implicit` service serves the unrelated self-signed `other.pem`.
/// Dropping it recreates the service with `server.pem` again.
pub struct OtherCertificate(());

impl OtherCertificate {
    /// Recreates `ftps-implicit` with `FTPS_CERT=other` and waits until it is healthy.
    pub fn serve() -> Self {
        // The guard exists first, so a failed or partial recreate is still undone.
        let guard = Self(());
        recreate_implicit("other");
        guard
    }
}

impl Drop for OtherCertificate {
    fn drop(&mut self) {
        recreate_implicit("server");
    }
}

fn recreate_implicit(cert: &str) {
    docker_with_env(
        &[("FTPS_CERT", cert)],
        &["up", "-d", "--force-recreate", "--wait", "ftps-implicit"],
    );
}

/// Kills every vsftpd session process in both FTP services, dropping client connections.
/// The listener (the oldest vsftpd process) survives. Having no session is not an error.
pub fn kill_ftp_sessions() {
    const SCRIPT: &str = "l=$(pgrep -o -x vsftpd); for p in $(pgrep -x vsftpd); do \
                          [ \"$p\" = \"$l\" ] || kill -9 \"$p\" 2>/dev/null; done; true";
    for service in ["ftp", "ftps-implicit"] {
        docker(&["exec", "-T", service, "sh", "-c", SCRIPT]);
    }
}

/// A control connection opened by `open_control`, for tests that check the login itself.
pub struct TestControl(Control);

/// Runs the crate's `open_control`; an accepted certificate is pinned in `cfg`.
pub async fn open_test_control(
    cfg: &mut FtpConfig,
    prompter: &dyn FtpPrompter,
) -> Result<TestControl> {
    open_control(cfg, prompter).await.map(TestControl)
}

impl TestControl {
    pub fn has_mfmt(&self) -> bool {
        self.0.has_mfmt
    }

    pub fn has_mlsd(&self) -> bool {
        self.0.has_mlsd
    }

    pub fn has_utf8(&self) -> bool {
        self.0.has_utf8
    }

    /// Names in the current directory (`NLST`), over a data connection.
    pub async fn nlst(&mut self) -> Result<Vec<String>> {
        self.0
            .stream
            .nlst(None)
            .await
            .map_err(|e| crate::ftp::errors::map_ftp_error(e, "NLST"))
    }

    /// Sends a raw command and returns the reply's code and text, whatever the code.
    pub async fn quote(&mut self, command: &str) -> Result<(u32, String)> {
        quote(&mut self.0.stream, command, "raw command").await
    }
}

pub use crate::ftp::session::{FtpSession, PooledConn};

/// Sends a raw command on the session's control connection; returns the reply's code and
/// text, whatever the code.
pub async fn session_quote(session: &FtpSession, command: &str) -> Result<(u32, String)> {
    let command = command.to_owned();
    session
        .with_control(move |c| {
            Box::pin(async move { quote(&mut c.stream, &command, "raw command").await })
        })
        .await
}

/// Sends a raw command on a pooled transfer connection.
pub async fn pooled_quote(conn: &mut PooledConn, command: &str) -> Result<(u32, String)> {
    quote(conn.stream(), command, "raw command").await
}

/// Sends a raw command on the session's control connection; a reply of 400 or more is an
/// error (mapped like every FTP reply), otherwise the reply's text is returned.
pub async fn session_command(session: &FtpSession, command: &str) -> Result<String> {
    let command = command.to_owned();
    session
        .with_control(move |c| {
            Box::pin(async move {
                let (code, text) = quote(&mut c.stream, &command, "raw command").await?;
                if code >= 400 {
                    return Err(crate::ftp::errors::map_reply(code, &text, &command));
                }
                Ok(text)
            })
        })
        .await
}

/// Runs a command on the session's control connection that fails with `error`, as if the
/// command itself had returned it.
pub async fn session_fail(session: &FtpSession, error: hoprs_core::Error) -> Result<()> {
    session
        .with_control(move |_| Box::pin(async move { Err(error) }))
        .await
}

/// Sends `PWD` on the control connection, then calls [`FtpSession::disconnect`] *before the
/// command returns*: the command's reply and the disconnection race.
pub async fn session_disconnect_inside_command(session: &FtpSession) -> Result<String> {
    let handle = session.clone();
    session
        .with_control(move |c| {
            Box::pin(async move {
                let (_, text) = quote(&mut c.stream, "PWD", "raw command").await?;
                handle.disconnect().await;
                Ok(text)
            })
        })
        .await
}
