//! The backend the effects run against: the vault, the transfer engine, the session
//! manager (with the sessions' terminal shells) and the prompt broker, behind
//! [`BackendApi`].

pub mod broker;
#[cfg(test)]
pub mod fake;
pub mod mapping;
pub mod ops;
pub mod sessions;
pub mod shells;
pub mod vault;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use futures_util::stream::BoxStream;
use hoprs_core::{
    ConnectionState, Error, RemoteEntry, RemotePath, Result, SavedTab, Site, SiteId, Workspace,
    WorkspaceId,
};
use hoprs_import::{ImportOptions, ImportReport};
use hoprs_remote::RemoteFs;
pub use hoprs_ssh::known_hosts::KnownHostEntry;
pub use hoprs_ssh::{AuthMethod, ShellOutput};
use hoprs_transfer::{
    ConflictPolicy, EngineConfig, Event, QueueStore, Resolution, TransferEngine, TransferId,
    TransferItem, TransferSpec,
};
use hoprs_vault::{BackupImportReport, ExportParams, ExportReport, ImportMode, SecretStore, Vault};
use secrecy::SecretString;
use tokio::sync::{broadcast, mpsc, watch};

pub use broker::{BrokerEvent, PromptAnswer, PromptBroker, PromptId};
pub use ops::DeleteTarget;
pub use sessions::{
    ConnectError, ConnectResult, Connected, SessionConfig, SessionId, SessionManager,
};
pub use shells::ShellId;
pub use vault::{LazyKeyringStore, ProfileSnapshot, VaultHandle};

use crate::settings::{self, Settings, SettingsLoad};

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

/// The transfer queue as the engine has it, for the queue panel to start (or start over)
/// from.
#[derive(Clone, Debug, PartialEq)]
pub struct TransferSnapshot {
    /// The `seq` of the last engine event the items reflect (see
    /// [`TransferEngine::snapshot_with_seq`]).
    pub seq: u64,
    pub items: Vec<TransferItem>,
    /// Transfer workers per site.
    pub workers: usize,
}

/// A workspace's saved tabs, by site (see [`BackendApi::save_workspace_tabs`]).
pub type WorkspaceTabs = Vec<(SiteId, Vec<SavedTab>)>;

/// What `effects::run` may call. Each later task adds the methods its effects need.
#[async_trait]
pub trait BackendApi: Send + Sync {
    // Test convenience: the app reads through `profiles` and `connect_batch`.
    #[cfg(test)]
    async fn sites(&self) -> Result<Vec<Site>>;
    #[cfg(test)]
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
    /// Replaces the saved tabs of the workspace's sites named in `tabs` (the others keep
    /// theirs), reading the workspace as stored now: a form saved meanwhile is not undone.
    /// One atomic write of `workspaces.toml`; `NotFound` if the workspace is gone.
    async fn save_workspace_tabs(&self, workspace: WorkspaceId, tabs: WorkspaceTabs) -> Result<()>;

    /// Connects a saved site; see [`SessionManager::connect`].
    #[cfg(test)]
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
    /// Watches a session's connection state; `None` once it is closed. The sender goes
    /// away with the session, which ends the watch.
    fn session_state(&self, session: SessionId) -> Option<watch::Receiver<ConnectionState>>;
    /// How an SSH session logged in last; `None` for FTP, or before the first connection.
    fn session_auth(&self, session: SessionId) -> Option<AuthMethod>;
    /// The window is closing: shuts the transfer engine down (its running items keep their
    /// stored state, so the next start reopens them paused), then closes every session.
    async fn shutdown(&self);

    // Terminal shells, for the terminal tabs.
    /// Opens a terminal (PTY + shell, `cols` x `rows`) on an SSH session; see
    /// [`shells`]. Not connected: `Network`, "Not connected". Past the connection's channel
    /// budget: `Protocol`, "Too many channels on this connection (limit N)". An FTP session
    /// has no terminal (`Protocol`).
    async fn open_shell(&self, session: SessionId, cols: u16, rows: u16) -> Result<ShellId>;
    /// Sends keyboard input to the shell. Ended or unknown: `Network`, "Shell closed".
    async fn shell_input(&self, shell: ShellId, data: Vec<u8>) -> Result<()>;
    /// The terminal is now `cols` x `rows`.
    async fn shell_resize(&self, shell: ShellId, cols: u16, rows: u16) -> Result<()>;
    /// Closes the shell without waiting for the server (its output then yields `Closed`)
    /// and forgets it. Unknown ids are ignored.
    fn close_shell(&self, shell: ShellId);
    /// The shell's output (`Data`..., maybe `Exit`, then `Closed`), handed out once, for
    /// the tab's subscription; `None` afterwards and for unknown shells.
    fn shell_output(&self, shell: ShellId) -> Option<BoxStream<'static, ShellOutput>>;

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
    /// A new receiver of the engine's events. Subscribe first, then take
    /// [`transfer_snapshot`](Self::transfer_snapshot): events at or below its `seq` are
    /// already in it.
    fn transfer_events(&self) -> broadcast::Receiver<Event>;
    async fn transfer_snapshot(&self) -> TransferSnapshot;
    /// Removes the finished top-level items (and their children). Emits no event: take a
    /// new snapshot afterwards.
    async fn clear_finished(&self) -> Result<usize>;

    /// The `known_hosts` lines [`remove_known_host`](Self::remove_known_host) would delete
    /// for `host:port`, to show before it does (a line that also covers other hosts).
    async fn known_host_entries(&self, host: String, port: u16) -> Result<Vec<KnownHostEntry>>;
    /// Deletes every `known_hosts` line for `host:port` (the user's "Remove the old entry"
    /// after a changed host key), if they are still exactly `expected` (what the user saw
    /// and confirmed); `Conflict` otherwise, nothing removed. Returns how many.
    async fn remove_known_host(
        &self,
        host: String,
        port: u16,
        expected: Vec<KnownHostEntry>,
    ) -> Result<usize>;

    /// Delivers the UI's answer to a dialog (closing it is [`PromptAnswer::Cancel`]).
    async fn answer_prompt(&self, id: PromptId, answer: PromptAnswer);
    /// The dialogs to show, for the UI's subscription. Only the first call gets it.
    fn prompt_events(&self) -> Option<mpsc::UnboundedReceiver<BrokerEvent>>;

    /// "Import from FileZilla": reads and parses the `sitemanager.xml` at `path`, then
    /// applies it to the vault (sites, passwords to the keyring, optionally folders as
    /// workspaces). An unreadable or invalid file is an `Err` and changes nothing.
    async fn import_filezilla(&self, path: PathBuf, options: ImportOptions)
    -> Result<ImportReport>;
    /// Deletes the imported `sitemanager.xml` (see [`hoprs_import::delete_source`]: only a
    /// regular file that still looks like FileZilla's).
    async fn delete_sitemanager(&self, path: PathBuf) -> Result<()>;
    /// Writes the encrypted backup of every site, workspace and secret to `path`. Slow on
    /// purpose (Argon2id).
    async fn export_backup(&self, path: PathBuf, password: SecretString) -> Result<ExportReport>;
    /// Restores a backup made by [`export_backup`](Self::export_backup).
    async fn restore_backup(
        &self,
        path: PathBuf,
        password: SecretString,
        mode: ImportMode,
    ) -> Result<BackupImportReport>;

    /// The settings read when the backend opened (the engine and the sessions started
    /// with them), with the error if the file was unusable.
    fn settings(&self) -> SettingsLoad;
    /// Why the persistent transfer queue could not be opened, if it could not: this run
    /// keeps its transfers in memory.
    fn queue_error(&self) -> Option<Error>;
    /// Writes `settings.toml` (atomically). Does not apply anything.
    async fn save_settings(&self, settings: Settings) -> Result<()>;
    /// The conflict policy of the transfers dispatched from now on.
    async fn set_policy(&self, policy: ConflictPolicy);
    /// Transfer workers per site for the connections made from now on (`1..=16`).
    fn set_workers(&self, workers: usize);
    /// Keepalive (seconds, `10..=300`) of the SSH and FTP connections made from now on.
    fn set_keepalive(&self, secs: u64);
}

/// A remote listing asked without a session.
fn not_connected() -> Error {
    Error::new(hoprs_core::ErrorKind::NotFound, "Not connected")
}

/// Runs file work on the blocking pool.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    tokio::task::spawn_blocking(work).await.map_err(|e| {
        Error::new(hoprs_core::ErrorKind::Io, "Background task failed").with_detail(e.to_string())
    })?
}

/// Where the backend keeps its files.
#[derive(Clone, Debug)]
pub struct BackendConfig {
    /// `sites.toml` and `workspaces.toml` (see [`hoprs_vault::default_dir`]).
    pub profile_dir: PathBuf,
    /// The persistent transfer queue; `None` keeps it in memory.
    pub queue_path: Option<PathBuf>,
    /// The workers here are replaced by `settings.toml`'s.
    pub sessions: SessionConfig,
    /// Argon2id costs of the backups this backend writes.
    pub backup_params: ExportParams,
}

impl BackendConfig {
    /// The standard locations: profiles in `~/.config/hoprs`, the queue in
    /// `$XDG_DATA_HOME/hoprs/queue.db` (spec §5). `None` without a config directory.
    pub fn standard() -> Option<Self> {
        Some(Self {
            profile_dir: hoprs_vault::default_dir()?,
            queue_path: dirs::data_dir().map(|dir| dir.join("hoprs").join("queue.db")),
            sessions: SessionConfig::default(),
            backup_params: ExportParams::default(),
        })
    }
}

/// The real backend.
pub struct Backend {
    profile_dir: PathBuf,
    /// `settings.toml`, beside the profiles.
    settings_path: PathBuf,
    /// What `settings.toml` held when the backend opened.
    settings: SettingsLoad,
    /// Why the queue database could not be opened (the engine runs in memory then).
    queue_error: Option<Error>,
    backup_params: ExportParams,
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
    /// so the app still starts (the old file is left untouched);
    /// [`BackendApi::queue_error`] reports it.
    ///
    /// `settings.toml` (beside the profiles, also read here) gives the engine its conflict
    /// policy and the sessions their workers and keepalive; an unusable file means the defaults (it is
    /// logged, kept as is, and [`BackendApi::settings`] reports it).
    pub fn open(mut config: BackendConfig, secrets: Arc<dyn SecretStore>) -> Result<Backend> {
        let vault = VaultHandle::new(Vault::open(&config.profile_dir, secrets)?);
        let settings_path = config.profile_dir.join(settings::FILE_NAME);
        let loaded = settings::load(&settings_path);
        if let Some(error) = &loaded.error {
            tracing::warn!(%error, "settings.toml is unusable; this run uses the defaults");
        }
        let (engine, queue_error) = start_engine(
            config.queue_path.as_deref(),
            EngineConfig {
                default_policy: loaded.settings.default_policy,
                ..EngineConfig::default()
            },
        );
        config.sessions.workers = loaded.settings.workers_per_site;
        config.sessions.keepalive = std::time::Duration::from_secs(loaded.settings.keepalive_secs);
        let broker = PromptBroker::new(vault.clone());
        let sessions = SessionManager::new(
            engine.clone(),
            vault.clone(),
            broker.clone(),
            config.sessions,
        );
        Ok(Backend {
            profile_dir: config.profile_dir,
            settings_path,
            settings: loaded,
            queue_error,
            backup_params: config.backup_params,
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

/// The engine over the queue at `path`, or over an in-memory queue if that fails (with
/// why).
fn start_engine(path: Option<&Path>, config: EngineConfig) -> (TransferEngine, Option<Error>) {
    let persistent = path.map(|path| {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(Error::from)?;
        }
        TransferEngine::start(QueueStore::open(path)?, config.clone())
    });
    let error = match persistent {
        Some(Ok(engine)) => return (engine, None),
        Some(Err(err)) => {
            tracing::error!(error = %err, "cannot open the transfer queue; this run keeps it in memory");
            Some(err)
        }
        None => None,
    };
    let store = QueueStore::open_in_memory().expect("an in-memory queue always opens");
    let engine = TransferEngine::start(store, config).expect("an empty queue always starts");
    (engine, error)
}

#[async_trait]
impl BackendApi for Backend {
    #[cfg(test)]
    async fn sites(&self) -> Result<Vec<Site>> {
        self.vault.sites().await
    }

    #[cfg(test)]
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

    async fn save_workspace_tabs(&self, workspace: WorkspaceId, tabs: WorkspaceTabs) -> Result<()> {
        self.vault.save_workspace_tabs(workspace, tabs).await
    }

    #[cfg(test)]
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

    fn session_state(&self, session: SessionId) -> Option<watch::Receiver<ConnectionState>> {
        self.sessions.state(session)
    }

    fn session_auth(&self, session: SessionId) -> Option<AuthMethod> {
        self.sessions.auth(session)
    }

    /// The engine first: a session closed while the engine runs would pause its site,
    /// and an item paused that way reopens as the user's pause. Stopped first, the
    /// engine keeps each running item's stored state, which the next start recovers as
    /// paused.
    async fn shutdown(&self) {
        self.engine.clone().shutdown().await;
        self.sessions.close_all().await;
    }

    async fn open_shell(&self, session: SessionId, cols: u16, rows: u16) -> Result<ShellId> {
        self.sessions.open_shell(session, cols, rows).await
    }

    async fn shell_input(&self, shell: ShellId, data: Vec<u8>) -> Result<()> {
        self.sessions.shell_input(shell, data).await
    }

    async fn shell_resize(&self, shell: ShellId, cols: u16, rows: u16) -> Result<()> {
        self.sessions.shells().resize(shell, cols, rows).await
    }

    fn close_shell(&self, shell: ShellId) {
        self.sessions.shells().close(shell);
    }

    fn shell_output(&self, shell: ShellId) -> Option<BoxStream<'static, ShellOutput>> {
        self.sessions.shells().output(shell)
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

    fn transfer_events(&self) -> broadcast::Receiver<Event> {
        self.engine.subscribe()
    }

    async fn transfer_snapshot(&self) -> TransferSnapshot {
        let (seq, items) = self.engine.snapshot_with_seq().await;
        TransferSnapshot {
            seq,
            items,
            // As the engine clamps it.
            workers: self.sessions.workers().clamp(1, 16),
        }
    }

    async fn clear_finished(&self) -> Result<usize> {
        self.engine.clear_finished().await
    }

    async fn known_host_entries(&self, host: String, port: u16) -> Result<Vec<KnownHostEntry>> {
        let path = self.sessions.known_hosts_path();
        blocking(move || hoprs_ssh::known_hosts::entries_for(&path, &host, port)).await
    }

    async fn remove_known_host(
        &self,
        host: String,
        port: u16,
        expected: Vec<KnownHostEntry>,
    ) -> Result<usize> {
        let path = self.sessions.known_hosts_path();
        let removed =
            blocking(move || hoprs_ssh::known_hosts::remove_entries(&path, &host, port, &expected))
                .await?;
        tracing::info!(removed, "removed known_hosts entries at the user's request");
        Ok(removed)
    }

    async fn answer_prompt(&self, id: PromptId, answer: PromptAnswer) {
        self.broker.answer(id, answer);
    }

    fn prompt_events(&self) -> Option<mpsc::UnboundedReceiver<BrokerEvent>> {
        self.broker.take_events()
    }

    async fn import_filezilla(
        &self,
        path: PathBuf,
        options: ImportOptions,
    ) -> Result<ImportReport> {
        // Parsed before the vault is locked: a bad file never holds up the vault.
        let parsed = blocking(move || hoprs_import::read_sitemanager(&path)).await?;
        let report = self
            .vault
            .call(move |vault| hoprs_import::apply(vault, parsed, &options))
            .await?;
        tracing::info!(
            imported = report.imported.len(),
            skipped = report.skipped.len(),
            failures = report.failures.len(),
            "imported FileZilla sites"
        );
        Ok(report)
    }

    async fn delete_sitemanager(&self, path: PathBuf) -> Result<()> {
        blocking(move || hoprs_import::delete_source(&path)).await?;
        tracing::info!("deleted the imported sitemanager.xml at the user's request");
        Ok(())
    }

    async fn export_backup(&self, path: PathBuf, password: SecretString) -> Result<ExportReport> {
        let params = self.backup_params;
        self.vault
            .call(move |vault| vault.export(&path, &password, params))
            .await
    }

    async fn restore_backup(
        &self,
        path: PathBuf,
        password: SecretString,
        mode: ImportMode,
    ) -> Result<BackupImportReport> {
        self.vault
            .call(move |vault| vault.import_backup(&path, &password, mode))
            .await
    }

    fn settings(&self) -> SettingsLoad {
        self.settings.clone()
    }

    fn queue_error(&self) -> Option<Error> {
        self.queue_error.clone()
    }

    async fn save_settings(&self, settings: Settings) -> Result<()> {
        let path = self.settings_path.clone();
        blocking(move || settings::save(&path, &settings)).await
    }

    async fn set_policy(&self, policy: ConflictPolicy) {
        self.engine.set_policy(policy).await;
    }

    fn set_workers(&self, workers: usize) {
        self.sessions.set_workers(settings::clamp_workers(workers));
    }

    fn set_keepalive(&self, secs: u64) {
        self.sessions
            .set_keepalive(std::time::Duration::from_secs(settings::clamp_keepalive(
                secs,
            )));
    }
}

#[cfg(test)]
mod tests {
    use hoprs_core::{ErrorKind, Protocol, WorkspaceSite};
    use hoprs_transfer::{Direction, ResolveAction};
    use hoprs_vault::MemoryStore;

    use super::*;

    /// Argon2id at its cheapest: the tests check the plumbing, not the cost.
    const CHEAP_BACKUP: ExportParams = ExportParams {
        m_cost: 8,
        t_cost: 1,
        p_cost: 1,
    };

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../hoprs-import/tests/fixtures")
            .join(name)
    }

    fn open(dir: &Path) -> Backend {
        let config = BackendConfig {
            profile_dir: dir.join("profiles"),
            queue_path: Some(dir.join("data").join("queue.db")),
            sessions: SessionConfig::default(),
            backup_params: CHEAP_BACKUP,
        };
        Backend::open(config, Arc::new(MemoryStore::new())).unwrap()
    }

    /// A queue database that can't be read: the backend still opens, on an in-memory
    /// queue, and reports why (the file is left as it was).
    #[tokio::test]
    async fn corrupt_queue_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let queue = dir.path().join("data").join("queue.db");
        std::fs::create_dir_all(queue.parent().unwrap()).unwrap();
        let garbage = vec![0x5a_u8; 8192];
        std::fs::write(&queue, &garbage).unwrap();
        let backend = open(dir.path());
        let error = backend.queue_error().expect("the failure is reported");
        assert!(!error.message.is_empty());
        assert_eq!(std::fs::read(&queue).unwrap(), garbage, "left untouched");
        // The in-memory queue works.
        backend
            .enqueue(TransferSpec {
                site: SiteId::new(),
                direction: hoprs_transfer::Direction::Download,
                local: RemotePath::new("/local-x").unwrap(),
                remote: RemotePath::new("/r").unwrap(),
            })
            .await
            .unwrap();
        assert_eq!(backend.transfer_snapshot().await.items.len(), 1);

        // A queue that opens reports nothing.
        let fine = tempfile::tempdir().unwrap();
        assert_eq!(open(fine.path()).queue_error(), None);
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

    /// The queue's feed: events numbered after the snapshot's `seq`, the workers per site,
    /// and "Clear finished" (no event: only a new snapshot shows it).
    #[tokio::test]
    async fn transfer_feed_snapshot_and_clear() {
        use hoprs_transfer::{EventKind, TransferState};

        let dir = tempfile::tempdir().unwrap();
        let backend = open(dir.path());
        let mut events = backend.transfer_events();
        let empty = backend.transfer_snapshot().await;
        assert_eq!((empty.seq, empty.items.len(), empty.workers), (0, 0, 4));

        let id = backend
            .enqueue(TransferSpec {
                site: SiteId::new(),
                direction: Direction::Upload,
                local: RemotePath::new("/l").unwrap(),
                remote: RemotePath::new("/r").unwrap(),
            })
            .await
            .unwrap();
        let added = events.recv().await.unwrap();
        assert!(matches!(&added.kind, EventKind::Added(item) if item.id == id));
        backend.control(id, ControlOp::Cancel).await.unwrap();
        let cancelled = events.recv().await.unwrap();
        assert_eq!(cancelled.seq, added.seq + 1);
        let snapshot = backend.transfer_snapshot().await;
        assert_eq!(snapshot.seq, cancelled.seq);
        assert_eq!(snapshot.items[0].state, TransferState::Cancelled);

        assert_eq!(backend.clear_finished().await.unwrap(), 1);
        let after = backend.transfer_snapshot().await;
        assert_eq!((after.seq, after.items.len()), (snapshot.seq, 0));
    }

    /// "Remove the old entry": the preview lists what removing deletes, from the
    /// session manager's `known_hosts`.
    #[tokio::test]
    async fn known_host_entries_then_remove() {
        let dir = tempfile::tempdir().unwrap();
        let known_hosts = dir.path().join("known_hosts");
        let config = BackendConfig {
            profile_dir: dir.path().join("profiles"),
            queue_path: None,
            sessions: SessionConfig {
                known_hosts: Some(known_hosts.clone()),
                workers: 2,
                ..SessionConfig::default()
            },
            backup_params: CHEAP_BACKUP,
        };
        let backend = Backend::open(config, Arc::new(MemoryStore::new())).unwrap();
        let key = "AAAAC3NzaC1lZDI1NTE5AAAAIGrhflDkrs18RwIFvHFCsoXLdUL+0cVr81616Tzk8aBj";
        std::fs::write(
            &known_hosts,
            format!(
                "[h]:2222 ssh-ed25519 {key}\nother ssh-ed25519 {key}\n[h]:2222,other ssh-ed25519 {key}\n"
            ),
        )
        .unwrap();

        let entries = backend.known_host_entries("h".into(), 2222).await.unwrap();
        let lines: Vec<_> = entries
            .iter()
            .map(|e| (e.line, e.covers_other_hosts))
            .collect();
        assert_eq!(lines, [(1, false), (3, true)]);
        // Not what is in the file now (a stale preview): refused, nothing removed.
        let stale = entries[..1].to_vec();
        let err = backend
            .remove_known_host("h".into(), 2222, stale)
            .await
            .unwrap_err();
        assert_eq!(err.kind, hoprs_core::ErrorKind::Conflict);
        assert_eq!(
            backend.known_host_entries("h".into(), 2222).await.unwrap(),
            entries
        );
        assert_eq!(
            backend
                .remove_known_host("h".into(), 2222, entries)
                .await
                .unwrap(),
            2
        );
        assert_eq!(
            std::fs::read_to_string(&known_hosts).unwrap(),
            format!("other ssh-ed25519 {key}\n")
        );
        assert!(
            backend
                .known_host_entries("h".into(), 2222)
                .await
                .unwrap()
                .is_empty()
        );
    }

    /// Review Focus 5: closing stops the engine and the next start finds the unfinished
    /// items paused (nothing resumes on its own).
    #[tokio::test]
    async fn shutdown_stops_the_engine_and_items_reopen_paused() {
        use hoprs_transfer::{Direction, TransferState};

        let dir = tempfile::tempdir().unwrap();
        let backend = open(dir.path());
        let id = backend
            .enqueue(TransferSpec {
                site: SiteId::new(),
                direction: Direction::Upload,
                local: RemotePath::new("/l").unwrap(),
                remote: RemotePath::new("/r").unwrap(),
            })
            .await
            .unwrap();
        backend.shutdown().await;
        tokio::time::timeout(std::time::Duration::from_secs(5), backend.engine.stopped())
            .await
            .expect("the engine stopped");
        // Calls after the shutdown neither hang nor panic.
        backend.close(SessionId(1)).await;
        drop(backend);

        let reopened = open(dir.path());
        let items = reopened.transfer_snapshot().await.items;
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, id);
        assert_eq!(items[0].state, TransferState::Paused);
    }

    /// The tabs of the named sites are replaced in the stored workspace; the rest of it
    /// (a name changed meanwhile, other sites' tabs) is kept.
    #[tokio::test]
    async fn save_workspace_tabs_updates_only_the_named_sites() {
        use hoprs_core::{SavedTab, TabKind};

        let dir = tempfile::tempdir().unwrap();
        let backend = open(dir.path());
        let a = Site::new("a", "h", "u", Protocol::Ssh);
        let b = Site::new("b", "h", "u", Protocol::Ftp);
        backend.save_site(a.clone()).await.unwrap();
        backend.save_site(b.clone()).await.unwrap();
        let kept = SavedTab {
            kind: TabKind::Files,
            remote_dir: Some("/b".into()),
            local_dir: None,
        };
        let mut workspace = Workspace {
            id: WorkspaceId::new(),
            name: "w".into(),
            sites: vec![
                WorkspaceSite {
                    site: a.id.clone(),
                    tabs: Vec::new(),
                },
                WorkspaceSite {
                    site: b.id.clone(),
                    tabs: vec![kept.clone()],
                },
            ],
        };
        backend.save_workspace(workspace.clone()).await.unwrap();
        // Renamed in a form after the app read it.
        workspace.name = "renamed".into();
        backend.save_workspace(workspace.clone()).await.unwrap();

        let saved = SavedTab {
            kind: TabKind::Files,
            remote_dir: Some("/var/www".into()),
            local_dir: Some(PathBuf::from("/home/me")),
        };
        backend
            .save_workspace_tabs(
                workspace.id.clone(),
                vec![(a.id.clone(), vec![saved.clone()])],
            )
            .await
            .unwrap();
        let stored = &backend.workspaces().await.unwrap()[0];
        assert_eq!(stored.name, "renamed");
        assert_eq!(stored.sites[0].tabs, vec![saved.clone()]);
        assert_eq!(stored.sites[1].tabs, vec![kept]);
        // On disk too (a fresh vault reads it back).
        drop(backend);
        let reopened = open(dir.path());
        assert_eq!(
            reopened.workspaces().await.unwrap()[0].sites[0].tabs,
            vec![saved.clone()]
        );

        let err = reopened
            .save_workspace_tabs(WorkspaceId::new(), vec![(a.id, vec![saved])])
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
            backup_params: CHEAP_BACKUP,
        };
        let backend = Backend::open(config, Arc::new(MemoryStore::new())).unwrap();
        assert!(backend.engine.snapshot().await.is_empty());
        // The broken file is left as it was.
        assert_eq!(std::fs::read(&queue).unwrap(), b"not a database");
    }

    /// A real import of hoprs-import's FileZilla fixture into a temporary profile, with
    /// folders as workspaces; then the delete of the source file.
    #[tokio::test]
    async fn filezilla_import_into_a_temp_profile() {
        let dir = tempfile::tempdir().unwrap();
        let backend = open(dir.path());
        let options = ImportOptions {
            folders_as_workspaces: true,
        };

        // A file that is not there: an error, nothing written.
        let err = backend
            .import_filezilla(dir.path().join("missing.xml"), options)
            .await
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::NotFound);
        let err = backend
            .import_filezilla(fixture("sitemanager-broken.xml"), options)
            .await
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::Protocol);
        assert!(backend.sites().await.unwrap().is_empty());

        // A copy, so the delete below has something of its own to remove.
        let source = dir.path().join("sitemanager.xml");
        std::fs::copy(fixture("sitemanager-3.40-folders.xml"), &source).unwrap();
        let report = backend
            .import_filezilla(source.clone(), options)
            .await
            .unwrap();
        let names: Vec<_> = report.imported.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Alpha", "Beta", "Gamma", "Top level"]);
        assert_eq!(
            report.workspaces_created,
            [
                "Work",
                "Work/Clientes de Produção",
                "Work/Clientes de Produção/Deep"
            ]
        );
        assert!(report.failures.is_empty());
        let profiles = backend.profiles().await.unwrap();
        assert_eq!(profiles.sites.len(), 4);
        assert_eq!(profiles.workspaces.len(), 3);
        // The password went to the keyring, not to sites.toml.
        let alpha = profiles.sites.iter().find(|s| s.name == "Alpha").unwrap();
        assert!(alpha.has_password);
        let toml = std::fs::read_to_string(dir.path().join("profiles/sites.toml")).unwrap();
        assert!(!toml.contains("hunter2"));

        // Again: everything is already there.
        let again = backend
            .import_filezilla(source.clone(), options)
            .await
            .unwrap();
        assert!(again.imported.is_empty());
        assert_eq!(again.skipped.len(), 4);
        assert_eq!(backend.sites().await.unwrap().len(), 4);

        // Deleting the source: refused for a file that is not FileZilla's.
        let other = dir.path().join("notes.xml");
        std::fs::write(&other, "<?xml version=\"1.0\"?><notes/>").unwrap();
        let err = backend.delete_sitemanager(other.clone()).await.unwrap_err();
        assert_eq!(err.kind, ErrorKind::Protocol);
        assert!(other.exists());
        backend.delete_sitemanager(source.clone()).await.unwrap();
        assert!(!source.exists());
    }

    /// Export, then restore (merge and replace), through the vault on the blocking pool.
    #[tokio::test]
    async fn backup_export_and_restore() {
        let dir = tempfile::tempdir().unwrap();
        let backend = open(dir.path());
        let site = Site::new("prod", "h", "u", Protocol::Ssh);
        backend.save_site(site.clone()).await.unwrap();
        backend
            .set_password(site.id.clone(), Some(SecretString::from("s3cret")))
            .await
            .unwrap();
        let path = dir.path().join("out").join("hoprs.backup");

        let err = backend
            .export_backup(path.clone(), SecretString::from(""))
            .await
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::Protocol);
        assert!(!path.exists());
        let report = backend
            .export_backup(path.clone(), SecretString::from("pw"))
            .await
            .unwrap();
        assert_eq!((report.sites, report.workspaces, report.secrets), (1, 0, 1));
        let bytes = std::fs::read(&path).unwrap();
        assert!(!bytes.windows(6).any(|w| w == b"s3cret"));

        // A wrong password changes nothing.
        backend.delete_site(site.id.clone()).await.unwrap();
        assert!(
            backend
                .restore_backup(path.clone(), SecretString::from("nope"), ImportMode::Merge)
                .await
                .is_err()
        );
        assert!(backend.sites().await.unwrap().is_empty());

        let restored = backend
            .restore_backup(path.clone(), SecretString::from("pw"), ImportMode::Merge)
            .await
            .unwrap();
        assert_eq!((restored.added_sites, restored.secrets_restored), (1, 1));
        assert!(restored.secret_errors.is_empty());
        let sites = backend.sites().await.unwrap();
        assert_eq!(sites.len(), 1);
        assert!(sites[0].has_password);

        // Replace: a site made since is gone.
        backend
            .save_site(Site::new("later", "h2", "u", Protocol::Ftp))
            .await
            .unwrap();
        backend
            .restore_backup(path, SecretString::from("pw"), ImportMode::Replace)
            .await
            .unwrap();
        let names: Vec<_> = backend
            .sites()
            .await
            .unwrap()
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert_eq!(names, ["prod"]);
    }

    /// `settings.toml` is read at open (workers for the sessions, the policy for the
    /// engine), saved on request, and an unusable file is reported and left alone.
    #[tokio::test]
    async fn settings_apply_at_open_and_change() {
        use hoprs_remote::MemFs;
        use hoprs_transfer::{Direction, EventKind, SiteEndpoints, TransferState};

        let dir = tempfile::tempdir().unwrap();
        let profiles = dir.path().join("profiles");
        std::fs::create_dir_all(&profiles).unwrap();
        let file = profiles.join(settings::FILE_NAME);
        std::fs::write(
            &file,
            "workers_per_site = 3\ndefault_policy = \"skip\"\nkeepalive_secs = 60\n",
        )
        .unwrap();
        let backend = open(dir.path());
        let loaded = backend.settings();
        assert_eq!(loaded.error, None);
        assert_eq!(
            loaded.settings,
            Settings {
                workers_per_site: 3,
                default_policy: ConflictPolicy::Skip,
                keepalive_secs: 60,
                ..Settings::default()
            }
        );
        assert_eq!(backend.transfer_snapshot().await.workers, 3);
        assert_eq!(backend.sessions.workers(), 3);
        assert_eq!(
            backend.sessions.keepalive(),
            std::time::Duration::from_secs(60)
        );

        // The saved policy is the engine's from the start: an existing file is skipped,
        // never asked about.
        let site = SiteId::new();
        let local = MemFs::new();
        let remote = MemFs::new();
        let now = std::time::SystemTime::now();
        local.add_file("/l/a.txt", b"local", now);
        remote.add_file("/r/a.txt", b"remote", now);
        backend
            .engine
            .register_site(
                site.clone(),
                SiteEndpoints {
                    local: Arc::new(local),
                    remote: Arc::new(remote),
                    workers: 1,
                },
            )
            .await;
        let mut events = backend.transfer_events();
        let id = backend
            .enqueue(TransferSpec {
                site,
                direction: Direction::Download,
                local: RemotePath::new("/l/a.txt").unwrap(),
                remote: RemotePath::new("/r/a.txt").unwrap(),
            })
            .await
            .unwrap();
        let state = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if let EventKind::StateChanged { id: changed, state } =
                    events.recv().await.unwrap().kind
                    && changed == id
                    && !matches!(state, TransferState::Queued | TransferState::Active)
                {
                    return state;
                }
            }
        })
        .await
        .expect("the transfer settles");
        assert!(matches!(state, TransferState::Skipped(_)), "{state:?}");

        // Workers change for the next connections, clamped.
        backend.set_workers(40);
        assert_eq!(backend.sessions.workers(), 16);
        // The keepalive: clamped to 10..=300 s.
        backend.set_keepalive(2);
        assert_eq!(
            backend.sessions.keepalive(),
            std::time::Duration::from_secs(10)
        );
        backend.set_keepalive(45);
        assert_eq!(
            backend.sessions.keepalive(),
            std::time::Duration::from_secs(45)
        );
        backend.set_workers(0);
        assert_eq!(backend.transfer_snapshot().await.workers, 1);

        let saved = Settings {
            workers_per_site: 9,
            default_policy: ConflictPolicy::Resume,
            ..Settings::default()
        };
        backend.save_settings(saved).await.unwrap();
        assert_eq!(settings::load(&file).settings, saved);
        drop(backend);

        // An unusable file: the defaults, the error, and the file as it was.
        std::fs::write(&file, "workers_per_site = [").unwrap();
        let backend = open(dir.path());
        let loaded = backend.settings();
        assert_eq!(loaded.settings, Settings::default());
        assert!(loaded.error.is_some());
        assert_eq!(backend.sessions.workers(), 4);
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "workers_per_site = ["
        );
    }
}
