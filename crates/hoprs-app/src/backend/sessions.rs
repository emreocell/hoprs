//! The session manager: connects saved sites, registers them with the transfer engine,
//! watches them and closes them.
//!
//! **Connect shape (ruling R5).** `SftpFs::new` needs a *connected* SSH session, so
//! [`SessionManager::connect`] starts the connection (`SshSession::start` /
//! `FtpFs::start`), keeps it under a fresh [`SessionId`] at once (its state is observable
//! and [`close`](SessionManager::close) can cancel it, prompts included), then waits for
//! the first `Connected`. Only then does it build the remote filesystem (`SftpFs` over the
//! session, or the `FtpFs` itself), register the site with the engine
//! (`LocalFs::new("/")` as the local side) and spawn its `spawn_site_watcher`. FTP takes the
//! same path for uniformity: `connect` returns once either protocol can list.
//!
//! If the first connection ends `Disconnected`, `connect` fails with a [`ConnectError`]
//! that still carries the session id: the session stays (so the UI shows its error) and
//! [`reconnect`](SessionManager::reconnect) retries it (`reconnect_now`, then the same
//! wait-and-register). [`close`](SessionManager::close) drops it.
//!
//! One session per site: `connect` on a site that already has one returns that session
//! (retrying it if it is `Disconnected`). The engine keys endpoints by site, so two live
//! sessions of one site would replace each other's endpoints.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use futures_util::future::BoxFuture;
use futures_util::stream::FuturesUnordered;
use futures_util::{FutureExt, StreamExt};
use hoprs_core::{ConnectionState, Error, ErrorKind, Protocol, Result, Site, SiteId};
use hoprs_remote::{Connection, FtpFs, LocalFs, RemoteFs, SftpConfig, SftpFs};
use hoprs_ssh::SshSession;
use hoprs_transfer::{SiteEndpoints, TransferEngine, spawn_site_watcher};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

use super::broker::{PromptBroker, SessionPrompter};
use super::mapping::{SiteSecrets, ftp_config, ssh_config};
use super::vault::VaultHandle;

/// Identifies one open session. Monotonic: never reused within a run.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SessionId(pub u64);

/// A connected session.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Connected {
    pub session: SessionId,
    /// The keyring could not be reached when the session's secrets were read, so its
    /// stored password or passphrase (if any) was not used: the UI says so, which tells a
    /// locked keyring apart from no stored secret.
    pub keyring_unavailable: bool,
}

/// What connecting one site gives.
pub type ConnectResult = std::result::Result<Connected, ConnectError>;

/// A failed connect. `session` is `Some` when the session exists (its first connection
/// failed): [`SessionManager::reconnect`] retries it, [`SessionManager::close`] drops it.
#[derive(Clone, Debug, PartialEq)]
pub struct ConnectError {
    pub session: Option<SessionId>,
    pub error: Error,
    /// As in [`Connected::keyring_unavailable`]: the keyring was unreachable when the
    /// secrets were read (a likely reason for an authentication failure).
    pub keyring_unavailable: bool,
}

impl ConnectError {
    fn before_start(error: Error) -> Self {
        Self {
            session: None,
            error,
            keyring_unavailable: false,
        }
    }
}

/// Session manager options.
#[derive(Clone, Debug)]
pub struct SessionConfig {
    /// Replaces `~/.ssh/known_hosts` (tests).
    pub known_hosts: Option<PathBuf>,
    /// Transfer workers per site; also the SFTP stream channels and FTP transfer
    /// connections (both clamped to `1..=8`). The starting value: see
    /// [`SessionManager::set_workers`].
    pub workers: usize,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            known_hosts: None,
            workers: 4,
        }
    }
}

/// The live connection of a session.
#[derive(Clone)]
enum Link {
    Ssh(SshSession),
    Ftp(FtpFs),
    /// A test double that only reports state.
    #[cfg(test)]
    Fake(Arc<dyn Connection>),
}

struct Entry {
    site: SiteId,
    link: Link,
    prompter: SessionPrompter,
    /// The secrets were read while the keyring was unreachable.
    keyring_unavailable: bool,
    /// Set once the first connection is up and the site is registered.
    remote: Option<Arc<dyn RemoteFs>>,
    watcher: Option<JoinHandle<()>>,
}

/// Connects, registers, watches and closes sessions. Cheap to clone.
#[derive(Clone)]
pub struct SessionManager {
    inner: Arc<Inner>,
}

struct Inner {
    engine: TransferEngine,
    vault: VaultHandle,
    broker: PromptBroker,
    config: SessionConfig,
    /// Transfer workers per site for the next connections (Settings).
    workers: AtomicUsize,
    local: Arc<dyn RemoteFs>,
    next_id: AtomicU64,
    sessions: Mutex<HashMap<SessionId, Entry>>,
    /// Serializes connect, reconnect and close per site.
    site_locks: Mutex<HashMap<SiteId, Arc<tokio::sync::Mutex<()>>>>,
    /// Sites `close` paused in the engine; the next connect resumes them.
    paused_by_close: Mutex<HashSet<SiteId>>,
    /// Remotes of the test sessions made by `attach`, until they are ready.
    #[cfg(test)]
    test_remotes: Mutex<HashMap<SessionId, Arc<dyn RemoteFs>>>,
}

impl Link {
    fn connection(&self) -> Arc<dyn Connection> {
        match self {
            Self::Ssh(session) => Arc::new(session.clone()),
            Self::Ftp(fs) => Arc::new(fs.clone()),
            #[cfg(test)]
            Self::Fake(conn) => conn.clone(),
        }
    }

    fn state(&self) -> watch::Receiver<ConnectionState> {
        match self {
            Self::Ssh(session) => session.state(),
            Self::Ftp(fs) => Connection::state(fs),
            #[cfg(test)]
            Self::Fake(conn) => conn.state(),
        }
    }

    async fn reconnect_now(&self) {
        match self {
            Self::Ssh(session) => session.reconnect_now().await,
            Self::Ftp(fs) => fs.reconnect_now().await,
            #[cfg(test)]
            Self::Fake(_) => {}
        }
    }

    async fn disconnect(&self) {
        match self {
            Self::Ssh(session) => session.disconnect().await,
            Self::Ftp(fs) => fs.disconnect().await,
            #[cfg(test)]
            Self::Fake(_) => {}
        }
    }
}

fn closed() -> Error {
    Error::new(ErrorKind::NotFound, "Session closed")
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl SessionManager {
    pub fn new(
        engine: TransferEngine,
        vault: VaultHandle,
        broker: PromptBroker,
        config: SessionConfig,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                engine,
                vault,
                broker,
                workers: AtomicUsize::new(config.workers),
                config,
                local: Arc::new(LocalFs::new("/")),
                next_id: AtomicU64::new(1),
                sessions: Mutex::new(HashMap::new()),
                site_locks: Mutex::new(HashMap::new()),
                paused_by_close: Mutex::new(HashSet::new()),
                #[cfg(test)]
                test_remotes: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// Transfer workers per site for the connections made from now on.
    pub fn workers(&self) -> usize {
        self.inner.workers.load(Ordering::Relaxed)
    }

    /// Sets the workers per site (Settings). Sessions already registered keep theirs;
    /// the next connection of a site (a new session) uses this value.
    pub fn set_workers(&self, workers: usize) {
        self.inner.workers.store(workers, Ordering::Relaxed);
    }

    /// Connects the saved site `site` (see the module docs). The app connects through
    /// [`connect_batch`](Self::connect_batch); this is for the tests.
    #[cfg(test)]
    pub async fn connect(&self, site: SiteId) -> ConnectResult {
        let (_, result) = self
            .connect_many(vec![site])
            .await
            .pop()
            .expect("one result per site");
        result
    }

    /// Connects several sites at once and returns when all are done. Their secrets are
    /// read with one [`SecretBatch`](super::vault::SecretBatch): a locked keyring raises at
    /// most one unlock prompt, and the sites after it ask for their passwords instead.
    /// Results come in the order of `sites`; a site missing from the vault is `NotFound`.
    #[cfg(test)]
    pub async fn connect_many(&self, sites: Vec<SiteId>) -> Vec<(SiteId, ConnectResult)> {
        futures_util::future::join_all(self.site_connects(sites).await).await
    }

    /// Open workspace: like [`connect_many`](Self::connect_many) (one secret batch, every
    /// site in parallel), but each site's result is sent on `results` as soon as that site
    /// finishes, so one slow or failing site never holds back the others. Returns once
    /// every result was sent (or the receiver is gone).
    pub async fn connect_batch(
        &self,
        sites: Vec<SiteId>,
        results: mpsc::UnboundedSender<(SiteId, ConnectResult)>,
    ) {
        let mut pending: FuturesUnordered<_> =
            self.site_connects(sites).await.into_iter().collect();
        while let Some(result) = pending.next().await {
            // A send fails only once the receiver is gone. In the app the receiver and this
            // future live in the same stream, so they are dropped together: the remaining
            // connects are then cancelled with it (not finished). A session already started
            // stays registered, `Disconnected` or connecting, for the next connect or close.
            let _ = results.send(result);
        }
    }

    /// Reads the secrets of `sites` as one batch, then one connect future per site.
    async fn site_connects(
        &self,
        sites: Vec<SiteId>,
    ) -> Vec<BoxFuture<'_, (SiteId, ConnectResult)>> {
        let read = match self.inner.vault.sites_with_secrets(sites.clone()).await {
            Ok(read) => read,
            Err(error) => {
                return sites
                    .into_iter()
                    .map(|site| {
                        let result = Err(ConnectError::before_start(error.clone()));
                        futures_util::future::ready((site, result)).boxed()
                    })
                    .collect();
            }
        };
        read.into_iter()
            .map(|(id, found)| {
                async move {
                    let result = match found {
                        Ok((site, secrets)) => self.connect_site(site, secrets).await,
                        Err(error) => Err(ConnectError::before_start(error)),
                    };
                    (id, result)
                }
                .boxed()
            })
            .collect()
    }

    async fn connect_site(&self, site: Site, secrets: SiteSecrets) -> ConnectResult {
        let site_lock = self.site_lock(&site.id);
        let _turn = site_lock.lock().await;
        let existing = lock(&self.inner.sessions)
            .iter()
            .find(|(_, entry)| entry.site == site.id)
            .map(|(id, entry)| (*id, entry.link.clone(), entry.keyring_unavailable));
        // An existing session keeps the secrets (and the flag) it was started with.
        let (id, keyring_unavailable) = match existing {
            Some((id, link, keyring_unavailable)) => {
                // Does nothing unless `Disconnected`.
                link.reconnect_now().await;
                (id, keyring_unavailable)
            }
            None => (self.start(&site, &secrets), secrets.keyring_unavailable),
        };
        match self.ready(id).await {
            Ok(()) => Ok(Connected {
                session: id,
                keyring_unavailable,
            }),
            Err(error) => Err(ConnectError {
                session: Some(id),
                error,
                keyring_unavailable,
            }),
        }
    }

    /// Starts the connection of `site` under a new session id.
    fn start(&self, site: &Site, secrets: &SiteSecrets) -> SessionId {
        let id = SessionId(self.inner.next_id.fetch_add(1, Ordering::SeqCst));
        let prompter = self
            .inner
            .broker
            .session(id, site.id.clone(), &site.host, site.port);
        let config = &self.inner.config;
        let link = match site.protocol {
            Protocol::Ssh => {
                let mut ssh = ssh_config(site, secrets);
                if let Some(known_hosts) = &config.known_hosts {
                    ssh.known_hosts = known_hosts.clone();
                }
                Link::Ssh(SshSession::start(ssh, Arc::new(prompter.clone())))
            }
            Protocol::Ftp | Protocol::FtpsExplicit | Protocol::FtpsImplicit => {
                let mut ftp = ftp_config(site, secrets);
                ftp.transfer_connections = self.workers().clamp(1, 8);
                Link::Ftp(FtpFs::start(ftp, Arc::new(prompter.clone())))
            }
        };
        self.insert(
            id,
            site.id.clone(),
            link,
            prompter,
            secrets.keyring_unavailable,
        );
        id
    }

    fn insert(
        &self,
        id: SessionId,
        site: SiteId,
        link: Link,
        prompter: SessionPrompter,
        keyring_unavailable: bool,
    ) {
        let entry = Entry {
            site,
            link,
            prompter,
            keyring_unavailable,
            remote: None,
            watcher: None,
        };
        lock(&self.inner.sessions).insert(id, entry);
    }

    /// Waits for the session's first `Connected` (or its failure), then, once per session,
    /// builds its remote filesystem, registers the site and spawns its watcher. Called
    /// with the site's lock held.
    ///
    /// The outcome also settles the secrets typed with "Save in the system keyring" (see
    /// [`SessionPrompter::take_pending_secrets`]): `Connected` writes them to the keyring,
    /// a failure drops them.
    async fn ready(&self, id: SessionId) -> Result<()> {
        let (link, site, built, prompter) = {
            let sessions = lock(&self.inner.sessions);
            let entry = sessions.get(&id).ok_or_else(closed)?;
            (
                entry.link.clone(),
                entry.site.clone(),
                entry.remote.is_some(),
                entry.prompter.clone(),
            )
        };
        let mut state = link.state();
        let outcome = state
            .wait_for(|s| {
                matches!(
                    s,
                    ConnectionState::Connected | ConnectionState::Disconnected(_)
                )
            })
            .await
            .map(|s| s.clone())
            .map_err(|_| closed())?;
        if let ConnectionState::Disconnected(error) = outcome {
            prompter.discard_pending_secrets();
            return Err(error);
        }
        let typed = {
            let sessions = lock(&self.inner.sessions);
            if !sessions.contains_key(&id) {
                // Closed while connecting: `close` dropped the secrets already.
                return Err(closed());
            }
            // Taken under the lock `close` removes the session with: a close that comes
            // first leaves nothing to save; one that comes after finds them gone. Either
            // way a closed session never stores what it took.
            prompter.take_pending_secrets()
        };
        self.save_typed_secrets(&site, typed).await;
        if built {
            return Ok(());
        }

        let workers = self.workers();
        let remote: Arc<dyn RemoteFs> = match &link {
            Link::Ssh(session) => {
                let sftp = SftpConfig {
                    stream_channels: workers.clamp(1, 8),
                };
                Arc::new(SftpFs::new(session.clone(), sftp).await?)
            }
            Link::Ftp(fs) => Arc::new(fs.clone()),
            #[cfg(test)]
            Link::Fake(_) => self.test_remote(id)?,
        };
        let engine = &self.inner.engine;
        let endpoints = SiteEndpoints {
            local: self.inner.local.clone(),
            remote: remote.clone(),
            workers,
        };
        engine.register_site(site.clone(), endpoints).await;
        if lock(&self.inner.paused_by_close).remove(&site) {
            engine.resume_site(site.clone()).await;
        }
        let watcher = spawn_site_watcher(engine.clone(), site.clone(), link.connection());

        {
            let mut sessions = lock(&self.inner.sessions);
            if let Some(entry) = sessions.get_mut(&id) {
                entry.remote = Some(remote);
                entry.watcher = Some(watcher);
                return Ok(());
            }
        }
        // Closed while this registered the site (`close` may have paused it before the
        // registration and resume above): undo, so the site ends paused. No other session
        // of the site can exist meanwhile: this runs under the site's lock.
        watcher.abort();
        lock(&self.inner.paused_by_close).insert(site.clone());
        engine.pause_site(site).await;
        Err(closed())
    }

    /// Writes the secrets the user typed with "Save in the system keyring" now that the
    /// session connected with them. A failure is logged: the session works anyway, and
    /// the next connect asks again.
    async fn save_typed_secrets(
        &self,
        site: &SiteId,
        typed: Vec<(hoprs_vault::SecretKind, secrecy::SecretString)>,
    ) {
        for (kind, secret) in typed {
            let saved = self
                .inner
                .vault
                .set_secret(site.clone(), kind, secret)
                .await;
            if let Err(err) = saved {
                tracing::warn!(site = %site, secret = kind.account(), error = %err, "cannot save the typed secret in the keyring");
            }
        }
    }

    /// Retries a session whose connection is `Disconnected` (first connection included)
    /// and waits for it like `connect`.
    pub async fn reconnect(&self, id: SessionId) -> Result<()> {
        let (site, link) = {
            let sessions = lock(&self.inner.sessions);
            let entry = sessions.get(&id).ok_or_else(closed)?;
            (entry.site.clone(), entry.link.clone())
        };
        let site_lock = self.site_lock(&site);
        let _turn = site_lock.lock().await;
        link.reconnect_now().await;
        self.ready(id).await
    }

    /// Closes a session: refuses its prompts, aborts its site watcher, pauses its site in
    /// the engine (the next connect of the site resumes it), then disconnects and drops
    /// the connection. Unknown ids are ignored.
    ///
    /// It does not take the site's lock: a connect waiting on this session's prompts or
    /// first connection holds it, and closing is what ends that wait. A `ready` still
    /// running for this session notices the close and pauses the site again itself.
    pub async fn close(&self, id: SessionId) {
        let (entry, replaced) = {
            let mut sessions = lock(&self.inner.sessions);
            let Some(entry) = sessions.remove(&id) else {
                return;
            };
            let replaced = sessions.values().any(|other| other.site == entry.site);
            (entry, replaced)
        };
        entry.prompter.cancel();
        entry.prompter.discard_pending_secrets();
        if let Some(watcher) = &entry.watcher {
            watcher.abort();
        }
        if !replaced {
            lock(&self.inner.paused_by_close).insert(entry.site.clone());
            self.inner.engine.pause_site(entry.site.clone()).await;
        }
        entry.link.disconnect().await;
        drop(entry);
    }

    /// Closes every session (the window is closing).
    pub async fn close_all(&self) {
        let mut ids: Vec<SessionId> = lock(&self.inner.sessions).keys().copied().collect();
        ids.sort();
        futures_util::future::join_all(ids.into_iter().map(|id| self.close(id))).await;
    }

    /// How an SSH session logged in last; `None` for FTP and before the first connection.
    pub fn auth(&self, id: SessionId) -> Option<hoprs_ssh::AuthMethod> {
        match &lock(&self.inner.sessions).get(&id)?.link {
            Link::Ssh(session) => session.auth_method(),
            Link::Ftp(_) => None,
            #[cfg(test)]
            Link::Fake(_) => None,
        }
    }

    /// The remote filesystem of a connected session.
    pub fn remote(&self, id: SessionId) -> Result<Arc<dyn RemoteFs>> {
        let sessions = lock(&self.inner.sessions);
        let entry = sessions.get(&id).ok_or_else(closed)?;
        entry
            .remote
            .clone()
            .ok_or_else(|| Error::new(ErrorKind::Network, "Not connected"))
    }

    /// The `known_hosts` file SSH sessions check: the configured one, else
    /// `~/.ssh/known_hosts`.
    pub fn known_hosts_path(&self) -> PathBuf {
        match &self.inner.config.known_hosts {
            Some(path) => path.clone(),
            None => hoprs_ssh::SshConfig::new("", 22, "").known_hosts,
        }
    }

    /// The local filesystem every session transfers with.
    pub fn local(&self) -> Arc<dyn RemoteFs> {
        self.inner.local.clone()
    }

    /// Watches a session's connection state.
    pub fn state(&self, id: SessionId) -> Option<watch::Receiver<ConnectionState>> {
        lock(&self.inner.sessions)
            .get(&id)
            .map(|entry| entry.link.state())
    }

    fn site_lock(&self, site: &SiteId) -> Arc<tokio::sync::Mutex<()>> {
        lock(&self.inner.site_locks)
            .entry(site.clone())
            .or_default()
            .clone()
    }

    /// Adds a session over a test connection and `remote`, then readies it like `connect`.
    #[cfg(test)]
    async fn attach(
        &self,
        site: SiteId,
        conn: Arc<dyn Connection>,
        remote: Arc<dyn RemoteFs>,
    ) -> Result<SessionId> {
        let id = SessionId(self.inner.next_id.fetch_add(1, Ordering::SeqCst));
        let prompter = self.inner.broker.session(id, site.clone(), "test", 0);
        self.insert(id, site.clone(), Link::Fake(conn), prompter, false);
        lock(&self.inner.test_remotes).insert(id, remote);
        let site_lock = self.site_lock(&site);
        let _turn = site_lock.lock().await;
        self.ready(id).await?;
        Ok(id)
    }

    /// The prompter of a session (tests answer its prompts directly).
    #[cfg(test)]
    fn prompter(&self, id: SessionId) -> Option<SessionPrompter> {
        lock(&self.inner.sessions)
            .get(&id)
            .map(|entry| entry.prompter.clone())
    }

    #[cfg(test)]
    fn test_remote(&self, id: SessionId) -> Result<Arc<dyn RemoteFs>> {
        lock(&self.inner.test_remotes)
            .remove(&id)
            .ok_or_else(closed)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use hoprs_core::{EntryKind, RemotePath};
    use hoprs_remote::MemFs;
    use hoprs_transfer::{EngineConfig, QueueStore};
    use hoprs_vault::{MemoryStore, SecretKind, Vault};
    use secrecy::SecretString;

    use super::super::broker::{BrokerEvent, PromptAnswer, PromptKind};
    use super::*;

    struct FakeConn {
        state: watch::Sender<ConnectionState>,
    }

    impl Connection for FakeConn {
        fn state(&self) -> watch::Receiver<ConnectionState> {
            self.state.subscribe()
        }

        fn generation(&self) -> u64 {
            1
        }
    }

    struct Fixture {
        manager: SessionManager,
        broker: PromptBroker,
        vault: VaultHandle,
        dir: tempfile::TempDir,
    }

    fn fixture() -> Fixture {
        fixture_with(Arc::new(MemoryStore::new()))
    }

    fn fixture_with(store: Arc<dyn hoprs_vault::SecretStore>) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::open(dir.path(), store).unwrap();
        let vault = VaultHandle::new(vault);
        let broker = PromptBroker::new(vault.clone());
        let engine = TransferEngine::start(
            QueueStore::open_in_memory().unwrap(),
            EngineConfig::default(),
        )
        .unwrap();
        let config = SessionConfig {
            known_hosts: Some(dir.path().join("known_hosts")),
            workers: 2,
        };
        let manager = SessionManager::new(engine, vault.clone(), broker.clone(), config);
        Fixture {
            manager,
            broker,
            vault,
            dir,
        }
    }

    async fn eventually(what: &str, mut check: impl FnMut() -> bool) {
        for _ in 0..200 {
            if check() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("timed out waiting for {what}");
    }

    #[tokio::test]
    async fn close_aborts_watcher() {
        let f = fixture();
        let (state, _) = watch::channel(ConnectionState::Connected);
        let conn = Arc::new(FakeConn { state });
        let remote = MemFs::new();
        remote.add_dir("/srv");
        let site = SiteId::new();
        let id = f
            .manager
            .attach(site.clone(), conn.clone(), Arc::new(remote))
            .await
            .unwrap();
        let listed = f
            .manager
            .remote(id)
            .unwrap()
            .list(&RemotePath::root())
            .await
            .unwrap();
        assert_eq!(listed[0].kind, EntryKind::Dir);
        // The session and its watcher hold the connection.
        assert!(
            Arc::strong_count(&conn) >= 3,
            "{}",
            Arc::strong_count(&conn)
        );
        assert!(f.manager.state(id).is_some());

        f.manager.close(id).await;
        eventually("the watcher to drop the connection", || {
            Arc::strong_count(&conn) == 1
        })
        .await;
        assert!(f.manager.remote(id).is_err());
        assert!(f.manager.state(id).is_none());
        // Closing twice is harmless.
        f.manager.close(id).await;
    }

    /// The window is closing: every session goes, its watch with it.
    #[tokio::test]
    async fn close_all_closes_every_session() {
        let f = fixture();
        let mut ids = Vec::new();
        let mut conns = Vec::new();
        for _ in 0..2 {
            let (state, _) = watch::channel(ConnectionState::Connected);
            let conn = Arc::new(FakeConn { state });
            let id = f
                .manager
                .attach(SiteId::new(), conn.clone(), Arc::new(MemFs::new()))
                .await
                .unwrap();
            ids.push(id);
            conns.push(conn);
        }
        assert!(ids.iter().all(|id| f.manager.state(*id).is_some()));
        // Not an SSH session: no login method to tell.
        assert_eq!(f.manager.auth(ids[0]), None);
        f.manager.close_all().await;
        assert!(ids.iter().all(|id| f.manager.state(*id).is_none()));
        assert!(f.manager.remote(ids[1]).is_err());
        for conn in conns {
            eventually("the watcher to drop the connection", || {
                Arc::strong_count(&conn) == 1
            })
            .await;
        }
        assert_eq!(f.manager.auth(SessionId(999)), None);
    }

    #[tokio::test]
    async fn session_ids_are_monotonic() {
        let f = fixture();
        let mut ids = Vec::new();
        for _ in 0..3 {
            let (state, _) = watch::channel(ConnectionState::Connected);
            let conn = Arc::new(FakeConn { state });
            let id = f
                .manager
                .attach(SiteId::new(), conn, Arc::new(MemFs::new()))
                .await
                .unwrap();
            ids.push(id);
        }
        f.manager.close(ids[1]).await;
        let (state, _) = watch::channel(ConnectionState::Connected);
        let conn = Arc::new(FakeConn { state });
        let last = f
            .manager
            .attach(SiteId::new(), conn, Arc::new(MemFs::new()))
            .await
            .unwrap();
        ids.push(last);
        assert!(ids.windows(2).all(|w| w[0] < w[1]), "{ids:?}");
    }

    #[tokio::test]
    async fn connect_unknown_site_is_not_found() {
        let f = fixture();
        let err = f.manager.connect(SiteId::new()).await.unwrap_err();
        assert_eq!(err.session, None);
        assert_eq!(err.error.kind, ErrorKind::NotFound);

        let missing = SiteId::new();
        let results = f.manager.connect_many(vec![missing.clone()]).await;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, missing);
        assert_eq!(
            results[0].1.as_ref().unwrap_err().error.kind,
            ErrorKind::NotFound
        );
    }

    /// A failed first connection keeps the session: its id comes with the error, its
    /// state shows the failure, and `close` drops it.
    #[tokio::test]
    async fn failed_first_connect_keeps_the_session() {
        let f = fixture();
        // Nothing listens on port 1.
        let mut site = Site::new("down", "127.0.0.1", "u", Protocol::Ftp);
        site.port = 1;
        f.vault.upsert_site(site.clone()).await.unwrap();
        let mut events = f.broker.take_events().unwrap();
        let answer = tokio::spawn({
            let broker = f.broker.clone();
            async move {
                while let Some(event) = events.recv().await {
                    if let BrokerEvent::Request(r) = event {
                        broker.answer(
                            r.id,
                            PromptAnswer::Secret {
                                secret: SecretString::from("pw"),
                                save: false,
                            },
                        );
                    }
                }
            }
        });

        let err = f.manager.connect(site.id.clone()).await.unwrap_err();
        let id = err.session.expect("the session is kept");
        assert_eq!(err.error.kind, ErrorKind::Network, "{:?}", err.error);
        assert!(matches!(
            *f.manager.state(id).unwrap().borrow(),
            ConnectionState::Disconnected(_)
        ));
        assert!(f.manager.remote(id).is_err());
        // Connecting the site again retries the same session.
        let again = f.manager.connect(site.id.clone()).await.unwrap_err();
        assert_eq!(again.session, Some(id));
        assert!(f.manager.reconnect(id).await.is_err());
        f.manager.close(id).await;
        assert!(f.manager.state(id).is_none());
        answer.abort();
    }

    /// Answers every dialog with the password `pw` (and refuses the rest).
    fn answer_passwords(broker: &PromptBroker) -> (JoinHandle<()>, Arc<Mutex<Vec<PromptKind>>>) {
        let mut events = broker.take_events().unwrap();
        let broker = broker.clone();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let task = tokio::spawn({
            let seen = seen.clone();
            async move {
                while let Some(event) = events.recv().await {
                    if let BrokerEvent::Request(r) = event {
                        seen.lock().unwrap().push(r.kind);
                        broker.answer(
                            r.id,
                            PromptAnswer::Secret {
                                secret: SecretString::from("pw"),
                                save: false,
                            },
                        );
                    }
                }
            }
        });
        (task, seen)
    }

    /// A locked keyring is reported with the connect result, error path included, and
    /// the stored password falls back to the prompt. A site with no stored secret is not
    /// marked.
    #[tokio::test]
    async fn keyring_unavailable_reaches_the_caller() {
        let store = Arc::new(super::super::vault::tests::LockedStore::default());
        let f = fixture_with(store.clone());
        let mut locked = Site::new("locked", "127.0.0.1", "u", Protocol::Ftp);
        locked.port = 1;
        let mut plain = Site::new("plain", "127.0.0.1", "u", Protocol::Ftp);
        plain.port = 1;
        f.vault.upsert_site(locked.clone()).await.unwrap();
        f.vault.upsert_site(plain.clone()).await.unwrap();
        let id = locked.id.clone();
        f.vault
            .call(move |v| v.set_secret(&id, SecretKind::Password, &SecretString::from("x")))
            .await
            .unwrap();
        let (answer, seen) = answer_passwords(&f.broker);

        let results = f
            .manager
            .connect_many(vec![locked.id.clone(), plain.id.clone()])
            .await;
        let locked_err = results[0].1.as_ref().unwrap_err();
        assert!(locked_err.keyring_unavailable, "{locked_err:?}");
        assert!(locked_err.session.is_some());
        let plain_err = results[1].1.as_ref().unwrap_err();
        assert!(!plain_err.keyring_unavailable, "{plain_err:?}");
        // Both asked for the password instead; the keyring was tried once.
        let passwords = seen
            .lock()
            .unwrap()
            .iter()
            .filter(|k| matches!(k, PromptKind::Password { .. }))
            .count();
        assert_eq!(passwords, 2);
        assert_eq!(store.gets.load(std::sync::atomic::Ordering::SeqCst), 1);

        // Connecting the kept session again reports the flag it was started with.
        let again = f.manager.connect(locked.id.clone()).await.unwrap_err();
        assert_eq!(again.session, locked_err.session);
        assert!(again.keyring_unavailable);
        answer.abort();
    }

    /// Open workspace: every site's result is sent on its own, a missing site included,
    /// and a locked keyring is asked once for the whole batch.
    #[tokio::test]
    async fn connect_batch_sends_each_result_with_one_secret_batch() {
        let store = Arc::new(super::super::vault::tests::LockedStore::default());
        let f = fixture_with(store.clone());
        let mut ids = Vec::new();
        for name in ["a", "b", "c"] {
            let mut site = Site::new(name, "127.0.0.1", "u", Protocol::Ftp);
            site.port = 1;
            f.vault.upsert_site(site.clone()).await.unwrap();
            let id = site.id.clone();
            f.vault
                .call(move |v| v.set_secret(&id, SecretKind::Password, &SecretString::from("x")))
                .await
                .unwrap();
            ids.push(site.id);
        }
        let missing = SiteId::new();
        ids.insert(1, missing.clone());
        let (answer, _) = answer_passwords(&f.broker);

        let (tx, mut rx) = mpsc::unbounded_channel();
        f.manager.connect_batch(ids.clone(), tx).await;
        let mut got = Vec::new();
        while let Some(result) = rx.recv().await {
            got.push(result);
        }
        assert_eq!(got.len(), 4);
        let mut sites: Vec<_> = got.iter().map(|(id, _)| id.clone()).collect();
        sites.sort_by(|a, b| a.0.cmp(&b.0));
        let mut expected = ids.clone();
        expected.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(sites, expected, "one result per site");
        for (id, result) in &got {
            let err = result.as_ref().unwrap_err();
            if *id == missing {
                assert_eq!(err.error.kind, ErrorKind::NotFound);
            } else {
                assert!(err.keyring_unavailable, "{err:?}");
            }
        }
        assert_eq!(
            store.gets.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "one keyring call for the whole batch"
        );
        answer.abort();
    }

    /// `close` pauses the site in the engine (nothing new is dispatched to the dead
    /// connection) and the next connect of the site resumes it.
    #[tokio::test]
    async fn close_pauses_the_site_and_reconnecting_resumes_it() {
        use hoprs_transfer::{Direction, TransferSpec, TransferState};

        let f = fixture();
        let site = SiteId::new();
        let remote = MemFs::new();
        remote.add_file("/f.txt", b"data", std::time::SystemTime::UNIX_EPOCH);
        let local = tempfile::tempdir().unwrap();
        let local_path = RemotePath::new(&format!(
            "{}/f.txt",
            local.path().to_string_lossy().replace('\\', "/")
        ));
        let Ok(local_path) = local_path else {
            return; // A drive-letter path: LocalFs("/") cannot address it (Windows).
        };
        let (state, _) = watch::channel(ConnectionState::Connected);
        let conn: Arc<dyn Connection> = Arc::new(FakeConn { state });
        let id = f
            .manager
            .attach(site.clone(), conn.clone(), Arc::new(remote.clone()))
            .await
            .unwrap();
        f.manager.close(id).await;

        let engine = f.manager.inner.engine.clone();
        let item = engine
            .enqueue(TransferSpec {
                site: site.clone(),
                direction: Direction::Download,
                local: local_path,
                remote: RemotePath::new("/f.txt").unwrap(),
            })
            .await
            .unwrap();
        let state_of = |items: Vec<hoprs_transfer::TransferItem>| {
            items.into_iter().find(|i| i.id == item).unwrap().state
        };
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(state_of(engine.snapshot().await), TransferState::Queued);

        f.manager
            .attach(site, conn, Arc::new(remote))
            .await
            .unwrap();
        for _ in 0..200 {
            if state_of(engine.snapshot().await) == TransferState::Done {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(state_of(engine.snapshot().await), TransferState::Done);
        assert_eq!(std::fs::read(local.path().join("f.txt")).unwrap(), b"data");
    }

    /// "Save in the system keyring": the typed password reaches the keyring only once the
    /// session it was typed for connects *with it* (auth reported it accepted); a session
    /// that got in another way, a failed connection or a close drops it.
    #[tokio::test]
    async fn typed_secret_saved_only_after_connect() {
        use hoprs_ssh::Prompter as _;
        use hoprs_vault::SecretStore as _;
        use secrecy::ExposeSecret;

        let store = Arc::new(MemoryStore::new());
        let f = fixture_with(store.clone());
        let mut events = f.broker.take_events().unwrap();
        let stored = |site: &Site| {
            store
                .get(&site.id, SecretKind::Password)
                .unwrap()
                .map(|s| s.expose_secret().to_owned())
        };

        // Each case: a session still connecting, whose password prompt is answered with
        // "save"; then the connection succeeds, fails, or the session is closed.
        for (n, outcome) in ["connects", "another method", "fails", "closes"]
            .into_iter()
            .enumerate()
        {
            let site = Site::new(outcome, "h", "u", Protocol::Ssh);
            f.vault.upsert_site(site.clone()).await.unwrap();
            let (state, _) = watch::channel(ConnectionState::Connecting);
            let conn = Arc::new(FakeConn { state });
            let attach = tokio::spawn({
                let manager = f.manager.clone();
                let site = site.id.clone();
                let conn = conn.clone();
                async move { manager.attach(site, conn, Arc::new(MemFs::new())).await }
            });
            let id = SessionId(n as u64 + 1);
            eventually("the session", || f.manager.prompter(id).is_some()).await;
            let prompter = f.manager.prompter(id).unwrap();
            let ask = tokio::spawn({
                let prompter = prompter.clone();
                async move { prompter.password("u", "h").await }
            });
            let Some(BrokerEvent::Request(r)) = events.recv().await else {
                panic!("a password prompt");
            };
            f.broker.answer(
                r.id,
                PromptAnswer::Secret {
                    secret: SecretString::from(format!("pw-{outcome}")),
                    save: true,
                },
            );
            ask.await.unwrap().unwrap();
            assert_eq!(stored(&site), None, "{outcome}: not before the outcome");

            match outcome {
                "connects" => {
                    // Auth reports the typed password right, then the state follows.
                    prompter.secret_accepted(hoprs_ssh::AcceptedSecret::Password);
                    conn.state.send_replace(ConnectionState::Connected);
                    attach.await.unwrap().unwrap();
                    assert_eq!(stored(&site), Some("pw-connects".into()));
                    assert!(
                        f.vault
                            .site(site.id.clone())
                            .await
                            .unwrap()
                            .unwrap()
                            .has_password
                    );
                }
                "another method" => {
                    // The server refused the password; keyboard-interactive (or a key)
                    // got in: the session connects, the typed password is not saved.
                    conn.state.send_replace(ConnectionState::Connected);
                    attach.await.unwrap().unwrap();
                    assert_eq!(stored(&site), None);
                    assert!(
                        !f.vault
                            .site(site.id.clone())
                            .await
                            .unwrap()
                            .unwrap()
                            .has_password
                    );
                }
                "fails" => {
                    let error = Error::new(ErrorKind::Auth, "Denied");
                    conn.state
                        .send_replace(ConnectionState::Disconnected(error));
                    assert!(attach.await.unwrap().is_err());
                    assert_eq!(stored(&site), None);
                    assert!(prompter.take_pending_secrets().is_empty(), "dropped");
                }
                _ => {
                    f.manager.close(id).await;
                    assert!(prompter.take_pending_secrets().is_empty(), "dropped");
                    // The close ends the wait: nothing is saved even if it connects now.
                    conn.state.send_replace(ConnectionState::Connected);
                    assert!(attach.await.unwrap().is_err());
                    assert_eq!(stored(&site), None);
                }
            }
        }
    }

    /// Answers every dialog: trusts host keys and certificates, refuses the rest. Returns
    /// the task and the kinds of the dialogs it saw.
    fn answer_prompts(broker: &PromptBroker) -> (JoinHandle<()>, Arc<Mutex<Vec<PromptKind>>>) {
        let mut events = broker.take_events().unwrap();
        let broker = broker.clone();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let task = tokio::spawn({
            let seen = seen.clone();
            async move {
                while let Some(event) = events.recv().await {
                    if let BrokerEvent::Request(r) = event {
                        let answer = match r.kind {
                            PromptKind::HostKey { .. } | PromptKind::Certificate { .. } => {
                                PromptAnswer::Decision(true)
                            }
                            _ => PromptAnswer::Cancel,
                        };
                        seen.lock().unwrap().push(r.kind);
                        broker.answer(r.id, answer);
                    }
                }
            }
        });
        (task, seen)
    }

    #[tokio::test]
    #[ignore = "needs docker: docker compose -f docker/compose.yml up -d --build --wait"]
    async fn connect_ssh_site_end_to_end() {
        let f = fixture();
        let mut site = Site::new("docker ssh", "127.0.0.1", "alice", Protocol::Ssh);
        site.port = hoprs_ssh::testing::SSH_PORT;
        site.ssh.use_agent = false;
        site.ssh.key_file = Some(hoprs_ssh::testing::fixture("id_ed25519"));
        f.vault.upsert_site(site.clone()).await.unwrap();
        let (prompts, seen) = answer_prompts(&f.broker);

        let connected = f.manager.connect(site.id.clone()).await.unwrap();
        assert!(!connected.keyring_unavailable);
        let id = connected.session;
        let entries = f
            .manager
            .remote(id)
            .unwrap()
            .list(&RemotePath::root())
            .await
            .unwrap();
        assert!(
            entries
                .iter()
                .any(|e| e.name == "home" && e.kind == EntryKind::Dir),
            "{entries:?}"
        );
        // The host key was asked once and recorded in the test's known_hosts.
        let seen = seen.lock().unwrap().clone();
        assert_eq!(seen.len(), 1, "{seen:?}");
        assert!(matches!(seen[0], PromptKind::HostKey { .. }), "{seen:?}");
        assert!(f.dir.path().join("known_hosts").exists());
        // Connecting again returns the same session.
        let again = f.manager.connect(site.id.clone()).await.unwrap();
        assert_eq!(again.session, id);
        // The status bar's login method.
        assert_eq!(
            f.manager.auth(id),
            Some(hoprs_ssh::AuthMethod::KeyFile {
                key_type: "ed25519".into()
            })
        );

        let state = f.manager.state(id).unwrap();
        f.manager.close(id).await;
        assert!(matches!(*state.borrow(), ConnectionState::Disconnected(_)));
        prompts.abort();
    }

    #[tokio::test]
    #[ignore = "needs docker: docker compose -f docker/compose.yml up -d --build --wait"]
    async fn connect_ftps_site_end_to_end() {
        let f = fixture();
        let mut site = Site::new("docker ftps", "127.0.0.1", "frank", Protocol::FtpsImplicit);
        site.port = hoprs_remote::testing::FTPS_IMPLICIT_PORT;
        f.vault.upsert_site(site.clone()).await.unwrap();
        let id_for_secret = site.id.clone();
        f.vault
            .call(move |v| {
                v.set_secret(
                    &id_for_secret,
                    SecretKind::Password,
                    &SecretString::from("frank-pass"),
                )
            })
            .await
            .unwrap();
        let (prompts, seen) = answer_prompts(&f.broker);

        let connected = f.manager.connect(site.id.clone()).await.unwrap();
        assert!(!connected.keyring_unavailable);
        let id = connected.session;
        let entries = f
            .manager
            .remote(id)
            .unwrap()
            .list(&RemotePath::root())
            .await
            .unwrap();
        assert!(!entries.is_empty());
        // One certificate dialog, no password dialog (the password came from the vault).
        let seen = seen.lock().unwrap().clone();
        assert_eq!(seen.len(), 1, "{seen:?}");
        assert!(
            matches!(seen[0], PromptKind::Certificate { .. }),
            "{seen:?}"
        );
        // The accepted certificate was pinned in the vault through the broker.
        let saved = f.vault.site(site.id.clone()).await.unwrap().unwrap();
        assert_eq!(
            saved.ftp.pinned_fingerprint,
            Some(hoprs_remote::testing::server_fingerprint())
        );

        // FTP has no login method of its own: the status bar names the protocol.
        assert_eq!(f.manager.auth(id), None);
        f.manager.close(id).await;
        prompts.abort();
    }
}
