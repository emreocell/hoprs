//! FTP connections: [`open_control`] (connect, TLS, greeting, login, `TYPE I`, `FEAT`) and
//! [`FtpSession`] (control connection, transfer connection pool, keepalive, reconnection).

use std::fmt;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex, PoisonError, Weak};
use std::time::Duration;

use futures_util::future::BoxFuture;
use hoprs_core::{ConnectionState, Error, ErrorKind, Result};
use secrecy::{ExposeSecret, SecretString};
use suppaftp::FtpError;
use suppaftp::tokio::{AsyncRustlsConnector, AsyncRustlsFtpStream};
use suppaftp::types::FileType;
use suppaftp::{Mode, Status};
use tokio::sync::{
    Mutex, MutexGuard, Notify, OwnedSemaphorePermit, Semaphore, mpsc, oneshot, watch,
};
use tokio::time::Instant;

use super::config::{FtpConfig, FtpSecurity, TransferMode};
use super::errors::{data_connect_failed, is_control_loss, map_ftp_error, reply_parts};
use super::prompt::FtpPrompter;
use super::tls::{RejectedCert, RejectedSlot, connector, fingerprint};

/// The suppaftp stream type for every security mode. The rustls variant also speaks plain
/// FTP, and `into_secure` (explicit FTPS) keeps the same type.
pub(crate) type FtpStream = AsyncRustlsFtpStream;

/// A logged-in control connection in binary mode.
pub(crate) struct Control {
    pub(crate) stream: FtpStream,
    /// `FEAT` lists `MFMT`.
    pub(crate) has_mfmt: bool,
    /// `FEAT` lists `MLST` (servers announce `MLSD` support this way).
    pub(crate) has_mlsd: bool,
    /// `FEAT` lists `UTF8` (and `OPTS UTF8 ON` was sent). Informational (tests).
    #[cfg_attr(not(feature = "testing"), allow(dead_code))]
    pub(crate) has_utf8: bool,
}

/// Opens a control connection in any [`FtpSecurity`] mode.
///
/// When `cfg.password` is `None` the password is asked *before* connecting.
/// `cfg.connect_timeout` bounds each attempt's TCP connect, TLS handshake, greeting, login,
/// `TYPE I` and `FEAT`; time spent in prompts never counts against it.
///
/// FTPS: a certificate that is neither trusted (system roots plus `cfg.extra_roots`, valid
/// for `cfg.host`) nor equal to `cfg.pinned_fingerprint` goes to
/// [`FtpPrompter::confirm_certificate`]. If the user accepts, the fingerprint is stored with
/// [`FtpPrompter::store_pin`], written to `cfg.pinned_fingerprint`, and the connection is
/// opened again with that pin, once and without asking again. If the user refuses, the
/// result is a `Tls` error "Certificate not trusted" with the fingerprint in the detail.
pub(crate) async fn open_control(
    cfg: &mut FtpConfig,
    prompter: &dyn FtpPrompter,
) -> Result<Control> {
    let password = match &cfg.password {
        Some(p) => p.clone(),
        None => prompter
            .password(&cfg.user, &cfg.host)
            .await
            .ok_or_else(|| Error::new(ErrorKind::Auth, "Cancelled by user"))?,
    };

    let rejected = match attempt(cfg, &password).await? {
        Attempt::Connected(control) => return Ok(control),
        Attempt::CertificateRejected(rejected) => rejected,
    };
    let fp = fingerprint(&rejected.der);
    if !prompter
        .confirm_certificate(&cfg.host, cfg.port, &fp, &rejected.problem)
        .await
    {
        return Err(not_trusted(&fp));
    }
    prompter.store_pin(&cfg.host, cfg.port, &fp).await;
    cfg.pinned_fingerprint = Some(fp);
    match attempt(cfg, &password).await? {
        Attempt::Connected(control) => Ok(control),
        // The server switched certificates between the two connections.
        Attempt::CertificateRejected(rejected) => Err(not_trusted(&fingerprint(&rejected.der))),
    }
}

fn not_trusted(fp: &str) -> Error {
    Error::new(ErrorKind::Tls, "Certificate not trusted").with_detail(fp.to_owned())
}

enum Attempt {
    Connected(Control),
    /// The TLS handshake failed on this certificate, which needs the user's decision.
    CertificateRejected(RejectedCert),
}

/// One connection attempt, bounded by `cfg.connect_timeout`.
async fn attempt(cfg: &FtpConfig, password: &SecretString) -> Result<Attempt> {
    let target = format!("{}:{}", cfg.host, cfg.port);
    let slot = RejectedSlot::default();
    let tls = match cfg.security {
        FtpSecurity::Plain => None,
        FtpSecurity::ExplicitTls | FtpSecurity::ImplicitTls => Some(connector(cfg, slot.clone())?),
    };
    let result = tokio::time::timeout(cfg.connect_timeout, connect_and_login(cfg, password, tls))
        .await
        .unwrap_or_else(|_| {
            Err(Error::new(ErrorKind::Network, "Connection timed out").with_detail(target))
        });
    match result {
        Ok(control) => Ok(Attempt::Connected(control)),
        // The verifier fills the slot only when it refuses a certificate, which aborts the
        // handshake; any other TLS failure leaves it empty and is returned as is.
        Err(e) if e.kind == ErrorKind::Tls => {
            match slot.lock().unwrap_or_else(|p| p.into_inner()).take() {
                Some(rejected) => Ok(Attempt::CertificateRejected(rejected)),
                None => Err(e),
            }
        }
        Err(e) => Err(e),
    }
}

async fn connect_and_login(
    cfg: &FtpConfig,
    password: &SecretString,
    tls: Option<AsyncRustlsConnector>,
) -> Result<Control> {
    let target = format!("{}:{}", cfg.host, cfg.port);
    let addr = (cfg.host.as_str(), cfg.port);
    let connect_error = |e| match e {
        FtpError::ConnectionError(e) => Error::new(ErrorKind::Network, "Connection failed")
            .with_detail(format!("{target}: {e}")),
        e => map_ftp_error(e, &target),
    };
    // A refused `AUTH TLS`, `PBSZ` or `PROT` means the server does not offer FTPS.
    let refused = |e| match e {
        FtpError::UnexpectedResponse(reply) => {
            let (code, text) = reply_parts(&reply);
            Error::new(ErrorKind::Tls, format!("Server refused FTPS: {target}"))
                .with_detail(format!("{code} {text}"))
        }
        e => connect_error(e),
    };
    // Connects and reads the greeting. The TLS server name is `cfg.host`, a DNS name or an
    // IP address; both are checked against the certificate.
    let stream = match (cfg.security, tls) {
        (FtpSecurity::ImplicitTls, Some(tls)) => {
            let mut stream = FtpStream::connect_secure_implicit(addr, tls, &cfg.host)
                .await
                .map_err(connect_error)?;
            // Unlike `into_secure`, suppaftp does not protect the data channel here: without
            // `PROT P` data connections would be plain (and refused by most servers).
            for command in ["PBSZ 0", "PROT P"] {
                stream
                    .custom_command(command, &[Status::CommandOk])
                    .await
                    .map_err(refused)?;
            }
            stream
        }
        (FtpSecurity::ExplicitTls, Some(tls)) => {
            let stream = FtpStream::connect(addr).await.map_err(connect_error)?;
            // suppaftp sends `AUTH TLS`, does the handshake, then `PBSZ 0` and `PROT P`
            // right away, *before* login (RFC 4217 allows either order).
            stream.into_secure(tls, &cfg.host).await.map_err(refused)?
        }
        (FtpSecurity::Plain, _) => FtpStream::connect(addr).await.map_err(connect_error)?,
        // Never fall back to plain FTP when FTPS was asked for.
        (FtpSecurity::ExplicitTls | FtpSecurity::ImplicitTls, None) => {
            return Err(Error::new(ErrorKind::Tls, "FTPS needs a TLS configuration"));
        }
    };

    // Bounded, and marked so that a failure is a data channel error (see `data_connect_failed`).
    let data_connect_timeout = cfg.connect_timeout;
    let mut stream = stream.passive_stream_builder(move |addr| {
        Box::pin(async move {
            match tokio::time::timeout(data_connect_timeout, tokio::net::TcpStream::connect(addr))
                .await
            {
                Ok(Ok(socket)) => Ok(socket),
                Ok(Err(e)) => Err(data_connect_failed(addr, e)),
                Err(_) => Err(data_connect_failed(
                    addr,
                    std::io::Error::new(std::io::ErrorKind::TimedOut, "timed out"),
                )),
            }
        })
    });
    match cfg.mode {
        TransferMode::Passive => {
            // `PASV`, so that the NAT workaround applies: suppaftp replaces the announced
            // address with the control connection's peer (a no-op when they are equal).
            // `PASV` cannot express IPv6, so an IPv6 server gets `EPSV`, which has no address.
            let ipv6 = stream
                .get_ref()
                .await
                .peer_addr()
                .is_ok_and(|a| a.is_ipv6());
            stream.set_mode(if ipv6 {
                Mode::ExtendedPassive
            } else {
                Mode::Passive
            });
            stream.set_passive_nat_workaround(cfg.pasv_nat_workaround);
        }
        // `PORT` (`EPRT` for IPv6) with the control connection's local address. The server
        // must connect back within `connect_timeout`, and only the control peer is accepted.
        TransferMode::Active => stream = stream.active_mode(cfg.connect_timeout),
    }

    let who = format!("{}@{target}", cfg.user);
    // suppaftp logs every command at trace level, `PASS <password>` included: a logger must
    // keep the `suppaftp` target below trace (see the Plan 3 follow-ups).
    stream
        .login(cfg.user.as_str(), password.expose_secret())
        .await
        .map_err(|e| map_ftp_error(e, &who))?;
    stream
        .transfer_type(FileType::Binary)
        .await
        .map_err(|e| map_ftp_error(e, &who))?;

    // A server without FEAT (a negative reply) just has no optional features.
    let features = match stream.feat().await {
        Ok(f) => f,
        Err(FtpError::UnexpectedResponse(_)) => Default::default(),
        Err(e) => return Err(map_ftp_error(e, &who)),
    };
    let has = |name: &str| features.keys().any(|k| k.eq_ignore_ascii_case(name));
    let (has_mfmt, has_mlsd, has_utf8) = (has("MFMT"), has("MLST"), has("UTF8"));

    if has_utf8 {
        // Best effort: servers that are always UTF-8 may refuse it.
        match stream
            .custom_command("OPTS UTF8 ON", &[Status::CommandOk])
            .await
        {
            Ok(_) | Err(FtpError::UnexpectedResponse(_)) => {}
            Err(e) => return Err(map_ftp_error(e, &who)),
        }
    }

    Ok(Control {
        stream,
        has_mfmt,
        has_mlsd,
        has_utf8,
    })
}

/// Sends a raw command and returns the reply's code and text, whatever the code. Socket
/// errors are mapped with `context` (see [`map_ftp_error`]).
pub(crate) async fn quote(
    stream: &mut FtpStream,
    command: &str,
    context: &str,
) -> Result<(u32, String)> {
    match stream.custom_command(command, &[]).await {
        Ok(reply) | Err(FtpError::UnexpectedResponse(reply)) => Ok(reply_parts(&reply)),
        Err(e) => Err(map_ftp_error(e, context)),
    }
}

// ---------------------------------------------------------------------------------------
// FtpSession: control connection, transfer connection pool, keepalive and reconnection.
// ---------------------------------------------------------------------------------------

/// Highest number of transfer connections a session opens.
const MAX_TRANSFER_CONNECTIONS: usize = 8;

/// An FTP session that stays connected: one control connection for commands, up to
/// `transfer_connections` extra connections for transfers (opened and logged in on demand),
/// a `NOOP` keepalive, and automatic reconnection. Cheap to clone; the connections close
/// when the last clone is dropped.
///
/// Every connection is opened by [`open_control`] with the session's config, so TLS,
/// certificate checks and login are identical for all of them. A certificate accepted on
/// any connection is pinned in that config for the following ones.
#[derive(Clone)]
pub struct FtpSession {
    inner: Arc<Inner>,
}

struct Inner {
    shared: Arc<Shared>,
    /// Dropping the last sender stops the supervisor.
    commands: mpsc::UnboundedSender<Command>,
}

/// State shared between the handles, the supervisor and the pooled connections.
struct Shared {
    /// Config for every new connection; it holds the password (asked once) and the latest
    /// accepted certificate pin.
    cfg: StdMutex<FtpConfig>,
    prompter: Arc<dyn FtpPrompter>,
    keepalive_interval: Duration,
    /// Bounds a keepalive `NOOP` and the check of an idle transfer connection.
    reply_timeout: Duration,
    /// Bounds every `with_control` command (see [`FtpConfig::command_timeout`]).
    command_timeout: Duration,
    /// Bounds the silence of a transfer's data connection (see [`FtpConfig::data_timeout`]).
    data_timeout: Duration,
    /// The control connection and its generation; `None` once lost or closed.
    control: Mutex<Option<(Control, u64)>>,
    /// When the control connection last finished a command.
    last_used: StdMutex<Instant>,
    generation: AtomicU64,
    state: watch::Sender<ConnectionState>,
    /// Idle transfer connections, tagged with their generation.
    idle: StdMutex<Vec<(FtpStream, u64)>>,
    /// One permit per checked-out transfer connection.
    permits: Arc<Semaphore>,
    /// `transfer_connections`: the number of permits.
    transfer_connections: usize,
    /// The most transfer connections checked out at once (tests).
    peak_checked_out: AtomicUsize,
    /// Tells the supervisor that the control connection of a generation was lost.
    lost: mpsc::UnboundedSender<(u64, Error)>,
    /// Asks the supervisor to probe the control connection now (a transfer connection was
    /// lost, so the server may be gone).
    probe: Notify,
}

impl Shared {
    fn set_state(&self, new: ConnectionState) {
        self.state.send_if_modified(|s| {
            let changed = *s != new;
            *s = new;
            changed
        });
    }

    fn is_connected(&self) -> bool {
        *self.state.borrow() == ConnectionState::Connected
    }

    fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    fn cfg(&self) -> FtpConfig {
        self.cfg
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Keeps a certificate pin accepted while opening a connection with a copy of the config.
    fn remember_pin(&self, used: &FtpConfig) {
        let mut cfg = self.cfg.lock().unwrap_or_else(PoisonError::into_inner);
        if cfg.pinned_fingerprint != used.pinned_fingerprint {
            cfg.pinned_fingerprint = used.pinned_fingerprint.clone();
        }
    }

    /// Opens and logs in a new connection with the session's config.
    async fn open(&self) -> Result<Control> {
        let mut cfg = self.cfg();
        let result = open_control(&mut cfg, &*self.prompter).await;
        self.remember_pin(&cfg);
        result
    }

    fn touch(&self) {
        *self
            .last_used
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Instant::now();
    }

    fn last_used(&self) -> Instant {
        *self
            .last_used
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// An idle transfer connection of `generation`; older ones are dropped (closed).
    fn take_idle(&self, generation: u64) -> Option<FtpStream> {
        let mut idle = self.idle.lock().unwrap_or_else(PoisonError::into_inner);
        idle.retain(|(_, g)| *g == generation);
        idle.pop().map(|(stream, _)| stream)
    }

    fn clear_idle(&self) {
        // Taken out first: the sockets close after the lock is released.
        let stale = std::mem::take(&mut *self.idle.lock().unwrap_or_else(PoisonError::into_inner));
        drop(stale);
    }

    /// Closes the control connection, unless a command is running on it (that command sees
    /// the state change and closes it), and every idle transfer connection. Never waits.
    fn close(&self) {
        if let Ok(mut slot) = self.control.try_lock() {
            slot.take();
        }
        self.clear_idle();
    }
}

/// Requests from the handles; each is acknowledged once the state reflects it.
enum Command {
    Disconnect(oneshot::Sender<()>),
}

impl fmt::Debug for FtpSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let cfg = self.inner.shared.cfg();
        f.debug_struct("FtpSession")
            .field("host", &cfg.host)
            .field("port", &cfg.port)
            .field("user", &cfg.user)
            .field("security", &cfg.security)
            .finish()
    }
}

fn not_connected() -> Error {
    Error::new(ErrorKind::Network, "Not connected")
}

fn user_disconnected() -> ConnectionState {
    ConnectionState::Disconnected(Error::new(ErrorKind::Network, "Disconnected by user"))
}

impl FtpSession {
    /// Opens the control connection (see [`open_control`]) and starts the supervisor.
    ///
    /// When `cfg.password` is `None` it is asked once, here: transfer connections and
    /// reconnections reuse it. `cfg.transfer_connections` is clamped to 1..=8.
    pub async fn connect(mut cfg: FtpConfig, prompter: Arc<dyn FtpPrompter>) -> Result<FtpSession> {
        if cfg.password.is_none() {
            let password = prompter
                .password(&cfg.user, &cfg.host)
                .await
                .ok_or_else(|| Error::new(ErrorKind::Auth, "Cancelled by user"))?;
            cfg.password = Some(password);
        }
        cfg.transfer_connections = cfg.transfer_connections.clamp(1, MAX_TRANSFER_CONNECTIONS);
        let control = open_control(&mut cfg, &*prompter).await?;

        let (state, _) = watch::channel(ConnectionState::Connected);
        let (lost, lost_rx) = mpsc::unbounded_channel();
        let shared = Arc::new(Shared {
            keepalive_interval: cfg.keepalive_interval,
            reply_timeout: cfg.connect_timeout,
            command_timeout: cfg.command_timeout,
            data_timeout: cfg.data_timeout,
            permits: Arc::new(Semaphore::new(cfg.transfer_connections)),
            transfer_connections: cfg.transfer_connections,
            peak_checked_out: AtomicUsize::new(0),
            cfg: StdMutex::new(cfg),
            prompter,
            control: Mutex::new(Some((control, 1))),
            last_used: StdMutex::new(Instant::now()),
            generation: AtomicU64::new(1),
            state,
            idle: StdMutex::new(Vec::new()),
            lost,
            probe: Notify::new(),
        });
        let (commands, rx) = mpsc::unbounded_channel();
        let supervisor = Supervisor {
            shared: shared.clone(),
            commands: rx,
            lost: lost_rx,
        };
        tokio::spawn(supervisor.run());
        Ok(FtpSession {
            inner: Arc::new(Inner { shared, commands }),
        })
    }

    /// Watches the connection state.
    pub fn state(&self) -> watch::Receiver<ConnectionState> {
        self.inner.shared.state.subscribe()
    }

    /// Starts at 1 and goes up by one after every successful reconnection. Transfer
    /// connections of an older generation are never handed out again.
    pub fn generation(&self) -> u64 {
        self.inner.shared.generation()
    }

    /// Runs `f` on the control connection; calls are serialized.
    ///
    /// Never waits for a reconnection: unless `Connected` it fails at once with `Network`,
    /// "Not connected". A control loss from `f` (see [`is_control_loss`]: socket errors,
    /// `421`, but not the data channel errors `425`/`426` or a refused passive connection)
    /// discards the control connection, starts the supervisor reconnecting, and is returned.
    /// So does a `Tls` error: suppaftp runs the data channel handshake after reading the
    /// transfer command's `150`, so its final reply is still pending on the control stream.
    ///
    /// The whole of `f` (a directory listing included) is bounded by
    /// [`FtpConfig::command_timeout`], generous on purpose; exceeding it is a control loss,
    /// `Network`, "Control command timed out". If the session is disconnected meanwhile, `f`
    /// is abandoned, the connection closed, and the result is "Not connected". If the returned
    /// future is dropped before `f` finishes, the connection is in an unknown state (a reply
    /// may be pending) and is treated as lost as well.
    pub(crate) async fn with_control<T, F>(&self, f: F) -> Result<T>
    where
        F: for<'c> FnOnce(&'c mut Control) -> BoxFuture<'c, Result<T>>,
    {
        let shared = &*self.inner.shared;
        if !shared.is_connected() {
            return Err(not_connected());
        }
        let mut state = shared.state.subscribe();
        let slot = shared.control.lock().await;
        let mut guard = InFlight {
            slot,
            shared,
            done: false,
        };
        let Some((control, generation)) = guard.slot.as_mut() else {
            guard.done = true;
            return Err(not_connected());
        };
        let generation = *generation;
        let outcome = tokio::select! {
            // A command that finished is seen first (the slot is dropped below if the session
            // was disconnected meanwhile).
            biased;
            result = tokio::time::timeout(shared.command_timeout, f(control)) => {
                Some(result.unwrap_or_else(|_| {
                    Err(Error::new(ErrorKind::Network, "Control command timed out"))
                }))
            }
            _ = state.wait_for(|s| matches!(s, ConnectionState::Disconnected(_))) => None,
        };
        guard.done = true;
        shared.touch();
        let Some(result) = outcome else {
            // Disconnected by the user: the abandoned command may have left a reply pending.
            guard.slot.take();
            return Err(not_connected());
        };
        if let Err(e) = &result
            && (is_control_loss(e) || e.kind == ErrorKind::Tls)
        {
            guard.slot.take();
            let _ = shared.lost.send((generation, e.clone()));
        }
        drop(guard);
        // Disconnected while the command ran or finished: `close` could not take the busy
        // connection, so it is taken here, after the lock is released (a disconnect landing
        // between a check and the release would otherwise be missed). `Disconnected` is final,
        // so nothing newer can be in the slot. If someone else holds the lock, they run this
        // same check once they release it.
        if matches!(*shared.state.borrow(), ConnectionState::Disconnected(_))
            && let Ok(mut slot) = shared.control.try_lock()
        {
            slot.take();
        }
        result
    }

    /// A logged-in transfer connection in binary mode: an idle one of the current
    /// generation, or a new one while fewer than `transfer_connections` are checked out.
    /// Otherwise waits for one to come back; the wait can be cancelled by dropping the
    /// future and never holds the control connection.
    ///
    /// Unless `Connected` (also when the state changes while waiting) it fails at once with
    /// `Network`, "Not connected". Errors opening a new connection are returned as is.
    pub async fn checkout(&self) -> Result<PooledConn> {
        let shared = &self.inner.shared;
        let mut state = shared.state.subscribe();
        if *state.borrow_and_update() != ConnectionState::Connected {
            return Err(not_connected());
        }
        let permit = tokio::select! {
            permit = shared.permits.clone().acquire_owned() => {
                permit.map_err(|_| not_connected())?
            }
            _ = state.wait_for(|s| *s != ConnectionState::Connected) => {
                return Err(not_connected());
            }
        };
        let checked_out = shared.transfer_connections - shared.permits.available_permits();
        shared
            .peak_checked_out
            .fetch_max(checked_out, Ordering::Relaxed);
        let pooled = |stream, generation| PooledConn {
            stream: Some(stream),
            generation,
            clean: false,
            shared: Arc::downgrade(shared),
            _permit: permit,
        };

        let generation = shared.generation();
        while let Some(mut stream) = shared.take_idle(generation) {
            // The server may have closed it while idle (idle timeout).
            if let Ok(Ok(())) = tokio::time::timeout(shared.reply_timeout, stream.noop()).await {
                return Ok(pooled(stream, generation));
            }
        }
        let control = shared.open().await?;
        if shared.generation() != generation || !shared.is_connected() {
            return Err(not_connected());
        }
        Ok(pooled(control.stream, generation))
    }

    /// The most transfer connections checked out at once so far.
    #[cfg(feature = "testing")]
    pub(crate) fn peak_transfer_connections(&self) -> usize {
        self.inner.shared.peak_checked_out.load(Ordering::Relaxed)
    }

    /// [`FtpConfig::command_timeout`]: bounds a whole command, a transfer's final reply
    /// included.
    pub(crate) fn command_timeout(&self) -> Duration {
        self.inner.shared.command_timeout
    }

    /// [`FtpConfig::data_timeout`]: bounds the silence of a transfer's data connection.
    pub(crate) fn data_timeout(&self) -> Duration {
        self.inner.shared.data_timeout
    }

    /// [`FtpConfig::connect_timeout`]: bounds a reply that should come at once (a `NOOP`).
    pub(crate) fn reply_timeout(&self) -> Duration {
        self.inner.shared.reply_timeout
    }

    /// Closes every connection and stops reconnecting: the state becomes `Disconnected`
    /// with `Network`, "Disconnected by user" (or keeps its error if already disconnected).
    pub async fn disconnect(&self) {
        let (ack, done) = oneshot::channel();
        if self.inner.commands.send(Command::Disconnect(ack)).is_ok() {
            let _ = done.await;
        }
    }
}

/// Holds the control connection during a command. Dropped before the command finished
/// (the caller's future was cancelled), it discards the connection and reports it lost.
struct InFlight<'a> {
    slot: MutexGuard<'a, Option<(Control, u64)>>,
    shared: &'a Shared,
    done: bool,
}

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        if !self.done
            && let Some((_, generation)) = self.slot.take()
        {
            let e = Error::new(ErrorKind::Network, "Connection lost")
                .with_detail("a command was interrupted before its reply");
            let _ = self.shared.lost.send((generation, e));
        }
    }
}

/// A transfer connection checked out of an [`FtpSession`]'s pool.
///
/// Dropped, it goes back to the pool only if [`mark_clean`](Self::mark_clean) was called
/// and the session has not reconnected since; otherwise it is closed. Its pool slot is
/// freed either way.
pub struct PooledConn {
    /// `Some` until dropped.
    stream: Option<FtpStream>,
    generation: u64,
    clean: bool,
    shared: Weak<Shared>,
    _permit: OwnedSemaphorePermit,
}

impl PooledConn {
    /// The logged-in connection, in binary mode.
    pub fn stream(&mut self) -> &mut FtpStream {
        self.stream
            .as_mut()
            .expect("the stream is present until drop")
    }

    /// The session generation this connection was opened in.
    #[cfg(feature = "testing")]
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Tells the session about an error on this connection. A connection loss (see
    /// [`is_control_loss`]) makes the session probe its control connection at once with a
    /// `NOOP`, so a server that is gone is detected without waiting for the keepalive. It does
    /// not reconnect by itself: only a failed probe does.
    pub fn report(&self, error: &Error) {
        if !is_control_loss(error) {
            return;
        }
        if let Some(shared) = self.shared.upgrade()
            && shared.generation() == self.generation
        {
            shared.probe.notify_one();
        }
    }

    /// The connection is idle again (no transfer open, every reply read) and can be reused.
    pub fn mark_clean(&mut self) {
        self.clean = true;
    }
}

impl Drop for PooledConn {
    fn drop(&mut self) {
        let (Some(stream), Some(shared)) = (self.stream.take(), self.shared.upgrade()) else {
            return;
        };
        if self.clean && self.generation == shared.generation() && shared.is_connected() {
            shared
                .idle
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push((stream, self.generation));
        }
    }
}

/// What the supervisor does next.
enum Phase {
    Connected,
    Reconnecting(Error),
    Disconnected,
    Stop,
}

/// Keeps the control connection alive, reconnects, and serves handle commands. Ends when
/// the last handle is dropped.
struct Supervisor {
    shared: Arc<Shared>,
    commands: mpsc::UnboundedReceiver<Command>,
    lost: mpsc::UnboundedReceiver<(u64, Error)>,
}

impl Supervisor {
    async fn run(mut self) {
        let mut phase = Phase::Connected;
        loop {
            phase = match phase {
                Phase::Connected => self.connected().await,
                Phase::Reconnecting(last) => self.reconnect(last).await,
                Phase::Disconnected => self.disconnected().await,
                Phase::Stop => break,
            };
        }
        self.shared.close();
    }

    async fn connected(&mut self) -> Phase {
        let Self {
            shared,
            commands,
            lost,
        } = self;
        let interval = shared.keepalive_interval;
        let mut next = shared.last_used() + interval;
        loop {
            let probe = tokio::select! {
                Some((generation, e)) = lost.recv() => {
                    // Reports about an older connection are stale.
                    if generation == shared.generation() {
                        return Phase::Reconnecting(e);
                    }
                    continue;
                }
                _ = tokio::time::sleep_until(next) => false,
                _ = shared.probe.notified() => true,
                command = commands.recv() => return on_command(shared, command),
            };
            // Commands are served while the NOOP waits for its reply: a disconnect drops the
            // NOOP (and its hold on the control connection) and closes the connection.
            let result = tokio::select! {
                result = keepalive(shared, probe) => result,
                command = commands.recv() => return on_command(shared, command),
            };
            match result {
                Ok(at) => next = at,
                Err(e) => return Phase::Reconnecting(e),
            }
        }
    }

    /// One round of attempts per `cfg.reconnect`. `Auth` and `Tls` errors end it at once.
    async fn reconnect(&mut self, mut last: Error) -> Phase {
        let Self {
            shared, commands, ..
        } = self;
        shared.clear_idle();
        let policy = shared.cfg().reconnect;
        for (index, delay) in policy.delays().into_iter().enumerate() {
            let attempt = u32::try_from(index + 1).unwrap_or(u32::MAX);
            shared.set_state(ConnectionState::Reconnecting { attempt });
            let try_once = async {
                tokio::time::sleep(delay).await;
                shared.open().await
            };
            // Every command ends the round, so the attempt is never resumed.
            let result = tokio::select! {
                result = try_once => result,
                command = commands.recv() => match command {
                    None => return Phase::Stop,
                    Some(Command::Disconnect(ack)) => {
                        // Dropping `try_once` abandons the attempt and its prompts.
                        shared.set_state(user_disconnected());
                        let _ = ack.send(());
                        return Phase::Disconnected;
                    }
                },
            };
            match result {
                Ok(control) => {
                    let mut slot = shared.control.lock().await;
                    shared.clear_idle();
                    let generation = shared.generation.fetch_add(1, Ordering::SeqCst) + 1;
                    *slot = Some((control, generation));
                    drop(slot);
                    shared.touch();
                    shared.set_state(ConnectionState::Connected);
                    return Phase::Connected;
                }
                Err(e) if matches!(e.kind, ErrorKind::Auth | ErrorKind::Tls) => {
                    shared.set_state(ConnectionState::Disconnected(e));
                    return Phase::Disconnected;
                }
                Err(e) => last = e,
            }
        }
        shared.set_state(ConnectionState::Disconnected(last));
        Phase::Disconnected
    }

    async fn disconnected(&mut self) -> Phase {
        loop {
            match self.commands.recv().await {
                None => return Phase::Stop,
                Some(Command::Disconnect(ack)) => {
                    let _ = ack.send(());
                }
            }
        }
    }
}

/// A handle command received while connected.
fn on_command(shared: &Shared, command: Option<Command>) -> Phase {
    match command {
        None => Phase::Stop,
        Some(Command::Disconnect(ack)) => {
            // A running command sees the state change and closes the connection.
            shared.set_state(user_disconnected());
            shared.close();
            let _ = ack.send(());
            Phase::Disconnected
        }
    }
}

/// Sends `NOOP` if the control connection has been idle for `keepalive_interval`, or
/// whenever it is not busy with `probe`. Returns when to check again, or the error that
/// lost the connection.
async fn keepalive(shared: &Shared, probe: bool) -> Result<Instant> {
    let interval = shared.keepalive_interval;
    // A running command means the connection is not idle.
    let Ok(mut slot) = shared.control.try_lock() else {
        return Ok(Instant::now() + interval);
    };
    let last = shared.last_used();
    if !probe && last.elapsed() < interval {
        return Ok(last + interval);
    }
    // Already lost: the report is on its way.
    let Some((control, _)) = slot.as_mut() else {
        return Ok(Instant::now() + interval);
    };
    let result = tokio::time::timeout(shared.reply_timeout, control.stream.noop()).await;
    shared.touch();
    let error = match result {
        Ok(Ok(())) => return Ok(Instant::now() + interval),
        Ok(Err(e)) => map_ftp_error(e, "keepalive"),
        Err(_) => Error::new(ErrorKind::Network, "Connection lost")
            .with_detail("no reply to the keepalive NOOP"),
    };
    slot.take();
    Err(error)
}
