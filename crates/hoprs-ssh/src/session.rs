//! SSH session: connection, host key verification and authentication.

use std::fmt;
use std::sync::Arc;

use hoprs_core::{Error, ErrorKind, Result};
use russh::client;
use russh::keys::PublicKeyOrCertificate;
use tokio::net::TcpStream;

use crate::config::SshConfig;
use crate::known_hosts::{self, HostKeyStatus};
use crate::prompt::Prompter;

/// A connected, authenticated SSH session. Cheap to clone.
#[derive(Clone)]
pub struct SshSession {
    inner: Arc<Inner>,
}

struct Inner {
    config: SshConfig,
    handle: client::Handle<ClientHandler>,
}

impl fmt::Debug for SshSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SshSession")
            .field("host", &self.host())
            .field("port", &self.port())
            .field("user", &self.user())
            .finish()
    }
}

impl SshSession {
    /// Connects, verifies the host key against `config.known_hosts` and authenticates.
    pub async fn connect(config: SshConfig, prompter: Arc<dyn Prompter>) -> Result<SshSession> {
        let handle = establish(&config, &prompter).await?;
        Ok(SshSession {
            inner: Arc::new(Inner { config, handle }),
        })
    }

    pub fn host(&self) -> &str {
        &self.inner.config.host
    }

    pub fn port(&self) -> u16 {
        self.inner.config.port
    }

    pub fn user(&self) -> &str {
        &self.inner.config.user
    }

    /// Closes the connection. Errors are ignored: the connection is gone either way.
    pub async fn disconnect(&self) {
        let _ = self
            .inner
            .handle
            .disconnect(russh::Disconnect::ByApplication, "", "en")
            .await;
    }
}

/// One full connection attempt: TCP, key exchange with host key check, authentication.
pub(crate) async fn establish(
    cfg: &SshConfig,
    prompter: &Arc<dyn Prompter>,
) -> Result<client::Handle<ClientHandler>> {
    let addr = (cfg.host.as_str(), cfg.port);
    let stream = match tokio::time::timeout(cfg.connect_timeout, TcpStream::connect(addr)).await {
        Err(_) => return Err(Error::new(ErrorKind::Network, "Connection timed out")),
        Ok(Err(e)) => {
            return Err(
                Error::new(ErrorKind::Network, "Connection failed").with_detail(e.to_string())
            );
        }
        Ok(Ok(stream)) => stream,
    };
    // Nagle only delays interactive traffic; a failure here is harmless.
    let _ = stream.set_nodelay(true);

    let russh_config = Arc::new(client::Config {
        keepalive_interval: Some(cfg.keepalive_interval),
        keepalive_max: cfg.keepalive_max as usize,
        ..Default::default()
    });
    let handler = ClientHandler {
        host: cfg.host.clone(),
        port: cfg.port,
        known_hosts: cfg.known_hosts.clone(),
        prompter: prompter.clone(),
    };
    let mut handle = client::connect_stream(russh_config, stream, handler)
        .await
        .map_err(|e| e.0)?;
    crate::auth::authenticate(&mut handle, cfg, prompter.as_ref()).await?;
    Ok(handle)
}

/// russh callbacks: host key verification.
pub(crate) struct ClientHandler {
    host: String,
    port: u16,
    known_hosts: std::path::PathBuf,
    prompter: Arc<dyn Prompter>,
}

/// Handler error: always carries the final `hoprs_core::Error`.
#[derive(Debug)]
pub(crate) struct HandlerError(pub(crate) Error);

impl From<russh::Error> for HandlerError {
    fn from(e: russh::Error) -> Self {
        HandlerError(map_russh_error(e))
    }
}

/// Maps a transport-level russh error. Never contains secrets: russh errors carry none.
pub(crate) fn map_russh_error(e: russh::Error) -> Error {
    use russh::Error as E;
    let kind = match &e {
        E::IO(_)
        | E::Disconnect
        | E::HUP
        | E::ConnectionTimeout
        | E::KeepaliveTimeout
        | E::InactivityTimeout
        | E::SendError
        | E::RecvError
        | E::Elapsed(_) => ErrorKind::Network,
        _ => ErrorKind::Protocol,
    };
    let message = match kind {
        ErrorKind::Network => "Connection lost",
        _ => "SSH protocol error",
    };
    Error::new(kind, message).with_detail(e.to_string())
}

impl client::Handler for ClientHandler {
    type Error = HandlerError;

    async fn check_server_key(
        &mut self,
        server_public_key: &PublicKeyOrCertificate,
    ) -> std::result::Result<bool, Self::Error> {
        let PublicKeyOrCertificate::PublicKey { key, .. } = server_public_key else {
            return Err(HandlerError(Error::new(
                ErrorKind::HostKey,
                "Host certificates are not supported",
            )));
        };
        let host = &self.host;
        let status =
            known_hosts::check(&self.known_hosts, host, self.port, key).map_err(HandlerError)?;
        match status {
            HostKeyStatus::Known => Ok(true),
            HostKeyStatus::Unknown => {
                let fingerprint = known_hosts::fingerprint(key);
                let accepted = self
                    .prompter
                    .confirm_host_key(host, self.port, key.algorithm().as_str(), &fingerprint)
                    .await;
                if !accepted {
                    return Err(HandlerError(Error::new(
                        ErrorKind::HostKey,
                        "Cancelled by user",
                    )));
                }
                known_hosts::learn(&self.known_hosts, host, self.port, key)
                    .map_err(HandlerError)?;
                Ok(true)
            }
            HostKeyStatus::Changed { line } => Err(HandlerError(
                Error::new(
                    ErrorKind::HostKey,
                    format!("Host key for {host} has changed"),
                )
                .with_detail(format!("known_hosts line {line}; remove it to continue")),
            )),
            HostKeyStatus::Revoked { line } => Err(HandlerError(
                Error::new(
                    ErrorKind::HostKey,
                    format!("Host key for {host} has been revoked"),
                )
                .with_detail(format!("known_hosts line {line}")),
            )),
        }
    }
}
