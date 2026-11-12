//! The backend the effects run against: the vault, the transfer engine, the session
//! manager and the prompt broker, behind [`BackendApi`].

// The effects that call most of this arrive in Tasks 5–11; the tests use all of it.
#![cfg_attr(not(test), allow(dead_code))]

pub mod broker;
#[cfg(test)]
pub mod fake;
pub mod mapping;
pub mod ops;
pub mod sessions;
pub mod vault;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use hoprs_core::{Error, RemoteEntry, RemotePath, Result, Site, SiteId, Workspace, WorkspaceId};
use hoprs_remote::RemoteFs;
use hoprs_transfer::{
    EngineConfig, QueueStore, Resolution, TransferEngine, TransferId, TransferSpec,
};
use hoprs_vault::{SecretStore, Vault};
use secrecy::SecretString;
use tokio::sync::mpsc;

pub use broker::{BrokerEvent, PromptAnswer, PromptBroker, PromptId};
pub use ops::DeleteTarget;
pub use sessions::{
    ConnectError, ConnectResult, Connected, SessionConfig, SessionId, SessionManager,
};
pub use vault::{LazyKeyringStore, ProfileSnapshot, VaultHandle};

/// Which pane of a files tab.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    Local,
    Remote,
}

/// What to do with a transfer item.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ControlOp {
    Pause,
    Resume,
    Cancel,
    Retry,
    Resolve(Resolution),
}

/// What `effects::run` may call. Each later task adds the methods its effects need.
#[async_trait]
pub trait BackendApi: Send + Sync {
    async fn sites(&self) -> Result<Vec<Site>>;
    async fn workspaces(&self) -> Result<Vec<Workspace>>;
    /// Sites, workspaces and unreadable entries, read together.
    async fn profiles(&self) -> Result<ProfileSnapshot>;
    /// The directory holding `sites.toml` ("Open sites.toml folder").
    fn profile_dir(&self) -> PathBuf;
    /// Saves a site edited in the UI. Fields the backend owns are kept from the stored
    /// copy: the FTPS pin and `has_password`/`has_key_passphrase` (see
    /// [`VaultHandle::save_site`]).
    async fn save_site(&self, site: Site) -> Result<()>;
    /// Stores (`Some`) or deletes (`None`) the site's password in the keyring, which also
    /// sets or clears its `has_password`.
    async fn set_password(&self, site: SiteId, password: Option<SecretString>) -> Result<()>;
    /// Also removes the site from every workspace and deletes its secrets.
    async fn delete_site(&self, id: SiteId) -> Result<()>;
    async fn save_workspace(&self, workspace: Workspace) -> Result<()>;
    async fn delete_workspace(&self, id: WorkspaceId) -> Result<()>;

    /// Connects a saved site; see [`SessionManager::connect`].
    async fn connect(&self, site: SiteId) -> std::result::Result<Connected, ConnectError>;
    /// Open workspace: connects `sites` in parallel, reading their secrets as one batch
    /// (at most one keyring unlock prompt), and sends each site's result on `results` as
    /// soon as it is known. Returns when every site is done; see
    /// [`SessionManager::connect_batch`].
    async fn connect_batch(
        &self,
        sites: Vec<SiteId>,
        results: mpsc::UnboundedSender<(SiteId, ConnectResult)>,
    );
    /// Retries a `Disconnected` session; see [`SessionManager::reconnect`].
    async fn reconnect(&self, session: SessionId) -> Result<()>;
    async fn close(&self, session: SessionId);

    /// Lists `path` on the local disk, or on the remote side of `session` (an error
    /// without one).
    async fn list(
        &self,
        session: Option<SessionId>,
        side: Side,
        path: RemotePath,
    ) -> Result<Vec<RemoteEntry>>;
    /// Creates the folder `path` (its parent must exist) on the side's filesystem.
    async fn mkdir(&self, session: Option<SessionId>, side: Side, path: RemotePath) -> Result<()>;
    /// Renames `from` to `to`; fails with `Conflict` if something is already at `to`
    /// (see [`ops::rename_no_replace`]).
    async fn rename(
        &self,
        session: Option<SessionId>,
        side: Side,
        from: RemotePath,
        to: RemotePath,
    ) -> Result<()>;
    /// Deletes `targets`, folders with everything in them (see [`ops::delete_all`]),
    /// sending the running count of removed items on `progress`. Returns the count.
    async fn delete(
        &self,
        session: Option<SessionId>,
        side: Side,
        targets: Vec<DeleteTarget>,
        progress: mpsc::UnboundedSender<u64>,
    ) -> Result<u64>;
    async fn chmod(
        &self,
        session: Option<SessionId>,
        side: Side,
        path: RemotePath,
        mode: u32,
    ) -> Result<()>;

    async fn enqueue(&self, spec: TransferSpec) -> Result<TransferId>;
    async fn control(&self, id: TransferId, op: ControlOp) -> Result<()>;

    /// Delivers the UI's answer to a dialog (closing it is [`PromptAnswer::Cancel`]).
    async fn answer_prompt(&self, id: PromptId, answer: PromptAnswer);
    /// The dialogs to show, for the UI's subscription. Only the first call gets it.
    fn prompt_events(&self) -> Option<mpsc::UnboundedReceiver<BrokerEvent>>;
}

/// A remote listing asked without a session.
fn not_connected() -> Error {
    Error::new(hoprs_core::ErrorKind::NotFound, "Not connected")
}

/// Where the backend keeps its files.
#[derive(Clone, Debug)]
pub struct BackendConfig {
    /// `sites.toml` and `workspaces.toml` (see [`hoprs_vault::default_dir`]).
    pub profile_dir: PathBuf,
    /// The persistent transfer queue; `None` keeps it in memory.
    pub queue_path: Option<PathBuf>,
    pub sessions: SessionConfig,
}

impl BackendConfig {
    /// The standard locations: profiles in `~/.config/hoprs`, the queue in
    /// `$XDG_DATA_HOME/hoprs/queue.db` (spec §5). `None` without a config directory.
    pub fn standard() -> Option<Self> {
        Some(Self {
            profile_dir: hoprs_vault::default_dir()?,
            queue_path: dirs::data_dir().map(|dir| dir.join("hoprs").join("queue.db")),
            sessions: SessionConfig::default(),
        })
    }
}

/// The real backend.
pub struct Backend {
    profile_dir: PathBuf,
    vault: VaultHandle,
    engine: TransferEngine,
    sessions: SessionManager,
    broker: PromptBroker,
}

impl Backend {
    /// Opens the vault and the queue and starts the engine. Must run inside the tokio
    /// runtime (iced's `boot` does). It reads the two small profile files and opens the
    /// queue database synchronously, once, before the first frame; the keyring is only
    /// reached later, on the blocking pool ([`LazyKeyringStore`]).
    ///
    /// A queue that cannot be opened or read is logged and replaced by an in-memory one,
    /// so the app still starts (the old file is left untouched).
    pub fn open(config: BackendConfig, secrets: Arc<dyn SecretStore>) -> Result<Backend> {
        let vault = VaultHandle::new(Vault::open(&config.profile_dir, secrets)?);
        let engine = start_engine(config.queue_path.as_deref());
        let broker = PromptBroker::new(vault.clone());
        let sessions = SessionManager::new(
            engine.clone(),
            vault.clone(),
            broker.clone(),
            config.sessions,
        );
        Ok(Backend {
            profile_dir: config.profile_dir,
            vault,
            engine,
            sessions,
            broker,
        })
    }
}

impl Backend {
    /// The filesystem of one side: the local disk, or the remote side of `session` (an
    /// error without one).
    fn fs(&self, session: Option<SessionId>, side: Side) -> Result<Arc<dyn RemoteFs>> {
        match (side, session) {
            (Side::Local, _) => Ok(self.sessions.local()),
            (Side::Remote, Some(session)) => self.sessions.remote(session),
            (Side::Remote, None) => Err(not_connected()),
        }
    }
}

/// The engine over the queue at `path`, or over an in-memory queue if that fails.
fn start_engine(path: Option<&Path>) -> TransferEngine {
    let persistent = path.map(|path| {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(Error::from)?;
        }
        TransferEngine::start(QueueStore::open(path)?, EngineConfig::default())
    });
    match persistent {
        Some(Ok(engine)) => return engine,
        Some(Err(err)) => {
            tracing::error!(error = %err, "cannot open the transfer queue; this run keeps it in memory");
        }
        None => {}
    }
    let store = QueueStore::open_in_memory().expect("an in-memory queue always opens");
    TransferEngine::start(store, EngineConfig::default()).expect("an empty queue always starts")
}

#[async_trait]
impl BackendApi for Backend {
    async fn sites(&self) -> Result<Vec<Site>> {
        self.vault.sites().await
    }

    async fn workspaces(&self) -> Result<Vec<Workspace>> {
        self.vault.workspaces().await
    }

    async fn profiles(&self) -> Result<ProfileSnapshot> {
        self.vault.snapshot().await
    }

    fn profile_dir(&self) -> PathBuf {
        self.profile_dir.clone()
    }

    async fn save_site(&self, site: Site) -> Result<()> {
        self.vault.save_site(site).await
    }

    async fn set_password(&self, site: SiteId, password: Option<SecretString>) -> Result<()> {
        self.vault.set_password(site, password).await
    }

    async fn delete_site(&self, id: SiteId) -> Result<()> {
        self.vault.remove_site(id).await
    }

    async fn save_workspace(&self, workspace: Workspace) -> Result<()> {
        self.vault.upsert_workspace(workspace).await
    }

    async fn delete_workspace(&self, id: WorkspaceId) -> Result<()> {
        self.vault.remove_workspace(id).await
    }

    async fn connect(&self, site: SiteId) -> std::result::Result<Connected, ConnectError> {
        self.sessions.connect(site).await
    }

    async fn connect_batch(
        &self,
        sites: Vec<SiteId>,
        results: mpsc::UnboundedSender<(SiteId, ConnectResult)>,
    ) {
        self.sessions.connect_batch(sites, results).await;
    }

    async fn reconnect(&self, session: SessionId) -> Result<()> {
        self.sessions.reconnect(session).await
    }

    async fn close(&self, session: SessionId) {
        self.sessions.close(session).await;
    }

    async fn list(
        &self,
        session: Option<SessionId>,
        side: Side,
        path: RemotePath,
    ) -> Result<Vec<RemoteEntry>> {
        self.fs(session, side)?.list(&path).await
    }

    async fn mkdir(&self, session: Option<SessionId>, side: Side, path: RemotePath) -> Result<()> {
        self.fs(session, side)?.mkdir(&path).await
    }

    async fn rename(
        &self,
        session: Option<SessionId>,
        side: Side,
        from: RemotePath,
        to: RemotePath,
    ) -> Result<()> {
        let fs = self.fs(session, side)?;
        ops::rename_no_replace(fs.as_ref(), &from, &to).await
    }

    async fn delete(
        &self,
        session: Option<SessionId>,
        side: Side,
        targets: Vec<DeleteTarget>,
        progress: mpsc::UnboundedSender<u64>,
    ) -> Result<u64> {
        let fs = self.fs(session, side)?;
        ops::delete_all(fs.as_ref(), targets, &progress).await
    }

    async fn chmod(
        &self,
        session: Option<SessionId>,
        side: Side,
        path: RemotePath,
        mode: u32,
    ) -> Result<()> {
        self.fs(session, side)?.chmod(&path, mode).await
    }

    async fn enqueue(&self, spec: TransferSpec) -> Result<TransferId> {
        self.engine.enqueue(spec).await
    }

    async fn control(&self, id: TransferId, op: ControlOp) -> Result<()> {
        match op {
            ControlOp::Pause => self.engine.pause(id).await,
            ControlOp::Resume => self.engine.resume(id).await,
            ControlOp::Cancel => self.engine.cancel(id).await,
            ControlOp::Retry => self.engine.retry(id).await,
            ControlOp::Resolve(resolution) => self.engine.resolve(id, resolution).await,
        }
    }

    async fn answer_prompt(&self, id: PromptId, answer: PromptAnswer) {
        self.broker.answer(id, answer);
    }

    fn prompt_events(&self) -> Option<mpsc::UnboundedReceiver<BrokerEvent>> {
        self.broker.take_events()
    }
}

#[cfg(test)]
mod tests {
    use hoprs_core::{ErrorKind, Protocol, WorkspaceSite};
    use hoprs_transfer::{Direction, ResolveAction};
    use hoprs_vault::MemoryStore;

    use super::*;

    fn open(dir: &Path) -> Backend {
        let config = BackendConfig {
            profile_dir: dir.join("profiles"),
            queue_path: Some(dir.join("data").join("queue.db")),
            sessions: SessionConfig::default(),
        };
        Backend::open(config, Arc::new(MemoryStore::new())).unwrap()
    }

    #[tokio::test]
    async fn backend_saves_lists_and_queues() {
        let dir = tempfile::tempdir().unwrap();
        let backend = open(dir.path());
        assert!(dir.path().join("data").join("queue.db").exists());

        let site = Site::new("s", "h", "u", Protocol::Ssh);
        backend.save_site(site.clone()).await.unwrap();
        let workspace = Workspace {
            id: WorkspaceId::new(),
            name: "w".into(),
            sites: vec![WorkspaceSite {
                site: site.id.clone(),
                tabs: Vec::new(),
            }],
        };
        backend.save_workspace(workspace.clone()).await.unwrap();
        assert_eq!(backend.sites().await.unwrap(), vec![site.clone()]);
        assert_eq!(backend.workspaces().await.unwrap(), vec![workspace.clone()]);

        // Open workspace with a site that no longer exists: it fails alone, inline.
        let gone = SiteId::new();
        let (tx, mut rx) = mpsc::unbounded_channel();
        backend.connect_batch(vec![gone.clone()], tx).await;
        let (site_id, result) = rx.recv().await.unwrap();
        assert_eq!(site_id, gone);
        assert_eq!(result.unwrap_err().error.kind, ErrorKind::NotFound);
        assert!(rx.recv().await.is_none(), "one result per site");
        assert_eq!(backend.profile_dir(), dir.path().join("profiles"));
        assert_eq!(backend.profiles().await.unwrap().sites, vec![site.clone()]);
        assert_eq!(
            backend.reconnect(SessionId(99)).await.unwrap_err().kind,
            ErrorKind::NotFound
        );
        backend.close(SessionId(99)).await;

        backend.delete_workspace(workspace.id).await.unwrap();
        backend.delete_site(site.id.clone()).await.unwrap();
        assert!(backend.sites().await.unwrap().is_empty());
        assert!(backend.workspaces().await.unwrap().is_empty());

        // The remote side of an unknown session is an error, not a panic.
        let err = backend
            .list(Some(SessionId(0)), Side::Remote, RemotePath::root())
            .await
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::NotFound);
        let err = backend
            .list(None, Side::Remote, RemotePath::root())
            .await
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::NotFound);

        // Transfers reach the engine; every control is routed to it.
        let id = backend
            .enqueue(TransferSpec {
                site: site.id,
                direction: Direction::Download,
                local: RemotePath::new("/local-x").unwrap(),
                remote: RemotePath::new("/r").unwrap(),
            })
            .await
            .unwrap();
        backend.control(id, ControlOp::Pause).await.unwrap();
        backend.control(id, ControlOp::Resume).await.unwrap();
        backend.control(id, ControlOp::Cancel).await.unwrap();
        backend.control(id, ControlOp::Retry).await.unwrap();
        let resolve = ControlOp::Resolve(Resolution {
            action: ResolveAction::Skip,
            apply_to_all: false,
        });
        // Nothing awaits a decision: the engine refuses, and the error comes back.
        assert!(backend.control(id, resolve).await.is_err());
        assert!(
            backend
                .control(TransferId(-1), ControlOp::Pause)
                .await
                .is_err()
        );

        assert!(backend.prompt_events().is_some());
        assert!(backend.prompt_events().is_none());
        backend
            .answer_prompt(PromptId(1), PromptAnswer::Cancel)
            .await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn local_side_needs_no_session() {
        let dir = tempfile::tempdir().unwrap();
        let backend = open(dir.path());
        let local = tempfile::tempdir().unwrap();
        std::fs::write(local.path().join("a.txt"), b"x").unwrap();
        let path = RemotePath::new(local.path().to_str().unwrap()).unwrap();
        let listed = backend.list(None, Side::Local, path).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "a.txt");
    }

    /// The file actions reach the local disk without a session; the remote side needs
    /// one.
    #[cfg(unix)]
    #[tokio::test]
    async fn local_file_actions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let backend = open(dir.path());
        let local = tempfile::tempdir().unwrap();
        let root = RemotePath::new(local.path().to_str().unwrap()).unwrap();
        let disk = |name: &str| local.path().join(name);

        backend
            .mkdir(None, Side::Local, root.join("new"))
            .await
            .unwrap();
        assert!(disk("new").is_dir());
        std::fs::write(disk("new").join("a.txt"), b"a").unwrap();
        backend
            .rename(None, Side::Local, root.join("new"), root.join("old"))
            .await
            .unwrap();
        assert!(disk("old").join("a.txt").is_file());
        backend
            .chmod(None, Side::Local, root.join("old").join("a.txt"), 0o600)
            .await
            .unwrap();
        let mode = std::fs::metadata(disk("old").join("a.txt"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);

        // A link to a folder is removed as a link; its target stays.
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("keep.txt"), b"k").unwrap();
        std::os::unix::fs::symlink(outside.path(), disk("old").join("link")).unwrap();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let removed = backend
            .delete(
                None,
                Side::Local,
                vec![DeleteTarget {
                    path: root.join("old"),
                    kind: hoprs_core::EntryKind::Dir,
                }],
                tx,
            )
            .await
            .unwrap();
        assert_eq!(removed, 3, "a.txt, link, old/");
        assert!(!disk("old").exists());
        assert!(outside.path().join("keep.txt").is_file());
        let mut last = 0;
        while let Some(n) = rx.recv().await {
            last = n;
        }
        assert_eq!(last, 3);

        let err = backend
            .mkdir(None, Side::Remote, root.join("x"))
            .await
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::NotFound);
    }

    #[tokio::test]
    async fn unreadable_queue_falls_back_to_memory() {
        let dir = tempfile::tempdir().unwrap();
        let queue = dir.path().join("queue.db");
        std::fs::write(&queue, b"not a database").unwrap();
        let config = BackendConfig {
            profile_dir: dir.path().join("profiles"),
            queue_path: Some(queue.clone()),
            sessions: SessionConfig::default(),
        };
        let backend = Backend::open(config, Arc::new(MemoryStore::new())).unwrap();
        assert!(backend.engine.snapshot().await.is_empty());
        // The broken file is left as it was.
        assert_eq!(std::fs::read(&queue).unwrap(), b"not a database");
    }
}
