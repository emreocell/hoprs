//! SSH session: connection, host key verification, authentication, keepalive and reconnection.
//!
//! [`SshSession`] is a cheap handle to a supervisor task that owns the current russh
//! connection. The supervisor makes the first connection, publishes [`SessionState`]
//! (`WaitingForUser` while an attempt waits on a prompt), reconnects with backoff when the
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
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::Duration;

use hoprs_core::{Error, ErrorKind, Result};
use russh::client;
use russh::keys::{Algorithm, PublicKey, PublicKeyOrCertificate};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot, watch};

use crate::auth::{AuthMethod, Remembered};
use crate::channels::{self, ChannelBudget, ChannelSlot};
use crate::config::SshConfig;
use crate::known_hosts::{self, HostKeyStatus};
use crate::prompt::Prompter;
use crate::watchdog::{self, WaitingHook, WatchedPrompter};

/// Observable connection state (the protocol-neutral [`hoprs_core::ConnectionState`]).
pub type SessionState = hoprs_core::ConnectionState;

/// An SSH session that stays connected: keepalive, disconnect detection and automatic
/// reconnection. Cheap to clone; the connection closes when the last clone is dropped.
#[derive(Clone)]
pub struct SshSession {
    inner: Arc<Inner>,
}

/// A channel opened by [`SshSession::open_channel`]. It holds its place in the channel
/// budget until dropped (or until [`into_parts`](Self::into_parts) hands the place over).
#[derive(Debug)]
pub struct CountedChannel {
    channel: russh::Channel<client::Msg>,
    slot: ChannelSlot,
}

impl CountedChannel {
    /// The session generation (connection) the channel was opened on.
    pub fn generation(&self) -> u64 {
        self.slot.generation()
    }

    /// The channel and its budget slot; the slot must live as long as the channel is used.
    pub fn into_parts(self) -> (russh::Channel<client::Msg>, ChannelSlot) {
        (self.channel, self.slot)
    }
}

impl std::ops::Deref for CountedChannel {
    type Target = russh::Channel<client::Msg>;

    fn deref(&self) -> &Self::Target {
        &self.channel
    }
}

impl std::ops::DerefMut for CountedChannel {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.channel
    }
}

struct Inner {
    config: SshConfig,
    shared: Arc<Shared>,
    /// Dropping the last sender stops the supervisor.
    commands: mpsc::UnboundedSender<Command>,
}

/// State shared between the handles and the supervisor.
struct Shared {
    /// The live connection and its generation; `Some` only while `Connected`. The
    /// generation is 0 until the first connection.
    handle: RwLock<Option<(Arc<client::Handle<ClientHandler>>, u64)>>,
    generation: AtomicU64,
    state: watch::Sender<SessionState>,
    /// How the latest connection authenticated; `None` until the first one.
    auth: RwLock<Option<AuthMethod>>,
    /// Open channels of the current connection (see [`crate::channels`]).
    budget: Arc<ChannelBudget>,
    /// Why the last connection (or the last reconnection attempt) failed; see
    /// [`SshSession::last_error`].
    last_error: Mutex<Option<Error>>,
}

impl Shared {
    fn set_state(&self, new: SessionState) {
        self.state.send_if_modified(|s| {
            let changed = *s != new;
            *s = new;
            changed
        });
    }

    /// `Reconnecting { attempt }`, `last` being why the connection (or the previous
    /// attempt) failed: recorded first, so whoever sees the state can read it.
    fn reconnecting(&self, attempt: u32, last: &Error) {
        *self
            .last_error
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(last.clone());
        self.set_state(SessionState::Reconnecting { attempt });
    }

    fn take_handle(&self) -> Option<Arc<client::Handle<ClientHandler>>> {
        self.handle
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
            .map(|(handle, _)| handle)
    }

    /// Publishes a new connection under the next generation.
    fn install(&self, handle: client::Handle<ClientHandler>, auth: AuthMethod) {
        *self.auth.write().unwrap_or_else(PoisonError::into_inner) = Some(auth);
        let mut slot = self.handle.write().unwrap_or_else(PoisonError::into_inner);
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        // Before the handle is published: no channel of this connection exists yet, and
        // those of the dead one stop counting.
        self.budget.reset(generation);
        *slot = Some((Arc::new(handle), generation));
    }
}

/// Publishes `WaitingForUser` while a connection attempt waits on a prompt, then puts back
/// the `Connecting` or `Reconnecting` state it replaced. Any other state (a disconnect by the
/// user meanwhile) is left alone.
fn waiting_hook(shared: Arc<Shared>) -> WaitingHook {
    let replaced: Mutex<Option<SessionState>> = Mutex::new(None);
    Arc::new(move |waiting| {
        let mut replaced = replaced.lock().unwrap_or_else(PoisonError::into_inner);
        if waiting {
            shared.state.send_if_modified(|s| {
                if matches!(
                    s,
                    SessionState::Connecting | SessionState::Reconnecting { .. }
                ) {
                    *replaced = Some(std::mem::replace(s, SessionState::WaitingForUser));
                    true
                } else {
                    false
                }
            });
        } else if let Some(previous) = replaced.take() {
            shared.state.send_if_modified(|s| {
                let restore = *s == SessionState::WaitingForUser;
                if restore {
                    *s = previous;
                }
                restore
            });
        }
    })
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
    /// Starts connecting in the background and returns at once, in `Connecting` (generation
    /// 0). Must be called from within a Tokio runtime.
    ///
    /// The first connection is a single attempt: it connects, verifies the host key against
    /// `config.known_hosts` and authenticates, then the state becomes `Connected`
    /// (generation 1). If it fails, the state becomes `Disconnected` with its error and
    /// [`reconnect_now`](Self::reconnect_now) tries again (in `Connecting`). While an attempt
    /// waits on a prompt the state is `WaitingForUser`.
    ///
    /// Reconnection attempts reuse `config` and `prompter`. A password or key passphrase
    /// typed at a prompt (not configured) that proved right is kept in memory for the
    /// session's reconnections and not asked again; one the server later refuses is
    /// forgotten, so the next attempt asks again. Reconnections must present the host key
    /// verified by the first connection: any other key, of any algorithm, ends the session
    /// with `HostKey`, "Host key for {host} has changed", without prompting.
    pub fn start(config: SshConfig, prompter: Arc<dyn Prompter>) -> SshSession {
        let (state, _) = watch::channel(SessionState::Connecting);
        let shared = Arc::new(Shared {
            handle: RwLock::new(None),
            generation: AtomicU64::new(0),
            state,
            auth: RwLock::new(None),
            budget: ChannelBudget::new(config.max_channels),
            last_error: Mutex::new(None),
        });
        let (commands, rx) = mpsc::unbounded_channel();
        let supervisor = Supervisor {
            cfg: config.clone(),
            prompter,
            host_key: None,
            remembered: Remembered::default(),
            shared: shared.clone(),
            commands: rx,
        };
        tokio::spawn(supervisor.run());
        SshSession {
            inner: Arc::new(Inner {
                config,
                shared,
                commands,
            }),
        }
    }

    /// [`start`](Self::start), then waits for the first connection: the session once
    /// `Connected`, or the error once `Disconnected` (the session is then dropped).
    pub async fn connect(config: SshConfig, prompter: Arc<dyn Prompter>) -> Result<SshSession> {
        let session = Self::start(config, prompter);
        let mut state = session.state();
        let outcome = state
            .wait_for(|s| matches!(s, SessionState::Connected | SessionState::Disconnected(_)))
            .await
            .map(|s| s.clone());
        match outcome {
            Ok(SessionState::Disconnected(e)) => Err(e),
            Ok(_) => Ok(session),
            // The supervisor lives as long as a handle does; this one is still here.
            Err(_) => Err(Error::coded(
                ErrorKind::Network,
                "net-connection-lost",
                "Connection lost",
            )),
        }
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

    /// 0 until the first connection, which makes it 1; goes up by one after every successful
    /// reconnection. Channels opened under an older generation are dead.
    ///
    /// Reading this next to [`open_channel`](Self::open_channel) can race with a reconnect;
    /// callers that track which connection a channel belongs to should use
    /// [`CountedChannel::generation`].
    pub fn generation(&self) -> u64 {
        self.inner.shared.generation.load(Ordering::SeqCst)
    }

    /// Why the last connection ended, set before the state becomes `Reconnecting` (then, at
    /// each further attempt, why the previous attempt failed). `None` until a connection
    /// ends. A `Disconnected` state carries its own reason.
    pub fn last_error(&self) -> Option<Error> {
        self.inner
            .shared
            .last_error
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// How the latest connection authenticated (the agent and its key type, a key file, a
    /// password...); `None` until the first connection. Kept while reconnecting.
    pub fn auth_method(&self) -> Option<AuthMethod> {
        self.inner
            .shared
            .auth
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Opens a `session` channel on the current connection, counted against the channel
    /// budget (see [`crate::channels`]) until the returned channel is dropped. The channel
    /// knows the generation of the connection it was opened on (read together with the
    /// handle).
    ///
    /// Never waits for a connection: unless `Connected` it fails at once with `Network`,
    /// "Not connected". Past `max_channels`, or when the server refuses the channel, it
    /// fails with `Protocol`, "Too many channels on this connection (limit N)".
    pub async fn open_channel(&self) -> Result<CountedChannel> {
        self.open_counted(false).await
    }

    /// [`open_channel`](Self::open_channel) for a PTY, which never takes the last slot of
    /// the budget (kept for the SFTP metadata channel; see [`crate::channels`]).
    pub(crate) async fn open_pty_channel(&self) -> Result<CountedChannel> {
        self.open_counted(true).await
    }

    async fn open_counted(&self, pty: bool) -> Result<CountedChannel> {
        let shared = &self.inner.shared;
        let (handle, slot) = {
            // Under the lock `install` resets the budget with: the slot always belongs to
            // the connection of `handle`.
            let current = shared.handle.read().unwrap_or_else(PoisonError::into_inner);
            let Some((handle, generation)) = current.as_ref() else {
                return Err(Error::coded(
                    ErrorKind::Network,
                    "net-not-connected",
                    "Not connected",
                ));
            };
            let slot = if pty {
                shared.budget.acquire_pty(*generation)?
            } else {
                shared.budget.acquire(*generation)?
            };
            (handle.clone(), slot)
        };
        let channel = handle.channel_open_session().await.map_err(|e| match e {
            russh::Error::ChannelOpenFailure(_) => {
                channels::too_many_channels(shared.budget.limit()).with_detail(e.to_string())
            }
            other => map_russh_error(other),
        })?;
        Ok(CountedChannel { channel, slot })
    }

    /// The channel budget of this connection: `SshConfig::max_channels`, at least 1.
    pub fn max_channels(&self) -> usize {
        self.inner.shared.budget.limit()
    }

    /// The SFTP stream channels to open for `configured`, leaving room for terminals; see
    /// [`channels::sftp_stream_channels`].
    pub fn sftp_stream_channels(&self, configured: usize) -> usize {
        channels::sftp_stream_channels(self.max_channels(), configured)
    }

    /// Channels counted for the current connection. For tests.
    #[doc(hidden)]
    pub fn open_channels(&self) -> usize {
        self.inner.shared.budget.open()
    }

    /// When the state is `Disconnected`, starts a new round of reconnection attempts, the
    /// first one without delay (`Reconnecting`), or, if the session never connected, a new
    /// first connection attempt (`Connecting`). Does nothing otherwise. Returns once the
    /// state shows it.
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
    Connecting,
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
    /// `None` until the first connection.
    host_key: Option<PublicKey>,
    /// Secrets typed at prompts, reused by the reconnections.
    remembered: Remembered,
    shared: Arc<Shared>,
    commands: mpsc::UnboundedReceiver<Command>,
}

impl Supervisor {
    async fn run(mut self) {
        let mut phase = Phase::Connecting;
        loop {
            phase = match phase {
                Phase::Connecting => self.connecting().await,
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

    /// One first-connection attempt. A failure is final until `reconnect_now`.
    async fn connecting(&mut self) -> Phase {
        let Self {
            cfg,
            prompter,
            host_key,
            remembered,
            shared,
            commands,
        } = self;
        shared.set_state(SessionState::Connecting);
        let try_once = establish(cfg, prompter, None, remembered, shared);
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
                *host_key = Some(conn.host_key);
                shared.install(conn.handle, conn.auth);
                shared.set_state(SessionState::Connected);
                Phase::Connected(conn.lost)
            }
            Err(e) => {
                shared.set_state(SessionState::Disconnected(e));
                Phase::Disconnected
            }
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
                        .unwrap_or_else(|_| Error::coded(ErrorKind::Network, "net-connection-lost", "Connection lost"));
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
            remembered,
            shared,
            commands,
        } = self;
        for (index, delay) in cfg.reconnect.delays().into_iter().enumerate() {
            let attempt = u32::try_from(index + 1).unwrap_or(u32::MAX);
            shared.reconnecting(attempt, &last);
            let delay = if immediate && index == 0 {
                Duration::ZERO
            } else {
                delay
            };
            let try_once = async {
                tokio::time::sleep(delay).await;
                establish(cfg, prompter, host_key.as_ref(), remembered, shared).await
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
                    shared.install(conn.handle, conn.auth);
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
                Some(Command::ReconnectNow(ack)) if self.host_key.is_none() => {
                    self.shared.set_state(SessionState::Connecting);
                    let _ = ack.send(());
                    return Phase::Connecting;
                }
                Some(Command::ReconnectNow(ack)) => {
                    let last = match &*self.shared.state.borrow() {
                        SessionState::Disconnected(e) => e.clone(),
                        _ => user_disconnected_error(),
                    };
                    self.shared.reconnecting(1, &last);
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
    Error::coded(
        ErrorKind::Network,
        "net-disconnected-by-user",
        "Disconnected by user",
    )
}

fn user_disconnected() -> SessionState {
    SessionState::Disconnected(user_disconnected_error())
}

/// An established, authenticated connection.
pub(crate) struct Connection {
    pub(crate) handle: client::Handle<ClientHandler>,
    /// The server key that was verified.
    pub(crate) host_key: PublicKey,
    /// How it authenticated.
    pub(crate) auth: AuthMethod,
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
/// With `pinned`, the server must present exactly that key (see [`check_pinned`]). While a
/// prompt is open the session state is `WaitingForUser` (see [`waiting_hook`]); secrets typed
/// at prompts go to `remembered`.
///
/// Bounded by `cfg.connect_timeout` of network time; time spent in prompts does not count
/// (see [`crate::watchdog`]). On timeout the attempt future is dropped. If russh had already
/// spawned its session task (key exchange started), that task may linger until the server
/// closes the socket or russh's own timeouts end it; it holds no handle we expose, and its
/// prompts are cancelled (answered "no") once this function returns or is dropped.
async fn establish(
    cfg: &SshConfig,
    prompter: &Arc<dyn Prompter>,
    pinned: Option<&PublicKey>,
    remembered: &Remembered,
    shared: &Arc<Shared>,
) -> Result<Connection> {
    let watched =
        Arc::new(WatchedPrompter::new(prompter.clone()).with_hook(waiting_hook(shared.clone())));
    let _cancel = CancelPrompts(watched.clone());
    tokio::select! {
        result = attempt(cfg, watched.clone(), pinned, remembered) => result,
        _ = watchdog::expired(&watched, cfg.connect_timeout) => {
            Err(Error::coded(ErrorKind::Network, "net-connection-timed-out", "Connection timed out"))
        }
    }
}

async fn attempt(
    cfg: &SshConfig,
    prompter: Arc<WatchedPrompter>,
    pinned: Option<&PublicKey>,
    remembered: &Remembered,
) -> Result<Connection> {
    let stream = connect_tcp(cfg).await?;

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
        Error::coded(
            ErrorKind::Protocol,
            "ssh-protocol-error",
            "SSH protocol error",
        )
        .with_detail("the server key was never verified")
    })?;
    let auth =
        match crate::auth::authenticate(&mut handle, cfg, prompter.as_ref(), remembered).await {
            Ok(auth) => auth,
            Err(e) => {
                if e.kind == ErrorKind::Network && handle.is_closed() {
                    // `lost` resolves once the handler has seen the end of the session.
                    let _ = tokio::time::timeout(Duration::from_secs(1), lost).await;
                    if server_disconnected.load(Ordering::SeqCst) {
                        return Err(closed_during_auth());
                    }
                }
                return Err(e);
            }
        };
    Ok(Connection {
        handle,
        host_key,
        auth,
        lost,
    })
}

/// The TCP connection to `cfg.host:cfg.port`, tuned for an interactive session.
async fn connect_tcp(cfg: &SshConfig) -> Result<TcpStream> {
    let stream = TcpStream::connect((cfg.host.as_str(), cfg.port))
        .await
        .map_err(|e| {
            Error::coded(
                ErrorKind::Network,
                "net-connection-failed",
                "Connection failed",
            )
            .with_detail(e.to_string())
        })?;
    // Nagle only delays interactive traffic; a failure here is harmless.
    let _ = stream.set_nodelay(true);
    // So that routers and NATs see traffic on an idle session, and a vanished peer becomes a
    // socket error (see `crate::net`). Best effort, like Nagle above.
    let _ = crate::net::set_tcp_keepalive(&stream);
    Ok(stream)
}

/// The server sent `DISCONNECT` while we authenticated: most likely `MaxAuthTries` was
/// reached, which retrying cannot fix.
fn closed_during_auth() -> Error {
    Error::coded(ErrorKind::Auth, "auth-failed", "Authentication failed")
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

/// A connection refused because the server's host key changed: either `known_hosts` has
/// another key of the same type for the host (`line`, 1-based), or the server presented
/// another key than the one verified when the session first connected (`line: None`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostKeyChange {
    pub line: Option<usize>,
}

/// Recognizes the error this crate reports for a changed host key (see
/// [`HostKeyChange`]). Any other error, `HostKey` ones included (cancelled, revoked), is
/// `None`.
pub fn host_key_change(error: &Error) -> Option<HostKeyChange> {
    let changed = error.kind == ErrorKind::HostKey
        && error.message.starts_with("Host key for ")
        && error.message.ends_with(" has changed");
    if !changed {
        return None;
    }
    let line = error.detail.as_deref().and_then(|detail| {
        detail
            .strip_prefix(CHANGED_LINE_PREFIX)?
            .strip_suffix(CHANGED_LINE_SUFFIX)?
            .parse()
            .ok()
    });
    Some(HostKeyChange { line })
}

/// The message of a changed host key; [`host_key_change`] recognizes it.
fn changed_message(host: &str) -> String {
    format!("Host key for {host} has changed")
}

/// The detail of a change found in `known_hosts`, at `line` (1-based).
const CHANGED_LINE_PREFIX: &str = "known_hosts line ";
const CHANGED_LINE_SUFFIX: &str = "; remove it to continue";

/// `known_hosts` has another key of the same type for `host`, at `line`.
pub(crate) fn known_hosts_changed(host: &str, line: usize) -> Error {
    Error::coded(
        ErrorKind::HostKey,
        "host-key-changed",
        changed_message(host),
    )
    .arg("host", host)
    .with_detail(format!("{CHANGED_LINE_PREFIX}{line}{CHANGED_LINE_SUFFIX}"))
}

/// `known_hosts` revokes the presented key for `host`, at `line`.
fn known_hosts_revoked(host: &str, line: usize) -> Error {
    Error::coded(
        ErrorKind::HostKey,
        "host-key-revoked",
        format!("Host key for {host} has been revoked"),
    )
    .arg("host", host)
    .with_detail(format!("known_hosts line {line}"))
}

/// Refuses a server key other than `pinned` (the key verified on the session's first
/// connection), whatever its algorithm.
fn check_pinned(pinned: Option<&PublicKey>, host: &str, key: &PublicKey) -> Result<()> {
    match pinned {
        Some(pinned) if pinned.key_data() != key.key_data() => Err(pinned_key_changed(
            host,
            key.algorithm().as_str(),
            &known_hosts::fingerprint(key),
        )),
        _ => Ok(()),
    }
}

/// The server presented a `key_type` key (`fingerprint`) instead of the one this session
/// verified when it first connected.
pub(crate) fn pinned_key_changed(host: &str, key_type: &str, fingerprint: &str) -> Error {
    Error::coded(
        ErrorKind::HostKey,
        "host-key-changed",
        changed_message(host),
    )
    .arg("host", host)
    .with_detail(format!(
        "the server presented a {key_type} key ({fingerprint}) instead of the key verified \
         when this session connected"
    ))
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
    let (code, message) = match kind {
        ErrorKind::Network => ("net-connection-lost", "Connection lost"),
        _ => ("ssh-protocol-error", "SSH protocol error"),
    };
    let detail = match e {
        // The connection log warns about it (see `hoprs_core::KEEPALIVE_UNANSWERED`).
        E::KeepaliveTimeout => hoprs_core::KEEPALIVE_UNANSWERED.to_owned(),
        e => e.to_string(),
    };
    Error::coded(kind, code, message).with_detail(detail)
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
                    Error::coded(ErrorKind::Network, "net-connection-lost", "Connection lost")
                        .with_detail(format!(
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
            return Err(HandlerError(Error::coded(
                ErrorKind::HostKey,
                "host-key-certificate-unsupported",
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
            HostKeyStatus::Unknown { other_key_types } => {
                let fingerprint = known_hosts::fingerprint(key);
                let accepted = self
                    .prompter
                    .confirm_host_key(
                        host,
                        self.port,
                        key.algorithm().as_str(),
                        &fingerprint,
                        &other_key_types,
                    )
                    .await;
                if !accepted {
                    return Err(HandlerError(Error::coded(
                        ErrorKind::HostKey,
                        "auth-cancelled",
                        "Cancelled by user",
                    )));
                }
                known_hosts::learn(&self.known_hosts, host, self.port, key)
                    .map_err(HandlerError)?;
                self.accepted(key);
                Ok(true)
            }
            HostKeyStatus::Changed { line } => Err(HandlerError(known_hosts_changed(host, line))),
            HostKeyStatus::Revoked { line } => Err(HandlerError(known_hosts_revoked(host, line))),
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
    fn host_key_change_is_recognized() {
        let ed = key(Algorithm::Ed25519);
        let pinned = check_pinned(Some(&ed), "h", &key(Algorithm::Ed25519)).unwrap_err();
        assert_eq!(host_key_change(&pinned), Some(HostKeyChange { line: None }));
        let changed = known_hosts_changed("h", 7);
        assert_eq!(changed.kind, ErrorKind::HostKey);
        assert_eq!(
            host_key_change(&changed),
            Some(HostKeyChange { line: Some(7) })
        );
        // Other host key errors are not a change.
        for other in [
            Error::new(ErrorKind::HostKey, "Cancelled by user"),
            known_hosts_revoked("h", 3),
            Error::new(ErrorKind::Network, "Host key for h has changed"),
        ] {
            assert_eq!(host_key_change(&other), None, "{other:?}");
        }
    }

    /// The SSH socket has TCP keepalive (60 s idle, 10 s interval, 3 probes), so routers and
    /// NATs see traffic on an idle session and a vanished peer becomes a socket error.
    #[tokio::test]
    async fn tcp_keepalive_is_set_on_sockets() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let cfg = SshConfig::new("127.0.0.1", port, "u");
        let stream = connect_tcp(&cfg).await.unwrap();
        assert_eq!(
            crate::net::tcp_keepalive_of(&stream).unwrap(),
            crate::net::KeepaliveSettings::expected()
        );
        assert!(stream.nodelay().unwrap());
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

    #[test]
    fn host_key_errors_carry_their_codes_and_host() {
        let changed = known_hosts_changed("example.org", 3);
        assert_eq!(changed.code, Some("host-key-changed"));
        assert_eq!(changed.args, vec![("host", "example.org".to_owned())]);
        assert_eq!(changed.message, "Host key for example.org has changed");
        let revoked = known_hosts_revoked("example.org", 3);
        assert_eq!(revoked.code, Some("host-key-revoked"));
        let pinned = pinned_key_changed("example.org", "ssh-ed25519", "SHA256:x");
        assert_eq!(pinned.code, Some("host-key-changed"));
    }

    #[test]
    fn transport_errors_carry_their_codes_and_keep_the_detail_uncoded() {
        let lost = map_russh_error(russh::Error::Disconnect);
        assert_eq!(lost.code, Some("net-connection-lost"));
        assert_eq!(lost.detail.as_deref(), Some("Disconnected"));
        let protocol = map_russh_error(russh::Error::Inconsistent);
        assert_eq!(protocol.code, Some("ssh-protocol-error"));
        assert!(user_disconnected_error().code == Some("net-disconnected-by-user"));
    }
}
