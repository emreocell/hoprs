//! SSH session: connection, host key verification, authentication, keepalive and reconnection.
//!
//! [`SshSession`] is a cheap handle to a supervisor task that owns the current russh
//! connection. The supervisor publishes [`SessionState`], reconnects with backoff when the
//! connection is lost, and ends when the last handle is dropped.
//!
//! Disconnect detection (russh 0.65): keepalive is russh's own (`client::Config::
//! keepalive_interval` / `keepalive_max`); after `keepalive_max` unanswered keepalives the
//! session loop ends with `KeepaliveTimeout`. Every end of an established session (server
//! `DISCONNECT`, EOF, I/O error, keepalive timeout) calls `Handler::disconnected`, except
//! when shutting down the write half fails first (e.g. after a reset), which returns early.
//! So the handler also signals when it is dropped, which happens whenever the session task
//! ends; no polling of `Handle::is_closed` is needed.

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, PoisonError, RwLock};
use std::time::Duration;

use hoprs_core::{Error, ErrorKind, Result};
use russh::client;
use russh::keys::PublicKeyOrCertificate;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot, watch};

use crate::config::SshConfig;
use crate::known_hosts::{self, HostKeyStatus};
use crate::prompt::Prompter;
use crate::watchdog::{self, WatchedPrompter};

/// Observable connection state.
#[derive(Clone, Debug, PartialEq)]
pub enum SessionState {
    /// First connection in progress. [`SshSession::connect`] only returns once connected,
    /// so a session handle never reports it.
    Connecting,
    Connected,
    /// Connection lost; `attempt` (from 1) is the reconnection attempt in progress.
    Reconnecting {
        attempt: u32,
    },
    /// Gave up, failed for good (host key, authentication) or closed by the user.
    Disconnected(Error),
}

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
    /// The live connection; `Some` only while `Connected`.
    handle: RwLock<Option<Arc<client::Handle<ClientHandler>>>>,
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
    }

    fn current_handle(&self) -> Option<Arc<client::Handle<ClientHandler>>> {
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
    /// for (not configured) is prompted for again.
    pub async fn connect(config: SshConfig, prompter: Arc<dyn Prompter>) -> Result<SshSession> {
        let conn = establish(&config, &prompter).await?;
        let (state, _) = watch::channel(SessionState::Connected);
        let shared = Arc::new(Shared {
            handle: RwLock::new(Some(Arc::new(conn.handle))),
            generation: AtomicU64::new(1),
            state,
        });
        let (commands, rx) = mpsc::unbounded_channel();
        let supervisor = Supervisor {
            cfg: config.clone(),
            prompter,
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
    pub fn generation(&self) -> u64 {
        self.inner.shared.generation.load(Ordering::SeqCst)
    }

    /// Opens a `session` channel on the current connection. Never waits for a reconnection:
    /// while `Reconnecting` or `Disconnected` it fails at once with `Network`, "Not connected".
    pub async fn open_channel(&self) -> Result<russh::Channel<client::Msg>> {
        let Some(handle) = self.inner.shared.current_handle() else {
            return Err(Error::new(ErrorKind::Network, "Not connected"));
        };
        handle.channel_open_session().await.map_err(map_russh_error)
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
                establish(cfg, prompter).await
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
                    *shared
                        .handle
                        .write()
                        .unwrap_or_else(PoisonError::into_inner) = Some(Arc::new(conn.handle));
                    shared.generation.fetch_add(1, Ordering::SeqCst);
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
        self.shared.set_state(user_disconnected());
        if let Some(handle) = self.shared.take_handle() {
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
///
/// Bounded by `cfg.connect_timeout` of network time; time spent in prompts does not count
/// (see [`crate::watchdog`]). On timeout the attempt future is dropped. If russh had already
/// spawned its session task (key exchange started), that task may linger until the server
/// closes the socket or russh's own timeouts end it; it holds no handle we expose, and its
/// prompts are cancelled (answered "no") once this function returns or is dropped.
pub(crate) async fn establish(cfg: &SshConfig, prompter: &Arc<dyn Prompter>) -> Result<Connection> {
    let watched = Arc::new(WatchedPrompter::new(prompter.clone()));
    let _cancel = CancelPrompts(watched.clone());
    tokio::select! {
        result = attempt(cfg, watched.clone()) => result,
        _ = watchdog::expired(&watched, cfg.connect_timeout) => {
            Err(Error::new(ErrorKind::Network, "Connection timed out"))
        }
    }
}

async fn attempt(cfg: &SshConfig, prompter: Arc<WatchedPrompter>) -> Result<Connection> {
    let stream = TcpStream::connect((cfg.host.as_str(), cfg.port))
        .await
        .map_err(|e| {
            Error::new(ErrorKind::Network, "Connection failed").with_detail(e.to_string())
        })?;
    // Nagle only delays interactive traffic; a failure here is harmless.
    let _ = stream.set_nodelay(true);

    let russh_config = Arc::new(client::Config {
        keepalive_interval: Some(cfg.keepalive_interval),
        keepalive_max: cfg.keepalive_max as usize,
        ..Default::default()
    });
    let (lost_tx, lost) = oneshot::channel();
    let handler = ClientHandler {
        host: cfg.host.clone(),
        port: cfg.port,
        known_hosts: cfg.known_hosts.clone(),
        prompter: prompter.clone(),
        lost: Some(lost_tx),
    };
    let mut handle = client::connect_stream(russh_config, stream, handler)
        .await
        .map_err(|e| e.0)?;
    crate::auth::authenticate(&mut handle, cfg, prompter.as_ref()).await?;
    Ok(Connection { handle, lost })
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

impl client::Handler for ClientHandler {
    type Error = HandlerError;

    async fn disconnected(
        &mut self,
        reason: client::DisconnectReason<Self::Error>,
    ) -> std::result::Result<(), Self::Error> {
        let (error, result) = match reason {
            client::DisconnectReason::ReceivedDisconnect(info) => (
                Error::new(ErrorKind::Network, "Connection lost").with_detail(format!(
                    "server disconnected ({:?}): {}",
                    info.reason_code, info.message
                )),
                Ok(()),
            ),
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
