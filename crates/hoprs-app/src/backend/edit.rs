//! Remote editing (Plan 6, Task 7): a remote file is downloaded to a private local
//! directory, opened in the user's editor, watched, and uploaded again on every save.
//!
//! **Where.** `<root>/<session>/<remote path>`, the root being `$XDG_RUNTIME_DIR/hoprs/edit`
//! (else the user's cache directory, see [`default_edit_dir`]). On Unix the directories are
//! `0700` and the files `0600`. The remote path is mapped component by component and
//! refused if any component is empty, `.`, `..`, holds a separator or NUL, or would make
//! the path leave the session directory ([`local_path_for`]). A session directory found
//! on disk before this run's first edit of that session (a crash left it; ids restart
//! each run) is moved aside as `<session>.stale-<timestamp>`, never deleted.
//!
//! **Editor.** The configured command is split with shell quoting rules ([`shlex`]) but no
//! shell runs: no expansion, no substitution. The path is the last argument, as one
//! argument. The editor is spawned detached (its own process group, null stdio) and
//! never waited on by the caller; a thread reaps it when it exits ([`launch_editor`]).
//!
//! **Watching.** One `notify` watcher per session directory, watching the directory of
//! each edited file (added once it exists) and not the files themselves: editors that save by renaming a temporary file over the
//! original, or by removing and recreating it, are seen like plain writes (the event names
//! the edited path either way). Access and metadata events are ignored (our own reads
//! would otherwise wake us). Each session has a worker task that debounces every file
//! separately ([`DEBOUNCE`], tokio time) and then syncs it. A sync uploads only if the
//! local bytes differ from what was last downloaded or uploaded (a hash), so our own
//! writes and editors that touch without changing anything upload nothing.
//!
//! **Upload.** Before uploading, the remote file is `stat`ed: if its size, or its mtime to
//! the second (FTP's `MDTM` resolution; both stamps come from the same server and the
//! same command, so they compare exactly at that resolution), differ from the
//! [`RemoteStamp`] of the last download or upload, nothing is sent and
//! [`EditEvent::RemoteChanged`] asks the user ([`EditChoice`]). Otherwise the bytes go up
//! by [`UploadMode`]: SFTP writes a temporary file beside the original and renames it over
//! (OpenSSH's `posix-rename` replaces atomically; a server without it refuses to rename
//! over an existing file, and the upload then falls back to writing in place); FTP writes
//! in place (`RNTO` over an existing file is refused by many servers). After the upload
//! the file is `stat`ed again for the new stamp.
//!
//! **Down.** A save while the session is not `Connected`, or one whose upload fails with
//! a network error, is held as [`EditState::Pending`] ([`EditEvent::Pending`]). The worker
//! watches the session's state and syncs every pending edit when the session is
//! `Connected` again (Review Focus 3). An in-place upload cut in the middle has damaged
//! the remote copy itself, so its retry skips the stamp check (the change was ours).
//!
//! **Closing.** [`EditManager::finish`] removes a session's local files only when none of
//! them holds unsent changes (or the user said to discard them); it waits for an upload in
//! flight first. A closed session leaves its edits as they are until then.
//!
//! Logs name paths and sizes, never contents.

use std::collections::HashMap;
use std::ffi::OsString;
use std::hash::{DefaultHasher, Hasher};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hoprs_core::{ConnectionState, EntryKind, Error, ErrorKind, RemoteEntry, RemotePath, Result};
use hoprs_remote::RemoteFs;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{broadcast, mpsc, watch};
use tokio::task::JoinHandle;
use tokio::time::Instant;

use super::{SessionId, blocking};

/// Quiet time after the last change of a file before it is uploaded.
pub const DEBOUNCE: Duration = Duration::from_millis(500);

/// One remote file being edited locally.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EditId(pub u64);

/// What the edits tell the UI ([`EditManager::subscribe`]).
#[derive(Clone, Debug, PartialEq)]
pub enum EditEvent {
    /// A save reached the server.
    Uploaded { id: EditId, at: SystemTime },
    /// The server's copy changed since the last download or upload: the save was not
    /// sent; the user picks an [`EditChoice`].
    RemoteChanged { id: EditId },
    /// The session is down: the save is held and goes up on the next `Connected`.
    Pending { id: EditId },
    /// The upload failed for another reason; the next save tries again.
    Failed { id: EditId, error: Error },
    /// The edit ended ([`EditManager::finish`]); its local file is gone.
    Closed { id: EditId },
}

/// The user's answer to [`EditEvent::RemoteChanged`].
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum EditChoice {
    /// Upload the local version over the server's.
    Overwrite,
    /// Download the server's version over the local file, keeping the local one beside
    /// it as `<name>.local-<timestamp>`.
    KeepRemote,
    /// Upload the local version beside the original as `<name>.hoprs-<timestamp>`, then
    /// continue from the server's version (downloaded over the local file).
    SaveCopy,
}

/// Where an edit stands.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum EditState {
    /// The local file is what was last downloaded or uploaded.
    Synced,
    /// Saved locally, not sent yet (the debounce, or a save not seen yet).
    Modified,
    /// Held while the session is down.
    Pending,
    /// Held because the server's copy changed.
    Conflict,
    /// The last upload failed.
    Failed,
}

/// An edit as [`EditManager::pending`] and [`EditManager::finish`] report it.
#[derive(Clone, Debug, PartialEq)]
pub struct EditSummary {
    pub id: EditId,
    pub session: SessionId,
    pub remote: RemotePath,
    pub local: PathBuf,
    pub state: EditState,
}

/// How a save reaches the server (see the module docs).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum UploadMode {
    /// Temporary file beside the original, renamed over it; in place if the rename is
    /// refused (SFTP).
    TempRename,
    /// Straight into the file (FTP).
    InPlace,
}

/// The server's copy as last seen: what a save is checked against.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct RemoteStamp {
    pub size: u64,
    pub mtime: Option<SystemTime>,
}

impl RemoteStamp {
    fn of(entry: &RemoteEntry) -> Self {
        Self {
            size: entry.size,
            mtime: entry.modified,
        }
    }

    /// Same size, and the same mtime to the second when both are known.
    fn matches(&self, other: &RemoteStamp) -> bool {
        let secs = |t: SystemTime| t.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs());
        self.size == other.size
            && match (self.mtime, other.mtime) {
                (Some(a), Some(b)) => secs(a) == secs(b),
                _ => true,
            }
    }
}

/// Opens a local file in the editor: `(editor command, path)`. Runs on the blocking pool.
pub type Launcher = Arc<dyn Fn(&str, &Path) -> Result<()> + Send + Sync>;

/// See the module docs. Cheap to clone.
#[derive(Clone)]
pub struct EditManager {
    inner: Arc<Inner>,
}

struct Inner {
    root: PathBuf,
    watch_files: bool,
    launcher: Launcher,
    next_id: AtomicU64,
    events: broadcast::Sender<EditEvent>,
    edits: Mutex<HashMap<EditId, Arc<Edit>>>,
    sessions: Mutex<HashMap<SessionId, SessionEdits>>,
    /// Serializes the creation of session directories.
    setup: tokio::sync::Mutex<()>,
}

/// A session's directory, watcher and worker.
struct SessionEdits {
    dir: PathBuf,
    files: mpsc::UnboundedSender<PathBuf>,
    worker: JoinHandle<()>,
    /// Watches the directory of each edited file; dropping it stops it.
    watcher: Option<notify::RecommendedWatcher>,
}

struct Edit {
    id: EditId,
    session: SessionId,
    remote: RemotePath,
    local: PathBuf,
    fs: Arc<dyn RemoteFs>,
    mode: UploadMode,
    state: watch::Receiver<ConnectionState>,
    /// One sync or resolution at a time.
    turn: Arc<tokio::sync::Mutex<()>>,
    sync: Mutex<SyncState>,
}

struct SyncState {
    stamp: RemoteStamp,
    /// Hash of the bytes last downloaded or uploaded.
    synced: u64,
    state: EditState,
    /// The next upload skips the stamp check: the user chose "Overwrite", or an in-place
    /// upload of ours was cut and changed the remote copy itself.
    skip_check: bool,
}

impl Edit {
    fn connected(&self) -> bool {
        matches!(*self.state.borrow(), ConnectionState::Connected)
    }

    fn sync_state(&self) -> MutexGuard<'_, SyncState> {
        lock(&self.sync)
    }

    fn set_state(&self, state: EditState) {
        self.sync_state().state = state;
    }

    fn summary(&self, state: EditState) -> EditSummary {
        EditSummary {
            id: self.id,
            session: self.session,
            remote: self.remote.clone(),
            local: self.local.clone(),
            state,
        }
    }
}

/// How a sync ended.
#[derive(Debug)]
enum Outcome {
    Unchanged,
    Uploaded,
    Pending,
    Conflict,
    Failed(Error),
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn digest(data: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    hasher.write(data);
    hasher.finish()
}

/// `YYYYmmdd-HHMMSS`, local time, for the names of kept copies.
fn timestamp() -> String {
    chrono::Local::now().format("%Y%m%d-%H%M%S").to_string()
}

fn no_edit() -> Error {
    Error::new(ErrorKind::NotFound, "No such edit")
}

impl EditManager {
    /// Edits under `root`, watched with `notify`, opened with [`launch_editor`].
    pub fn new(root: PathBuf) -> Self {
        Self::with_options(root, true, Arc::new(launch_editor))
    }

    /// `watch_files: false` leaves the file events to [`file_event`](Self::file_event)
    /// (tests, the fake backend).
    pub fn with_options(root: PathBuf, watch_files: bool, launcher: Launcher) -> Self {
        Self {
            inner: Arc::new(Inner {
                root,
                watch_files,
                launcher,
                next_id: AtomicU64::new(1),
                events: broadcast::channel(256).0,
                edits: Mutex::new(HashMap::new()),
                sessions: Mutex::new(HashMap::new()),
                setup: tokio::sync::Mutex::new(()),
            }),
        }
    }

    /// Downloads `remote` into the session's directory, remembers its [`RemoteStamp`] and
    /// opens it with `editor`. A file already being edited is not downloaded again (its
    /// local copy may hold unsent changes): the editor opens it again, same id. `state`
    /// is the session's connection state (the edits of a session follow the first one's).
    pub async fn start(
        &self,
        session: SessionId,
        fs: Arc<dyn RemoteFs>,
        state: watch::Receiver<ConnectionState>,
        mode: UploadMode,
        remote: RemotePath,
        editor: &str,
    ) -> Result<EditId> {
        let existing = lock(&self.inner.edits)
            .values()
            .find(|edit| edit.session == session && edit.remote == remote)
            .cloned();
        if let Some(edit) = existing {
            self.launch(editor, &edit.local).await?;
            return Ok(edit.id);
        }
        let entry = fs.stat(&remote).await?;
        if entry.kind != EntryKind::File {
            return Err(Error::new(ErrorKind::Io, "Only files can be edited")
                .with_detail(remote.to_string()));
        }
        let dir = self.session_dir(session, &state).await?;
        let local = local_path_for(&dir, remote.as_str())?;
        let data = fetch(fs.as_ref(), &remote).await?;
        let size = data.len();
        let synced = digest(&data);
        {
            let local = local.clone();
            blocking(move || write_private(&local, &data)).await?;
        }
        if let Some(parent) = local.parent() {
            self.watch_dir(session, parent)?;
        }
        let id = EditId(self.inner.next_id.fetch_add(1, Ordering::SeqCst));
        let edit = Arc::new(Edit {
            id,
            session,
            remote: remote.clone(),
            local: local.clone(),
            fs,
            mode,
            state,
            turn: Arc::default(),
            sync: Mutex::new(SyncState {
                stamp: RemoteStamp::of(&entry),
                synced,
                state: EditState::Synced,
                skip_check: false,
            }),
        });
        lock(&self.inner.edits).insert(id, edit);
        tracing::info!(edit = id.0, remote = %remote, local = %local.display(), size, "remote edit started");
        if let Err(error) = self.launch(editor, &local).await {
            lock(&self.inner.edits).remove(&id);
            let _ = blocking(move || {
                let _ = std::fs::remove_file(&local);
                Ok(())
            })
            .await;
            return Err(error);
        }
        Ok(id)
    }

    /// Settles an [`EditEvent::RemoteChanged`] (see [`EditChoice`]). "Overwrite" while the
    /// session is down is held like a save ([`EditEvent::Pending`]); the other two need
    /// the session.
    pub async fn resolve(&self, id: EditId, choice: EditChoice) -> Result<()> {
        let edit = self.inner.edit(id).ok_or_else(no_edit)?;
        match choice {
            EditChoice::Overwrite => match self.inner.sync(&edit, true).await {
                Outcome::Failed(error) => Err(error),
                Outcome::Conflict => {
                    Err(Error::new(ErrorKind::Conflict, "The remote file changed"))
                }
                Outcome::Unchanged | Outcome::Uploaded | Outcome::Pending => Ok(()),
            },
            EditChoice::KeepRemote => {
                let _turn = edit.turn.lock().await;
                let local = edit.local.clone();
                let backup = sibling(&local, &format!(".local-{}", timestamp()));
                let kept = backup.clone();
                blocking(move || match std::fs::copy(&local, &backup) {
                    Ok(_) => Ok(()),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    Err(e) => Err(e.into()),
                })
                .await?;
                tracing::info!(edit = id.0, kept = %kept.display(), "kept the local version beside the edit");
                self.inner.adopt_remote(&edit).await
            }
            EditChoice::SaveCopy => {
                let _turn = edit.turn.lock().await;
                let local = edit.local.clone();
                let data = blocking(move || read_local(&local))
                    .await?
                    .ok_or_else(|| Error::new(ErrorKind::NotFound, "The local file is gone"))?;
                let name = edit.remote.file_name().unwrap_or("file");
                let parent = edit.remote.parent().unwrap_or_else(RemotePath::root);
                let copy = parent.join(&format!("{name}.hoprs-{}", timestamp()));
                if edit.fs.stat(&copy).await.is_ok() {
                    return Err(Error::new(ErrorKind::Conflict, "The copy already exists")
                        .with_detail(copy.to_string()));
                }
                write_remote(edit.fs.as_ref(), &copy, &data).await?;
                tracing::info!(edit = id.0, copy = %copy, size = data.len(), "saved the local version beside the remote file");
                self.inner.adopt_remote(&edit).await
            }
        }
    }

    /// The session's edits with something not on the server yet (anything but
    /// [`EditState::Synced`]; a synced edit whose local file differs counts as
    /// [`EditState::Modified`]).
    pub async fn pending(&self, session: SessionId) -> Vec<EditSummary> {
        let edits = self.inner.session_edits(session);
        let mut pending = Vec::new();
        for edit in edits {
            if let Some(summary) = unsent(&edit).await {
                pending.push(summary);
            }
        }
        pending
    }

    /// Ends the session's edits: with nothing unsent (or `discard`), stops its watcher and
    /// worker, deletes its local directory and sends [`EditEvent::Closed`] for each edit,
    /// returning an empty list. Otherwise changes nothing and returns the unsent edits.
    /// An upload in flight finishes first.
    pub async fn finish(&self, session: SessionId, discard: bool) -> Result<Vec<EditSummary>> {
        let edits = self.inner.session_edits(session);
        let mut turns = Vec::with_capacity(edits.len());
        for edit in &edits {
            turns.push(edit.turn.clone().lock_owned().await);
        }
        if !discard {
            let mut pending = Vec::new();
            for edit in &edits {
                if let Some(summary) = unsent(edit).await {
                    pending.push(summary);
                }
            }
            if !pending.is_empty() {
                return Ok(pending);
            }
        }
        let record = lock(&self.inner.sessions).remove(&session);
        if let Some(record) = &record {
            record.worker.abort();
        }
        {
            let mut all = lock(&self.inner.edits);
            for edit in &edits {
                all.remove(&edit.id);
            }
        }
        if let Some(record) = record {
            let dir = record.dir.clone();
            drop(record);
            blocking(move || match std::fs::remove_dir_all(&dir) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(Error::from(e).with_detail(dir.display().to_string())),
            })
            .await?;
        }
        drop(turns);
        let mut ids: Vec<EditId> = edits.iter().map(|edit| edit.id).collect();
        ids.sort();
        tracing::info!(
            session = session.0,
            edits = ids.len(),
            discard,
            "remote edits finished"
        );
        for id in ids {
            self.inner.emit(EditEvent::Closed { id });
        }
        Ok(Vec::new())
    }

    /// The edits' events, from now on.
    pub fn subscribe(&self) -> broadcast::Receiver<EditEvent> {
        self.inner.events.subscribe()
    }

    /// The local copy of an edit, while it lasts.
    pub fn local_path(&self, id: EditId) -> Option<PathBuf> {
        self.inner.edit(id).map(|edit| edit.local.clone())
    }

    /// The watcher's report that `path` (in the session's directory) changed.
    pub fn file_event(&self, session: SessionId, path: PathBuf) {
        if let Some(record) = lock(&self.inner.sessions).get(&session) {
            let _ = record.files.send(path);
        }
    }

    /// Adds `dir` (an edited file's directory, which exists now) to the session's watcher.
    /// Watching each directory once it exists, rather than the session directory
    /// recursively, cannot miss a directory created just before a recursive watch would
    /// have reached it.
    fn watch_dir(&self, session: SessionId, dir: &Path) -> Result<()> {
        use notify::Watcher;
        let mut sessions = lock(&self.inner.sessions);
        match sessions
            .get_mut(&session)
            .and_then(|record| record.watcher.as_mut())
        {
            Some(watcher) => watcher
                .watch(dir, notify::RecursiveMode::NonRecursive)
                .map_err(watch_error),
            None => Ok(()),
        }
    }

    async fn launch(&self, editor: &str, path: &Path) -> Result<()> {
        let launcher = self.inner.launcher.clone();
        let (editor, path) = (editor.to_owned(), path.to_owned());
        blocking(move || launcher(&editor, &path)).await
    }

    /// The session's directory, created (with its watcher and worker) on first use.
    async fn session_dir(
        &self,
        session: SessionId,
        state: &watch::Receiver<ConnectionState>,
    ) -> Result<PathBuf> {
        let _setup = self.inner.setup.lock().await;
        if let Some(record) = lock(&self.inner.sessions).get(&session) {
            return Ok(record.dir.clone());
        }
        let root = self.inner.root.clone();
        let dir = blocking(move || prepare_session_dir(&root, session)).await?;
        let (files, rx) = mpsc::unbounded_channel();
        let watcher = if self.inner.watch_files {
            Some(start_watcher(files.clone())?)
        } else {
            None
        };
        let worker = tokio::spawn(run_worker(
            Arc::downgrade(&self.inner),
            session,
            rx,
            state.clone(),
        ));
        lock(&self.inner.sessions).insert(
            session,
            SessionEdits {
                dir: dir.clone(),
                files,
                worker,
                watcher,
            },
        );
        Ok(dir)
    }
}

impl Inner {
    fn emit(&self, event: EditEvent) {
        let _ = self.events.send(event);
    }

    fn edit(&self, id: EditId) -> Option<Arc<Edit>> {
        lock(&self.edits).get(&id).cloned()
    }

    fn session_edits(&self, session: SessionId) -> Vec<Arc<Edit>> {
        let mut edits: Vec<Arc<Edit>> = lock(&self.edits)
            .values()
            .filter(|edit| edit.session == session)
            .cloned()
            .collect();
        edits.sort_by_key(|edit| edit.id);
        edits
    }

    fn edit_at(&self, session: SessionId, path: &Path) -> Option<EditId> {
        lock(&self.edits)
            .values()
            .find(|edit| edit.session == session && edit.local == path)
            .map(|edit| edit.id)
    }

    /// Uploads the local file if it changed (see the module docs). `overwrite` skips the
    /// stamp check, now and, if this ends pending, on the retry.
    async fn sync(&self, edit: &Arc<Edit>, overwrite: bool) -> Outcome {
        let _turn = edit.turn.lock().await;
        let id = edit.id;
        let local = edit.local.clone();
        let data = match blocking(move || read_local(&local)).await {
            Ok(Some(data)) => data,
            // Mid-save (removed, not recreated yet): the creation brings another event.
            Ok(None) => return Outcome::Unchanged,
            Err(error) => return self.failed(edit, error),
        };
        let hash = digest(&data);
        let (stamp, skip_check) = {
            let mut st = edit.sync_state();
            if overwrite {
                st.skip_check = true;
            }
            if hash == st.synced && !overwrite {
                st.state = EditState::Synced;
                return Outcome::Unchanged;
            }
            (st.stamp, st.skip_check)
        };
        if !edit.connected() {
            return self.pending(edit);
        }
        let before = match edit.fs.stat(&edit.remote).await {
            Ok(entry) => Some(entry),
            Err(error) if error.kind == ErrorKind::NotFound => None,
            Err(error) if error.kind == ErrorKind::Network => return self.pending(edit),
            Err(error) => return self.failed(edit, error),
        };
        let unchanged = before
            .as_ref()
            .is_some_and(|entry| stamp.matches(&RemoteStamp::of(entry)));
        if !skip_check && !unchanged {
            edit.set_state(EditState::Conflict);
            tracing::warn!(edit = id.0, remote = %edit.remote, "the remote file changed during the edit; upload held");
            self.emit(EditEvent::RemoteChanged { id });
            return Outcome::Conflict;
        }
        let mode_bits = before.as_ref().and_then(|entry| entry.mode);
        if let Err((error, clobbered)) =
            upload(edit.fs.as_ref(), &edit.remote, edit.mode, &data, mode_bits).await
        {
            if clobbered {
                edit.sync_state().skip_check = true;
            }
            return if error.kind == ErrorKind::Network {
                self.pending(edit)
            } else {
                self.failed(edit, error)
            };
        }
        let stamp = match edit.fs.stat(&edit.remote).await {
            Ok(entry) => RemoteStamp::of(&entry),
            // Uploaded, but the new mtime is unknown: check the size only next time.
            Err(_) => RemoteStamp {
                size: data.len() as u64,
                mtime: None,
            },
        };
        *edit.sync_state() = SyncState {
            stamp,
            synced: hash,
            state: EditState::Synced,
            skip_check: false,
        };
        tracing::info!(edit = id.0, remote = %edit.remote, size = data.len(), "remote edit uploaded");
        self.emit(EditEvent::Uploaded {
            id,
            at: SystemTime::now(),
        });
        Outcome::Uploaded
    }

    fn pending(&self, edit: &Edit) -> Outcome {
        edit.set_state(EditState::Pending);
        tracing::info!(edit = edit.id.0, remote = %edit.remote, "session not connected; edit held until it reconnects");
        self.emit(EditEvent::Pending { id: edit.id });
        Outcome::Pending
    }

    fn failed(&self, edit: &Edit, error: Error) -> Outcome {
        edit.set_state(EditState::Failed);
        tracing::warn!(edit = edit.id.0, remote = %edit.remote, %error, "remote edit upload failed");
        self.emit(EditEvent::Failed {
            id: edit.id,
            error: error.clone(),
        });
        Outcome::Failed(error)
    }

    /// Downloads the server's copy over the local file and takes its stamp. The caller
    /// holds the edit's turn.
    async fn adopt_remote(&self, edit: &Edit) -> Result<()> {
        let entry = edit.fs.stat(&edit.remote).await?;
        let data = fetch(edit.fs.as_ref(), &edit.remote).await?;
        let synced = digest(&data);
        let size = data.len();
        let local = edit.local.clone();
        blocking(move || write_private(&local, &data)).await?;
        *edit.sync_state() = SyncState {
            stamp: RemoteStamp::of(&entry),
            synced,
            state: EditState::Synced,
            skip_check: false,
        };
        tracing::info!(edit = edit.id.0, remote = %edit.remote, size, "remote version downloaded over the edit");
        Ok(())
    }
}

/// The edit's summary if something of it is not on the server.
async fn unsent(edit: &Arc<Edit>) -> Option<EditSummary> {
    let (state, synced) = {
        let st = edit.sync_state();
        (st.state, st.synced)
    };
    if state != EditState::Synced {
        return Some(edit.summary(state));
    }
    let local = edit.local.clone();
    let changed = blocking(move || Ok(read_local(&local)?.map(|data| digest(&data))))
        .await
        .ok()
        .flatten()
        .is_some_and(|hash| hash != synced);
    changed.then(|| edit.summary(EditState::Modified))
}

/// A session's worker: debounces file events per edit, syncs, and syncs the pending
/// edits when the session is connected again.
async fn run_worker(
    inner: Weak<Inner>,
    session: SessionId,
    mut files: mpsc::UnboundedReceiver<PathBuf>,
    mut state: watch::Receiver<ConnectionState>,
) {
    let mut due: HashMap<EditId, Instant> = HashMap::new();
    let mut connected = matches!(*state.borrow_and_update(), ConnectionState::Connected);
    let mut state_open = true;
    loop {
        let next = due.values().min().copied();
        let wake = async move {
            match next {
                Some(at) => tokio::time::sleep_until(at).await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            path = files.recv() => {
                let Some(path) = path else { break };
                let Some(inner) = inner.upgrade() else { break };
                if let Some(id) = inner.edit_at(session, &path) {
                    due.insert(id, Instant::now() + DEBOUNCE);
                }
            }
            changed = state.changed(), if state_open => {
                if changed.is_err() {
                    // The session is gone: its pending edits wait for `finish`.
                    state_open = false;
                    continue;
                }
                let now = matches!(*state.borrow_and_update(), ConnectionState::Connected);
                if now && !connected {
                    let Some(inner) = inner.upgrade() else { break };
                    for edit in inner.session_edits(session) {
                        if edit.sync_state().state == EditState::Pending {
                            tracing::info!(edit = edit.id.0, remote = %edit.remote, "session reconnected; uploading the held edit");
                            inner.sync(&edit, false).await;
                        }
                    }
                }
                connected = now;
            }
            () = wake => {
                let now = Instant::now();
                let mut ready: Vec<EditId> =
                    due.iter().filter(|(_, at)| **at <= now).map(|(id, _)| *id).collect();
                ready.sort();
                let Some(inner) = inner.upgrade() else { break };
                for id in ready {
                    due.remove(&id);
                    if let Some(edit) = inner.edit(id) {
                        inner.sync(&edit, false).await;
                    }
                }
            }
        }
    }
}

/// A session's watcher, watching nothing yet (see [`EditManager::watch_dir`]): sends the
/// paths of saves.
fn start_watcher(files: mpsc::UnboundedSender<PathBuf>) -> Result<notify::RecommendedWatcher> {
    let watcher =
        notify::recommended_watcher(move |event: notify::Result<notify::Event>| match event {
            Ok(event) if is_save(&event.kind) => {
                for path in event.paths {
                    let _ = files.send(path);
                }
            }
            Ok(_) => {}
            Err(error) => tracing::warn!(%error, "file watcher error"),
        })
        .map_err(watch_error)?;
    Ok(watcher)
}

fn watch_error(error: notify::Error) -> Error {
    Error::new(ErrorKind::Io, "Cannot watch the edited files").with_detail(error.to_string())
}

/// Whether a watcher event may be a save: creations, removals (an editor recreating the
/// file), data and name changes, and a close after writing. Not reads, opens or metadata.
fn is_save(kind: &notify::EventKind) -> bool {
    use notify::EventKind;
    use notify::event::{AccessKind, AccessMode, ModifyKind};
    match kind {
        EventKind::Create(_) | EventKind::Remove(_) => true,
        EventKind::Modify(ModifyKind::Metadata(_)) => false,
        EventKind::Modify(_) => true,
        EventKind::Access(AccessKind::Close(AccessMode::Write)) => true,
        _ => false,
    }
}

/// Reads a whole remote file.
async fn fetch(fs: &dyn RemoteFs, remote: &RemotePath) -> Result<Vec<u8>> {
    let mut reader = fs.open_read(remote, 0).await?;
    let mut data = Vec::new();
    reader.read_to_end(&mut data).await.map_err(Error::from)?;
    Ok(data)
}

/// Creates or replaces a remote file with `data`.
async fn write_remote(fs: &dyn RemoteFs, remote: &RemotePath, data: &[u8]) -> Result<()> {
    let mut writer = fs.open_write(remote, 0).await?;
    writer.write_all(data).await.map_err(Error::from)?;
    writer.shutdown().await.map_err(Error::from)?;
    Ok(())
}

/// Uploads `data` by `mode` (see the module docs). The error says whether the remote copy
/// itself may be damaged (an in-place write that did not finish).
async fn upload(
    fs: &dyn RemoteFs,
    remote: &RemotePath,
    mode: UploadMode,
    data: &[u8],
    mode_bits: Option<u32>,
) -> std::result::Result<(), (Error, bool)> {
    if mode == UploadMode::TempRename
        && let (Some(name), Some(parent)) = (remote.file_name(), remote.parent())
    {
        let temp = parent.join(&format!(".{name}.hoprs-upload"));
        match write_remote(fs, &temp, data).await {
            Ok(()) => {
                if let Some(bits) = mode_bits {
                    // Best effort: keep the original's permissions on the replacement.
                    let _ = fs.chmod(&temp, bits & 0o7777).await;
                }
                match fs.rename(&temp, remote).await {
                    Ok(()) => return Ok(()),
                    Err(error) if error.kind == ErrorKind::Network => {
                        return Err((error, false));
                    }
                    Err(error) => {
                        tracing::debug!(remote = %remote, %error, "rename over the file refused; writing in place");
                        let _ = fs.remove_file(&temp).await;
                    }
                }
            }
            Err(error) if error.kind == ErrorKind::Network => return Err((error, false)),
            Err(error) => {
                tracing::debug!(remote = %remote, %error, "cannot write a temporary file; writing in place");
                let _ = fs.remove_file(&temp).await;
            }
        }
    }
    write_remote(fs, remote, data).await.map_err(|e| (e, true))
}

/// The local file's bytes; `None` if it does not exist (right now).
fn read_local(path: &Path) -> Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(data) => Ok(Some(data)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::from(e).with_detail(path.display().to_string())),
    }
}

/// `path` with `suffix` added to its file name.
fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().map(OsString::from).unwrap_or_default();
    name.push(suffix);
    path.with_file_name(name)
}

/// Writes `data` to `path` (private, see [`create_private_dir_all`]), through a temporary
/// file renamed over it: the editor never sees a half-written file.
fn write_private(path: &Path, data: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| Error::new(ErrorKind::Io, "No parent directory"))?;
    create_private_dir_all(parent)?;
    let mut name = OsString::from(".");
    name.push(path.file_name().unwrap_or_default());
    name.push(".hoprs-dl");
    let temp = parent.join(name);
    let _ = std::fs::remove_file(&temp);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let written = options
        .open(&temp)
        .and_then(|mut file| file.write_all(data));
    if let Err(e) = written {
        let _ = std::fs::remove_file(&temp);
        return Err(Error::from(e).with_detail(temp.display().to_string()));
    }
    set_private(&temp, 0o600)?;
    std::fs::rename(&temp, path).map_err(|e| Error::from(e).with_detail(path.display().to_string()))
}

/// Creates `dir` and its missing parents, `0700` on Unix.
fn create_private_dir_all(dir: &Path) -> Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(dir)
        .map_err(|e| Error::from(e).with_detail(dir.display().to_string()))
}

/// Sets `mode` on Unix; nothing elsewhere (the user's profile directories are private
/// there already).
fn set_private(path: &Path, _mode: u32) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(_mode))
            .map_err(|e| Error::from(e).with_detail(path.display().to_string()))?;
    }
    Ok(())
}

/// Creates `<root>/<session>` (and the root), `0700`, moving aside a directory left by an
/// earlier run. Returns its canonical path (what the watcher reports).
fn prepare_session_dir(root: &Path, session: SessionId) -> Result<PathBuf> {
    create_private_dir_all(root)?;
    set_private(root, 0o700)?;
    let dir = root.join(session.0.to_string());
    if std::fs::symlink_metadata(&dir).is_ok() {
        let stale = root.join(format!("{}.stale-{}", session.0, timestamp()));
        tracing::warn!(dir = %dir.display(), moved_to = %stale.display(), "an edit directory from an earlier run was moved aside");
        std::fs::rename(&dir, &stale)
            .map_err(|e| Error::from(e).with_detail(dir.display().to_string()))?;
    }
    create_private_dir_all(&dir)?;
    set_private(&dir, 0o700)?;
    std::fs::canonicalize(&dir).map_err(|e| Error::from(e).with_detail(dir.display().to_string()))
}

fn refused(remote: &str) -> Error {
    Error::new(
        ErrorKind::Permission,
        "This remote path cannot be edited locally",
    )
    .with_detail(format!("{remote:?}"))
}

/// The local path of `remote` under `session_dir`: each component of the absolute remote
/// path must be a plain name (not empty, `.` or `..`, without `\`, NUL or, on Windows,
/// `:`), and the result must stay inside `session_dir`.
pub fn local_path_for(session_dir: &Path, remote: &str) -> Result<PathBuf> {
    let relative = remote.strip_prefix('/').ok_or_else(|| refused(remote))?;
    if relative.is_empty() {
        return Err(refused(remote));
    }
    let mut path = session_dir.to_path_buf();
    for part in relative.split('/') {
        let plain = !part.is_empty()
            && part != "."
            && part != ".."
            && !part.contains(['\\', '\0'])
            && !(cfg!(windows) && part.contains(':'));
        let mut components = Path::new(part).components();
        let single = matches!(
            (components.next(), components.next()),
            (Some(Component::Normal(_)), None)
        );
        if !plain || !single {
            return Err(refused(remote));
        }
        path.push(part);
    }
    if !path.starts_with(session_dir) || path == session_dir {
        return Err(refused(remote));
    }
    Ok(path)
}

/// The editor command split into program and arguments (shell quoting rules, no shell),
/// `path` last.
pub fn editor_argv(command: &str, path: &Path) -> Result<Vec<OsString>> {
    let words = shlex::split(command).ok_or_else(|| {
        Error::new(ErrorKind::Io, "The editor command has an unmatched quote")
            .with_detail(command.to_owned())
    })?;
    if words.is_empty() {
        return Err(Error::new(ErrorKind::Io, "No editor command is set"));
    }
    let mut argv: Vec<OsString> = words.into_iter().map(OsString::from).collect();
    argv.push(path.as_os_str().to_owned());
    Ok(argv)
}

/// Launches the editor (see [`editor_argv`]) detached, without waiting for it: null
/// stdio, its own process group on Unix, reaped by a thread when it exits.
pub fn launch_editor(command: &str, path: &Path) -> Result<()> {
    let argv = editor_argv(command, path)?;
    let (program, args) = argv.split_first().expect("editor_argv is never empty");
    let mut child = std::process::Command::new(program);
    child
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        child.process_group(0);
    }
    let mut child = child.spawn().map_err(|e| {
        Error::new(ErrorKind::Io, "Cannot start the editor")
            .with_detail(format!("{}: {e}", program.to_string_lossy()))
    })?;
    tracing::info!(editor = %program.to_string_lossy(), path = %path.display(), "editor started");
    let reaper = std::thread::Builder::new()
        .name("hoprs-editor".to_owned())
        .spawn(move || {
            let _ = child.wait();
        });
    if let Err(error) = reaper {
        tracing::debug!(%error, "cannot start the editor's reaper thread");
    }
    Ok(())
}

/// `$XDG_RUNTIME_DIR/hoprs/edit`, else the cache directory's `hoprs/edit`, else the
/// temporary directory's.
pub fn default_edit_dir() -> PathBuf {
    dirs::runtime_dir()
        .or_else(dirs::cache_dir)
        .unwrap_or_else(std::env::temp_dir)
        .join("hoprs")
        .join("edit")
}

#[cfg(test)]
mod tests;
