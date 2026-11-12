//! An in-memory [`BackendApi`] for the UI tests of Tasks 5–11 (ruling R3: test-only).
//!
//! Sites and workspaces live in memory. Every site has its own [`MemFs`] as its remote
//! side, and one shared `MemFs` stands for the local disk. Transfers, controls, answers and
//! closes are recorded, and dialogs are pushed by the test with
//! [`FakeBackend::push_prompt`].
//!
//! Connects follow the real [`SessionManager`](super::SessionManager) rules:
//! - one session per site: connecting a site with an open session returns that session;
//! - each connect attempt takes the next scripted outcome of the site (default success);
//! - a failed first connect keeps the session (its id comes with the error); connecting
//!   the site again, or `reconnect`, retries it with the next outcome, while a connected
//!   session is returned as is;
//! - with the keyring "locked" ([`FakeBackend::lock_keyring`]), sessions of sites with a
//!   stored secret report `keyring_unavailable`;
//! - secrets are read like the real `SecretBatch`: every site with a stored secret costs
//!   one keyring read ([`FakeBackend::keyring_reads`]), except that within one
//!   `connect_batch` a locked keyring is read once and the remaining sites are marked
//!   without asking it again;
//! - `connect_batch` runs the sites concurrently and sends each result as it finishes; a
//!   site's connect can be held back with [`FakeBackend::hold_connect`].
//!
//! `save_site` keeps the backend-owned fields of a stored site (`has_*`, and the FTPS pin
//! by the vault's [`merged_pin`](super::vault::merged_pin) rule), and
//! `set_password` records the password and sets `has_password`, like the vault.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use futures_util::StreamExt;
use futures_util::stream::FuturesUnordered;
use hoprs_core::{
    Error, ErrorKind, RemoteEntry, RemotePath, Result, Site, SiteId, Workspace, WorkspaceId,
};
use hoprs_remote::{MemFs, RemoteFs};
use hoprs_transfer::{TransferId, TransferSpec};
use hoprs_vault::UnreadableEntry;
use secrecy::SecretString;
use tokio::sync::{mpsc, oneshot};

use super::{
    BackendApi, BrokerEvent, ConnectError, ConnectResult, Connected, ControlOp, ProfileSnapshot,
    PromptAnswer, PromptId, SessionId, Side,
};

/// See the module docs.
pub struct FakeBackend {
    state: Mutex<State>,
    next_session: AtomicU64,
    next_transfer: AtomicI64,
    events: mpsc::UnboundedSender<BrokerEvent>,
    receiver: Mutex<Option<mpsc::UnboundedReceiver<BrokerEvent>>>,
}

#[derive(Default)]
struct State {
    sites: Vec<Site>,
    workspaces: Vec<Workspace>,
    unreadable: Vec<UnreadableEntry>,
    /// Stored passwords, by site.
    passwords: HashMap<SiteId, SecretString>,
    /// Keyring reads made to fetch connect secrets.
    keyring_reads: u32,
    /// Connects of these sites wait until the sender is used or dropped.
    holds: HashMap<SiteId, oneshot::Receiver<()>>,
    /// Scripted outcomes of the next connects of a site; empty means success.
    connects: HashMap<SiteId, VecDeque<std::result::Result<(), Error>>>,
    sessions: HashMap<SessionId, SiteId>,
    /// Sessions whose connection is up (the others failed and wait for a retry).
    connected: HashSet<SessionId>,
    /// Sessions started while the keyring was locked.
    keyring_unavailable: HashSet<SessionId>,
    keyring_locked: bool,
    remotes: HashMap<SiteId, MemFs>,
    local: MemFs,
    enqueued: Vec<TransferSpec>,
    controls: Vec<(TransferId, ControlOp)>,
    answers: Vec<(PromptId, PromptAnswer)>,
    closed: Vec<SessionId>,
}

impl Default for FakeBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeBackend {
    pub fn new() -> Self {
        let (events, receiver) = mpsc::unbounded_channel();
        Self {
            state: Mutex::new(State::default()),
            next_session: AtomicU64::new(1),
            next_transfer: AtomicI64::new(1),
            events,
            receiver: Mutex::new(Some(receiver)),
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn with_site(self, site: Site) -> Self {
        self.state().sites.push(site);
        self
    }

    pub fn with_workspace(self, workspace: Workspace) -> Self {
        self.state().workspaces.push(workspace);
        self
    }

    pub fn with_unreadable(self, entry: UnreadableEntry) -> Self {
        self.state().unreadable.push(entry);
        self
    }

    /// Keyring reads made so far to fetch connect secrets.
    pub fn keyring_reads(&self) -> u32 {
        self.state().keyring_reads
    }

    /// The stored password of `site`, if any.
    pub fn password(&self, site: &SiteId) -> Option<SecretString> {
        self.state().passwords.get(site).cloned()
    }

    /// The next connect of `site` waits until the returned sender is used or dropped.
    pub fn hold_connect(&self, site: &SiteId) -> oneshot::Sender<()> {
        let (release, held) = oneshot::channel();
        self.state().holds.insert(site.clone(), held);
        release
    }

    /// The next connect of `site` fails with `error` (scripts queue up in order).
    pub fn fail_next_connect(&self, site: &SiteId, error: Error) {
        self.state()
            .connects
            .entry(site.clone())
            .or_default()
            .push_back(Err(error));
    }

    /// From now on, new sessions of sites with a stored secret report the keyring as
    /// unavailable.
    pub fn lock_keyring(&self) {
        self.state().keyring_locked = true;
    }

    /// The remote side of `site` (shared with the backend: fill it in the test).
    pub fn remote_fs(&self, site: &SiteId) -> MemFs {
        self.state()
            .remotes
            .entry(site.clone())
            .or_default()
            .clone()
    }

    /// The local side (shared with the backend).
    pub fn local_fs(&self) -> MemFs {
        self.state().local.clone()
    }

    /// Sends a broker event to the UI's subscription.
    pub fn push_prompt(&self, event: BrokerEvent) {
        let _ = self.events.send(event);
    }

    pub fn enqueued(&self) -> Vec<TransferSpec> {
        self.state().enqueued.clone()
    }

    pub fn controls(&self) -> Vec<(TransferId, ControlOp)> {
        self.state().controls.clone()
    }

    pub fn answers(&self) -> Vec<(PromptId, PromptAnswer)> {
        self.state().answers.clone()
    }

    pub fn closed(&self) -> Vec<SessionId> {
        self.state().closed.clone()
    }

    /// The site a session belongs to, while it is open.
    pub fn session_site(&self, session: SessionId) -> Option<SiteId> {
        self.state().sessions.get(&session).cloned()
    }

    /// One connection attempt of `session`: a connected session stays connected;
    /// otherwise the site's next scripted outcome decides.
    fn attempt(state: &mut State, session: SessionId, site: &SiteId) -> Result<()> {
        if state.connected.contains(&session) {
            return Ok(());
        }
        let outcome = state
            .connects
            .get_mut(site)
            .and_then(VecDeque::pop_front)
            .unwrap_or(Ok(()));
        if outcome.is_ok() {
            state.connected.insert(session);
        }
        outcome
    }

    async fn connect_one(&self, site: SiteId, keyring_down: &mut bool) -> ConnectResult {
        let started = self.start(&site, keyring_down)?;
        self.finish(&site, started).await
    }

    /// Reads the site's secrets (see the module docs) and finds or starts its session.
    fn start(
        &self,
        site: &SiteId,
        keyring_down: &mut bool,
    ) -> std::result::Result<(SessionId, bool), ConnectError> {
        let mut state = self.state();
        let Some(found) = state.sites.iter().find(|s| &s.id == site) else {
            return Err(ConnectError {
                session: None,
                error: Self::not_found("Unknown site"),
                keyring_unavailable: false,
            });
        };
        let has_secret = found.has_password || found.has_key_passphrase;
        let mut unavailable = false;
        if has_secret {
            if *keyring_down {
                unavailable = true;
            } else {
                state.keyring_reads += 1;
                if state.keyring_locked {
                    *keyring_down = true;
                    unavailable = true;
                }
            }
        }
        let existing = state
            .sessions
            .iter()
            .find(|(_, s)| *s == site)
            .map(|(id, _)| *id);
        let id = match existing {
            Some(id) => id,
            None => {
                let id = SessionId(self.next_session.fetch_add(1, Ordering::SeqCst));
                state.sessions.insert(id, site.clone());
                state.remotes.entry(site.clone()).or_default();
                if unavailable {
                    state.keyring_unavailable.insert(id);
                }
                id
            }
        };
        let keyring_unavailable = state.keyring_unavailable.contains(&id);
        Ok((id, keyring_unavailable))
    }

    /// Waits for a hold on the site, then makes the connection attempt.
    async fn finish(
        &self,
        site: &SiteId,
        (id, keyring_unavailable): (SessionId, bool),
    ) -> ConnectResult {
        let held = self.state().holds.remove(site);
        if let Some(held) = held {
            let _ = held.await;
        }
        let mut state = self.state();
        match Self::attempt(&mut state, id, site) {
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

    fn not_found(what: &str) -> Error {
        Error::new(ErrorKind::NotFound, what.to_owned())
    }
}

#[async_trait]
impl BackendApi for FakeBackend {
    async fn sites(&self) -> Result<Vec<Site>> {
        Ok(self.state().sites.clone())
    }

    async fn workspaces(&self) -> Result<Vec<Workspace>> {
        Ok(self.state().workspaces.clone())
    }

    async fn profiles(&self) -> Result<ProfileSnapshot> {
        let state = self.state();
        Ok(ProfileSnapshot {
            sites: state.sites.clone(),
            workspaces: state.workspaces.clone(),
            unreadable: state.unreadable.clone(),
        })
    }

    fn profile_dir(&self) -> PathBuf {
        PathBuf::from("fake-profiles")
    }

    async fn save_site(&self, mut site: Site) -> Result<()> {
        let mut state = self.state();
        match state.sites.iter_mut().find(|s| s.id == site.id) {
            Some(slot) => {
                site.ftp.pinned_fingerprint = super::vault::merged_pin(Some(slot), &site);
                site.has_password = slot.has_password;
                site.has_key_passphrase = slot.has_key_passphrase;
                *slot = site;
            }
            None => {
                site.ftp.pinned_fingerprint = None;
                site.has_password = false;
                site.has_key_passphrase = false;
                state.sites.push(site);
            }
        }
        Ok(())
    }

    async fn set_password(&self, site: SiteId, password: Option<SecretString>) -> Result<()> {
        let mut state = self.state();
        let slot = state
            .sites
            .iter_mut()
            .find(|s| s.id == site)
            .ok_or_else(|| Self::not_found("Unknown site"))?;
        slot.has_password = password.is_some();
        match password {
            Some(password) => state.passwords.insert(site, password),
            None => state.passwords.remove(&site),
        };
        Ok(())
    }

    async fn delete_site(&self, id: SiteId) -> Result<()> {
        let mut state = self.state();
        let before = state.sites.len();
        state.sites.retain(|s| s.id != id);
        if state.sites.len() == before {
            return Err(Self::not_found("Unknown site"));
        }
        for workspace in &mut state.workspaces {
            workspace.sites.retain(|s| s.site != id);
        }
        Ok(())
    }

    async fn save_workspace(&self, workspace: Workspace) -> Result<()> {
        let mut state = self.state();
        match state.workspaces.iter_mut().find(|w| w.id == workspace.id) {
            Some(slot) => *slot = workspace,
            None => state.workspaces.push(workspace),
        }
        Ok(())
    }

    async fn delete_workspace(&self, id: WorkspaceId) -> Result<()> {
        let mut state = self.state();
        let before = state.workspaces.len();
        state.workspaces.retain(|w| w.id != id);
        if state.workspaces.len() == before {
            return Err(Self::not_found("Unknown workspace"));
        }
        Ok(())
    }

    async fn connect(&self, site: SiteId) -> ConnectResult {
        let mut keyring_down = false;
        self.connect_one(site, &mut keyring_down).await
    }

    async fn connect_batch(
        &self,
        sites: Vec<SiteId>,
        results: mpsc::UnboundedSender<(SiteId, ConnectResult)>,
    ) {
        // One secret batch: the reads all happen first, as in `sites_with_secrets`.
        let mut keyring_down = false;
        let prepared: Vec<_> = sites
            .into_iter()
            .map(|site| {
                let start = self.start(&site, &mut keyring_down);
                (site, start)
            })
            .collect();
        let mut pending: FuturesUnordered<_> = prepared
            .into_iter()
            .map(|(site, start)| async move {
                let result = match start {
                    Ok(started) => self.finish(&site, started).await,
                    Err(error) => Err(error),
                };
                (site, result)
            })
            .collect();
        while let Some(result) = pending.next().await {
            let _ = results.send(result);
        }
    }

    async fn reconnect(&self, session: SessionId) -> Result<()> {
        let mut state = self.state();
        let site = state
            .sessions
            .get(&session)
            .cloned()
            .ok_or_else(|| Self::not_found("Unknown session"))?;
        Self::attempt(&mut state, session, &site)
    }

    async fn close(&self, session: SessionId) {
        let mut state = self.state();
        if state.sessions.remove(&session).is_some() {
            state.connected.remove(&session);
            state.keyring_unavailable.remove(&session);
            state.closed.push(session);
        }
    }

    async fn list(
        &self,
        session: SessionId,
        side: Side,
        path: RemotePath,
    ) -> Result<Vec<RemoteEntry>> {
        let fs = match side {
            Side::Local => self.local_fs(),
            Side::Remote => {
                let site = self
                    .session_site(session)
                    .ok_or_else(|| Self::not_found("Unknown session"))?;
                self.remote_fs(&site)
            }
        };
        fs.list(&path).await
    }

    async fn enqueue(&self, spec: TransferSpec) -> Result<TransferId> {
        self.state().enqueued.push(spec);
        Ok(TransferId(
            self.next_transfer.fetch_add(1, Ordering::SeqCst),
        ))
    }

    async fn control(&self, id: TransferId, op: ControlOp) -> Result<()> {
        self.state().controls.push((id, op));
        Ok(())
    }

    async fn answer_prompt(&self, id: PromptId, answer: PromptAnswer) {
        self.state().answers.push((id, answer));
    }

    fn prompt_events(&self) -> Option<mpsc::UnboundedReceiver<BrokerEvent>> {
        self.receiver
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use hoprs_core::Protocol;
    use hoprs_transfer::Direction;

    use super::*;

    #[tokio::test]
    async fn fake_backend_connects_lists_and_records() {
        let site = Site::new("s", "h", "u", Protocol::Ssh);
        let ws = Workspace {
            id: WorkspaceId::new(),
            name: "w".into(),
            sites: vec![hoprs_core::WorkspaceSite {
                site: site.id.clone(),
                tabs: Vec::new(),
            }],
        };
        let fake = FakeBackend::new()
            .with_site(site.clone())
            .with_workspace(ws.clone());
        fake.remote_fs(&site.id)
            .add_file("/srv/a.txt", b"abc", SystemTime::UNIX_EPOCH);
        fake.local_fs().add_dir("/home/me");

        // Scripted failure first, then success; unknown sites are NotFound.
        fake.fail_next_connect(&site.id, Error::new(ErrorKind::Auth, "Denied"));
        let err = fake.connect(site.id.clone()).await.unwrap_err();
        assert_eq!(err.error.kind, ErrorKind::Auth);
        let failed = err.session.unwrap();
        assert!(fake.reconnect(failed).await.is_ok());
        let id = fake.connect(site.id.clone()).await.unwrap().session;
        assert_eq!(id, failed, "the kept session was retried, not replaced");
        let missing = fake.connect(SiteId::new()).await.unwrap_err();
        assert_eq!(missing.session, None);

        let remote = fake
            .list(id, Side::Remote, RemotePath::new("/srv").unwrap())
            .await
            .unwrap();
        assert_eq!(remote[0].name, "a.txt");
        let local = fake
            .list(id, Side::Local, RemotePath::new("/home").unwrap())
            .await
            .unwrap();
        assert_eq!(local[0].name, "me");

        let spec = TransferSpec {
            site: site.id.clone(),
            direction: Direction::Upload,
            local: RemotePath::new("/home/me").unwrap(),
            remote: RemotePath::new("/srv").unwrap(),
        };
        let tid = fake.enqueue(spec.clone()).await.unwrap();
        fake.control(tid, ControlOp::Pause).await.unwrap();
        fake.answer_prompt(PromptId(7), PromptAnswer::Decision(true))
            .await;
        fake.close(id).await;
        assert_eq!(fake.enqueued(), vec![spec]);
        assert_eq!(fake.controls(), vec![(tid, ControlOp::Pause)]);
        assert!(matches!(
            fake.answers()[..],
            [(PromptId(7), PromptAnswer::Decision(true))]
        ));
        assert_eq!(fake.closed(), vec![id]);
        assert!(fake.session_site(id).is_none());

        let mut events = fake.prompt_events().unwrap();
        assert!(fake.prompt_events().is_none());
        fake.push_prompt(BrokerEvent::Withdrawn(PromptId(1)));
        assert_eq!(
            events.recv().await,
            Some(BrokerEvent::Withdrawn(PromptId(1)))
        );

        // Sites and workspaces.
        let (tx, mut rx) = mpsc::unbounded_channel();
        fake.connect_batch(vec![site.id.clone()], tx).await;
        let (_, result) = rx.recv().await.unwrap();
        fake.close(result.unwrap().session).await;
        fake.save_workspace(ws.clone()).await.unwrap();
        assert_eq!(fake.workspaces().await.unwrap().len(), 1);
        fake.delete_site(site.id.clone()).await.unwrap();
        assert!(fake.workspaces().await.unwrap()[0].sites.is_empty());
        fake.delete_workspace(ws.id).await.unwrap();
        assert!(fake.sites().await.unwrap().is_empty());
    }

    /// The fake follows the real `SessionManager` rules (see the module docs).
    #[tokio::test]
    async fn fake_backend_follows_session_manager_rules() {
        let site = Site::new("s", "h", "u", Protocol::Ftp);
        let mut secret = Site::new("p", "h", "u", Protocol::Ftp);
        secret.has_password = true;
        let fake = FakeBackend::new()
            .with_site(site.clone())
            .with_site(secret.clone());

        // A second connect of an open site returns the same session, without an attempt.
        let first = fake.connect(site.id.clone()).await.unwrap();
        assert!(!first.keyring_unavailable);
        fake.fail_next_connect(&site.id, Error::new(ErrorKind::Network, "down"));
        assert_eq!(fake.connect(site.id.clone()).await.unwrap(), first);
        // The scripted failure is still pending: a closed site starts a new session.
        fake.close(first.session).await;
        let err = fake.connect(site.id.clone()).await.unwrap_err();
        let kept = err
            .session
            .expect("a failed first connect keeps the session");
        assert!(kept > first.session);

        // Connecting again retries the kept session; so does reconnect.
        fake.fail_next_connect(&site.id, Error::new(ErrorKind::Network, "still down"));
        let again = fake.connect(site.id.clone()).await.unwrap_err();
        assert_eq!(again.session, Some(kept));
        assert_eq!(again.error.message, "still down");
        assert!(fake.reconnect(kept).await.is_ok());
        assert_eq!(fake.connect(site.id.clone()).await.unwrap().session, kept);
        // A connected session's reconnect consumes no script.
        fake.fail_next_connect(&site.id, Error::new(ErrorKind::Network, "unused"));
        assert!(fake.reconnect(kept).await.is_ok());
        assert!(fake.reconnect(SessionId(999)).await.is_err());

        // A locked keyring is reported for sites with a stored secret, on both paths.
        fake.lock_keyring();
        let ok = fake.connect(secret.id.clone()).await.unwrap();
        assert!(ok.keyring_unavailable);
        fake.close(ok.session).await;
        fake.fail_next_connect(&secret.id, Error::new(ErrorKind::Auth, "Denied"));
        let err = fake.connect(secret.id.clone()).await.unwrap_err();
        assert!(err.keyring_unavailable);
        let other = Site::new("o", "h", "u", Protocol::Ftp);
        let fake = fake.with_site(other.clone());
        assert!(!fake.connect(other.id).await.unwrap().keyring_unavailable);
    }
}
