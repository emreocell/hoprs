//! SSH session: connection, host key verification, authentication, keepalive and reconnection.
//!
//! [`SshSession`] is a cheap handle to a supervisor task that owns the current russh
//! connection. The supervisor publishes [`SessionState`], reconnects with backoff when the
//! connection is lost, and ends when the last handle is dropped.
//!
//! Disconnect detection (russh 0.65): keepalive is russh's own (`client::Config::
//! keepalive_interval` / `keepalive_max`): russh gives up when more than `keepalive_max`
//! keepalives go unanswered, i.e. `keepalive_interval x (keepalive_max + 1)` after the last
//! data received, ending the session loop with `KeepaliveTimeout`. russh reads
//! `keepalive_max = 0` as "never give up", so `russh_config` clamps it to at least 1.
//! Every end of an established session (server `DISCONNECT`, EOF, I/O error, keepalive
//! timeout) calls `Handler::disconnected`, except when shutting down the write half fails
//! first (e.g. after a reset), which returns early. So the handler also signals when it is
//! dropped, which happens whenever the session task ends; no polling of
//! `Handle::is_closed` is needed.

use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, PoisonError, RwLock};
use std::time::Duration;

use hoprs_core::{Error, ErrorKind, Result};
use russh::client;
use russh::keys::{Algorithm, PublicKey, PublicKeyOrCertificate};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot, watch};

use crate::config::SshConfig;
use crate::known_hosts::{self, HostKeyStatus};
use crate::prompt::Prompter;
use crate::watchdog::{self, WatchedPrompter};

/// Observable connection state (the protocol-neutral [`hoprs_core::ConnectionState`]).
pub type SessionState = hoprs_core::ConnectionState;

/// An SSH session that stays connected: keepalive, disconnect detection and automatic
/// reconnection. Cheap to clone; the connection closes when the last clone is dropped.
#[derive(Clone)]
pub struct SshSession {
    inner: Arc<Inner>,
}

struct Inner {
    config: SshConfig,
    shared: Arc<Shared>,
    /// Dropping the last sender stops the supervisor.
    commands: mpsc::UnboundedSender<Command>,
}

/// State shared between the handles and the supervisor.
struct Shared {
    /// The live connection and its generation; `Some` only while `Connected`.
    handle: RwLock<Option<(Arc<client::Handle<ClientHandler>>, u64)>>,
    generation: AtomicU64,
    state: watch::Sender<SessionState>,
}

impl Shared {
    fn set_state(&self, new: SessionState) {
        self.state.send_if_modified(|s| {
            let changed = *s != new;
            *s = new;
            changed
        });
    }

    fn take_handle(&self) -> Option<Arc<client::Handle<ClientHandler>>> {
        self.handle
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
            .map(|(handle, _)| handle)
    }

    /// Publishes a new connection under the next generation.
    fn install(&self, handle: client::Handle<ClientHandler>) {
        let mut slot = self.handle.write().unwrap_or_else(PoisonError::into_inner);
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        *slot = Some((Arc::new(handle), generation));
    }

    fn current_handle(&self) -> Option<(Arc<client::Handle<ClientHandler>>, u64)> {
        self.handle
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// Requests from the handles; each is acknowledged once the state reflects it.
enum Command {
    Disconnect(oneshot::Sender<()>),
    ReconnectNow(oneshot::Sender<()>),
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
    ///
    /// Reconnection attempts reuse `config` and `prompter`: a password that was prompted
    /// for (not configured) is prompted for again. They must present the host key verified
    /// by this first connection: any other key, of any algorithm, ends the session with
    /// `HostKey`, "Host key for {host} has changed", without prompting.
    pub async fn connect(config: SshConfig, prompter: Arc<dyn Prompter>) -> Result<SshSession> {
        let conn = establish(&config, &prompter, None).await?;
        let (state, _) = watch::channel(SessionState::Connected);
        let shared = Arc::new(Shared {
            handle: RwLock::new(Some((Arc::new(conn.handle), 1))),
            generation: AtomicU64::new(1),
            state,
        });
        let (commands, rx) = mpsc::unbounded_channel();
        let supervisor = Supervisor {
            cfg: config.clone(),
            prompter,
            host_key: conn.host_key,
            shared: shared.clone(),
            commands: rx,
        };
        tokio::spawn(supervisor.run(conn.lost));
        Ok(SshSession {
            inner: Arc::new(Inner {
                config,
                shared,
                commands,
            }),
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

    /// Watches the connection state.
    pub fn state(&self) -> watch::Receiver<SessionState> {
        self.inner.shared.state.subscribe()
    }

    /// Starts at 1 and goes up by one after every successful reconnection. Channels opened
    /// under an older generation are dead.
    ///
    /// Reading this next to [`open_channel`](Self::open_channel) can race with a reconnect;
    /// callers that track which connection a channel belongs to should use
    /// [`open_channel_tagged`](Self::open_channel_tagged).
    pub fn generation(&self) -> u64 {
        self.inner.shared.generation.load(Ordering::SeqCst)
    }

    /// Like [`open_channel`](Self::open_channel), also returning the generation of the
    /// connection the channel was opened on (read together with the handle).
    pub async fn open_channel_tagged(&self) -> Result<(russh::Channel<client::Msg>, u64)> {
        let Some((handle, generation)) = self.inner.shared.current_handle() else {
            return Err(Error::new(ErrorKind::Network, "Not connected"));
        };
        let channel = handle
            .channel_open_session()
            .await
            .map_err(map_russh_error)?;
        Ok((channel, generation))
    }

    /// Opens a `session` channel on the current connection. Never waits for a reconnection:
    /// while `Reconnecting` or `Disconnected` it fails at once with `Network`, "Not connected".
    pub async fn open_channel(&self) -> Result<russh::Channel<client::Msg>> {
        self.open_channel_tagged().await.map(|(channel, _)| channel)
    }

    /// Starts a new round of reconnection attempts, the first one without delay, when the
    /// state is `Disconnected`. Does nothing otherwise. Returns once the state shows it.
    pub async fn reconnect_now(&self) {
        self.request(Command::ReconnectNow).await;
    }

    /// Closes the connection and stops reconnecting: the state becomes `Disconnected` with
    /// `Network`, "Disconnected by user" (or keeps its error if already disconnected).
    pub async fn disconnect(&self) {
        self.request(Command::Disconnect).await;
    }

    async fn request(&self, command: impl FnOnce(oneshot::Sender<()>) -> Command) {
        let (ack, done) = oneshot::channel();
        if self.inner.commands.send(command(ack)).is_ok() {
            let _ = done.await;
        }
    }
}

/// What the supervisor does next.
enum Phase {
    Connected(oneshot::Receiver<Error>),
    Reconnecting { last: Error, immediate: bool },
    Disconnected,
    Stop,
}

/// Owns the connection: watches it, reconnects, and serves handle commands.
struct Supervisor {
    cfg: SshConfig,
    prompter: Arc<dyn Prompter>,
    /// The server key verified on the first connection; every reconnection must present it.
    host_key: PublicKey,
    shared: Arc<Shared>,
    commands: mpsc::UnboundedReceiver<Command>,
}

impl Supervisor {
    async fn run(mut self, lost: oneshot::Receiver<Error>) {
        let mut phase = Phase::Connected(lost);
        loop {
            phase = match phase {
                Phase::Connected(lost) => self.connected(lost).await,
                Phase::Reconnecting { last, immediate } => self.reconnect(last, immediate).await,
                Phase::Disconnected => self.disconnected().await,
                Phase::Stop => break,
            };
        }
        if let Some(handle) = self.shared.take_handle() {
            let _ = handle
                .disconnect(russh::Disconnect::ByApplication, "", "en")
                .await;
        }
    }

    async fn connected(&mut self, mut lost: oneshot::Receiver<Error>) -> Phase {
        loop {
            tokio::select! {
                reason = &mut lost => {
                    self.shared.take_handle();
                    // The sender is dropped without a reason when the session task ends
                    // without calling `disconnected`.
                    let last = reason
                        .unwrap_or_else(|_| Error::new(ErrorKind::Network, "Connection lost"));
                    return Phase::Reconnecting { last, immediate: false };
                }
                command = self.commands.recv() => match command {
                    None => return Phase::Stop,
                    Some(Command::Disconnect(ack)) => {
                        self.disconnect_by_user().await;
                        let _ = ack.send(());
                        return Phase::Disconnected;
                    }
                    Some(Command::ReconnectNow(ack)) => {
                        let _ = ack.send(());
                    }
                },
            }
        }
    }

    /// One round of attempts per `cfg.reconnect`. `HostKey` and `Auth` errors end it at once.
    async fn reconnect(&mut self, mut last: Error, immediate: bool) -> Phase {
        let Self {
            cfg,
            prompter,
            host_key,
            shared,
            commands,
        } = self;
        for (index, delay) in cfg.reconnect.delays().into_iter().enumerate() {
            let attempt = u32::try_from(index + 1).unwrap_or(u32::MAX);
            shared.set_state(SessionState::Reconnecting { attempt });
            let delay = if immediate && index == 0 {
                Duration::ZERO
            } else {
                delay
            };
            let try_once = async {
                tokio::time::sleep(delay).await;
                establish(cfg, prompter, Some(&*host_key)).await
            };
            tokio::pin!(try_once);
            let result = loop {
                tokio::select! {
                    result = &mut try_once => break result,
                    command = commands.recv() => match command {
                        None => return Phase::Stop,
                        Some(Command::Disconnect(ack)) => {
                            // Dropping `try_once` abandons the attempt and its prompts.
                            shared.set_state(user_disconnected());
                            let _ = ack.send(());
                            return Phase::Disconnected;
                        }
                        Some(Command::ReconnectNow(ack)) => {
                            let _ = ack.send(());
                        }
                    },
                }
            };
            match result {
                Ok(conn) => {
                    shared.install(conn.handle);
                    shared.set_state(SessionState::Connected);
                    return Phase::Connected(conn.lost);
                }
                Err(e) if matches!(e.kind, ErrorKind::HostKey | ErrorKind::Auth) => {
                    shared.set_state(SessionState::Disconnected(e));
                    return Phase::Disconnected;
                }
                Err(e) => last = e,
            }
        }
        shared.set_state(SessionState::Disconnected(last));
        Phase::Disconnected
    }

    async fn disconnected(&mut self) -> Phase {
        loop {
            match self.commands.recv().await {
                None => return Phase::Stop,
                Some(Command::Disconnect(ack)) => {
                    let _ = ack.send(());
                }
                Some(Command::ReconnectNow(ack)) => {
                    let last = match &*self.shared.state.borrow() {
                        SessionState::Disconnected(e) => e.clone(),
                        _ => user_disconnected_error(),
                    };
                    self.shared
                        .set_state(SessionState::Reconnecting { attempt: 1 });
                    let _ = ack.send(());
                    return Phase::Reconnecting {
                        last,
                        immediate: true,
                    };
                }
            }
        }
    }

    async fn disconnect_by_user(&self) {
        let handle = self.shared.take_handle();
        self.shared.set_state(user_disconnected());
        if let Some(handle) = handle {
            let _ = handle
                .disconnect(russh::Disconnect::ByApplication, "", "en")
                .await;
        }
    }
}

fn user_disconnected_error() -> Error {
    Error::new(ErrorKind::Network, "Disconnected by user")
}

fn user_disconnected() -> SessionState {
    SessionState::Disconnected(user_disconnected_error())
}

/// An established, authenticated connection.
pub(crate) struct Connection {
    pub(crate) handle: client::Handle<ClientHandler>,
    /// The server key that was verified.
    pub(crate) host_key: PublicKey,
    /// Resolves (with the reason, or closed without one) when the session task ends.
    pub(crate) lost: oneshot::Receiver<Error>,
}

/// Cancels the attempt's prompts when the attempt ends, however it ends.
struct CancelPrompts(Arc<WatchedPrompter>);

impl Drop for CancelPrompts {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

/// One full connection attempt: TCP, key exchange with host key check, authentication.
/// With `pinned`, the server must present exactly that key (see [`check_pinned`]).
///
/// Bounded by `cfg.connect_timeout` of network time; time spent in prompts does not count
/// (see [`crate::watchdog`]). On timeout the attempt future is dropped. If russh had already
/// spawned its session task (key exchange started), that task may linger until the server
/// closes the socket or russh's own timeouts end it; it holds no handle we expose, and its
/// prompts are cancelled (answered "no") once this function returns or is dropped.
pub(crate) async fn establish(
    cfg: &SshConfig,
    prompter: &Arc<dyn Prompter>,
    pinned: Option<&PublicKey>,
) -> Result<Connection> {
    let watched = Arc::new(WatchedPrompter::new(prompter.clone()));
    let _cancel = CancelPrompts(watched.clone());
    tokio::select! {
        result = attempt(cfg, watched.clone(), pinned) => result,
        _ = watchdog::expired(&watched, cfg.connect_timeout) => {
            Err(Error::new(ErrorKind::Network, "Connection timed out"))
        }
    }
}

async fn attempt(
    cfg: &SshConfig,
    prompter: Arc<WatchedPrompter>,
    pinned: Option<&PublicKey>,
) -> Result<Connection> {
    let stream = TcpStream::connect((cfg.host.as_str(), cfg.port))
        .await
        .map_err(|e| {
            Error::new(ErrorKind::Network, "Connection failed").with_detail(e.to_string())
        })?;
    // Nagle only delays interactive traffic; a failure here is harmless.
    let _ = stream.set_nodelay(true);

    let mut known = Vec::from_iter(pinned.map(PublicKey::algorithm));
    known.extend(known_hosts::known_algorithms(
        &cfg.known_hosts,
        &cfg.host,
        cfg.port,
    ));
    let russh_config = Arc::new(russh_config(cfg, &known));
    let (lost_tx, lost) = oneshot::channel();
    let (verified_tx, mut verified) = oneshot::channel();
    let server_disconnected = Arc::new(AtomicBool::new(false));
    let handler = ClientHandler {
        host: cfg.host.clone(),
        port: cfg.port,
        known_hosts: cfg.known_hosts.clone(),
        prompter: prompter.clone(),
        pinned: pinned.cloned(),
        verified: Some(verified_tx),
        server_disconnected: server_disconnected.clone(),
        lost: Some(lost_tx),
    };
    let mut handle = client::connect_stream(russh_config, stream, handler)
        .await
        .map_err(|e| e.0)?;
    // russh only completes the key exchange after `check_server_key` accepted the key.
    let host_key = verified.try_recv().map_err(|_| {
        Error::new(ErrorKind::Protocol, "SSH protocol error")
            .with_detail("the server key was never verified")
    })?;
    if let Err(e) = crate::auth::authenticate(&mut handle, cfg, prompter.as_ref()).await {
        if e.kind == ErrorKind::Network && handle.is_closed() {
            // `lost` resolves once the handler has seen the end of the session.
            let _ = tokio::time::timeout(Duration::from_secs(1), lost).await;
            if server_disconnected.load(Ordering::SeqCst) {
                return Err(closed_during_auth());
            }
        }
        return Err(e);
    }
    Ok(Connection {
        handle,
        host_key,
        lost,
    })
}

/// The server sent `DISCONNECT` while we authenticated: most likely `MaxAuthTries` was
/// reached, which retrying cannot fix.
fn closed_during_auth() -> Error {
    Error::new(ErrorKind::Auth, "Authentication failed")
        .with_detail("server closed the connection during authentication (MaxAuthTries?)")
}

/// russh client settings for `cfg`. `keepalive_max` is clamped to at least 1: russh treats
/// 0 as unlimited, which would never detect a hung connection. Host key algorithms in
/// `known` (already recorded for the host) are negotiated first, so a server cannot steer
/// the exchange to an algorithm we hold no key for while it still offers a known one.
fn russh_config(cfg: &SshConfig, known: &[Algorithm]) -> client::Config {
    client::Config {
        keepalive_interval: Some(cfg.keepalive_interval),
        keepalive_max: cfg.keepalive_max.max(1) as usize,
        preferred: russh::Preferred {
            key: preferred_keys(known).into(),
            ..russh::Preferred::DEFAULT
        },
        ..Default::default()
    }
}

/// Host key algorithms to negotiate: russh's defaults, with those of `known` (keys already
/// recorded for the host) moved first, keeping russh's order. An RSA key covers every RSA
/// signature algorithm.
fn preferred_keys(known: &[Algorithm]) -> Vec<Algorithm> {
    let rsa = |a: &Algorithm| matches!(a, Algorithm::Rsa { .. });
    let is_known = |a: &Algorithm| known.iter().any(|k| k == a || (rsa(k) && rsa(a)));
    let default = russh::Preferred::DEFAULT.key;
    let (mut first, rest): (Vec<_>, Vec<_>) = default.iter().cloned().partition(is_known);
    first.extend(rest);
    first
}

/// Refuses a server key other than `pinned` (the key verified on the session's first
/// connection), whatever its algorithm.
fn check_pinned(pinned: Option<&PublicKey>, host: &str, key: &PublicKey) -> Result<()> {
    match pinned {
        Some(pinned) if pinned.key_data() != key.key_data() => Err(Error::new(
            ErrorKind::HostKey,
            format!("Host key for {host} has changed"),
        )
        .with_detail(format!(
            "the server presented a {} key ({}) instead of the key verified when this \
             session connected",
            key.algorithm().as_str(),
            known_hosts::fingerprint(key)
        ))),
        _ => Ok(()),
    }
}

/// russh callbacks: host key verification and disconnect notification.
///
/// Lives inside russh's session task: `lost` fires with the reason from `disconnected`, or
/// closes when the task ends and drops the handler.
pub(crate) struct ClientHandler {
    host: String,
    port: u16,
    known_hosts: std::path::PathBuf,
    prompter: Arc<WatchedPrompter>,
    /// On a reconnection, the only key the server may present.
    pinned: Option<PublicKey>,
    /// Receives the server key once it is accepted.
    verified: Option<oneshot::Sender<PublicKey>>,
    /// Set when the server ended the session with `DISCONNECT`.
    server_disconnected: Arc<AtomicBool>,
    lost: Option<oneshot::Sender<Error>>,
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

impl ClientHandler {
    fn accepted(&mut self, key: &PublicKey) {
        if let Some(verified) = self.verified.take() {
            let _ = verified.send(key.clone());
        }
    }
}

impl client::Handler for ClientHandler {
    type Error = HandlerError;

    async fn disconnected(
        &mut self,
        reason: client::DisconnectReason<Self::Error>,
    ) -> std::result::Result<(), Self::Error> {
        let (error, result) = match reason {
            client::DisconnectReason::ReceivedDisconnect(info) => {
                self.server_disconnected.store(true, Ordering::SeqCst);
                (
                    Error::new(ErrorKind::Network, "Connection lost").with_detail(format!(
                        "server disconnected ({:?}): {}",
                        info.reason_code, info.message
                    )),
                    Ok(()),
                )
            }
            client::DisconnectReason::Error(HandlerError(e)) => (e.clone(), Err(HandlerError(e))),
        };
        if let Some(lost) = self.lost.take() {
            let _ = lost.send(error);
        }
        result
    }

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
        check_pinned(self.pinned.as_ref(), host, key).map_err(HandlerError)?;
        let status =
            known_hosts::check(&self.known_hosts, host, self.port, key).map_err(HandlerError)?;
        match status {
            HostKeyStatus::Known => {
                self.accepted(key);
                Ok(true)
            }
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
                self.accepted(key);
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

#[cfg(test)]
mod tests {
    use super::*;
    use russh::keys::{EcdsaCurve, HashAlg, PrivateKey};

    fn key(algorithm: Algorithm) -> PublicKey {
        PrivateKey::random(&mut rand::rng(), algorithm)
            .unwrap()
            .public_key()
            .clone()
    }

    #[test]
    fn preferred_keys_puts_known_algorithms_first() {
        let p256 = Algorithm::Ecdsa {
            curve: EcdsaCurve::NistP256,
        };
        let default = russh::Preferred::DEFAULT.key.to_vec();
        assert_eq!(preferred_keys(&[]), default);
        assert_eq!(preferred_keys(&[Algorithm::Ed25519]), default);

        let got = preferred_keys(&[Algorithm::Rsa { hash: None }, p256.clone()]);
        assert_eq!(
            got[..4],
            [
                p256,
                Algorithm::Rsa {
                    hash: Some(HashAlg::Sha512)
                },
                Algorithm::Rsa {
                    hash: Some(HashAlg::Sha256)
                },
                Algorithm::Rsa { hash: None },
            ]
        );
        assert_eq!(got[4], Algorithm::Ed25519);
        assert_eq!(got.len(), default.len());
    }

    #[test]
    fn pinned_key_refuses_any_other_key() {
        let ed = key(Algorithm::Ed25519);
        let other_ed = key(Algorithm::Ed25519);
        let ecdsa = key(Algorithm::Ecdsa {
            curve: EcdsaCurve::NistP256,
        });
        check_pinned(None, "h", &ecdsa).unwrap();
        check_pinned(Some(&ed), "h", &ed).unwrap();
        for other in [&other_ed, &ecdsa] {
            let e = check_pinned(Some(&ed), "h", other).unwrap_err();
            assert_eq!(e.kind, ErrorKind::HostKey);
            assert_eq!(e.message, "Host key for h has changed");
        }
    }

    #[test]
    fn keepalive_max_is_at_least_one() {
        let mut cfg = SshConfig::new("h", 22, "u");
        cfg.keepalive_max = 0;
        assert_eq!(russh_config(&cfg, &[]).keepalive_max, 1);
        cfg.keepalive_max = 3;
        assert_eq!(russh_config(&cfg, &[]).keepalive_max, 3);
        assert_eq!(
            russh_config(&cfg, &[]).keepalive_interval,
            Some(cfg.keepalive_interval)
        );
    }
}
