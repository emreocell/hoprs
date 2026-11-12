//! The engine actor: owns the queue store, the in-memory items, the per-site
//! worker slots and the progress tracker. Never awaits I/O itself: workers
//! run as spawned tasks and report back through `WorkerMsg`.
//!
//! Directories expand lazily: a worker lists a directory when it is
//! dispatched, and its entries join the queue as child items. Files are
//! dispatched before directories, so a large tree starts copying before it
//! has been listed completely. A directory stays `Active` (without holding a
//! worker slot) until all of its children are terminal.
//!
//! Control commands (pause, resume, cancel, retry, resolve) act on an item
//! and, for a directory, on its descendants. An item holding a worker slot
//! cannot change state until its worker reports back: the command records an
//! `Intent` and cancels the item's token, and the worker's result is mapped
//! through that intent.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::{Duration, Instant};

use hoprs_core::{EntryKind, Error, ErrorKind, RemoteEntry, Result, SiteId};
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::time::MissedTickBehavior;
use tokio_util::sync::CancellationToken;

use super::worker::{self, Prepared, Route, WorkerMsg};
use super::{EngineConfig, Event, Resolution, ResolveAction, SiteEndpoints};
use crate::conflict::Decision;
use crate::copy::CopyOutcome;
use crate::model::{
    ConflictInfo, ConflictPolicy, TransferId, TransferItem, TransferSpec, TransferState,
};
use crate::progress::ProgressTracker;
use crate::store::{NewChild, PausedFrom, QueueStore};

const MAX_WORKERS: usize = 16;
const SYMLINK_SKIP: &str = "symlinks are not followed";
const USER_SKIP: &str = "skipped by user";

fn unknown() -> Error {
    Error::new(ErrorKind::NotFound, "Unknown transfer")
}

fn conflict(message: &str) -> Error {
    Error::new(ErrorKind::Conflict, message)
}

/// Appends a queue-store error to `e`'s technical detail.
fn with_store_detail(e: Error, store_err: &Error) -> Error {
    let store = match &store_err.detail {
        Some(d) => format!("queue store: {}: {d}", store_err.message),
        None => format!("queue store: {}", store_err.message),
    };
    let detail = match &e.detail {
        Some(d) => format!("{d}; {store}"),
        None => store,
    };
    e.with_detail(detail)
}

pub(super) enum Command {
    RegisterSite {
        site: SiteId,
        endpoints: SiteEndpoints,
        reply: oneshot::Sender<()>,
    },
    Enqueue {
        spec: TransferSpec,
        reply: oneshot::Sender<Result<TransferId>>,
    },
    SetPolicy {
        policy: ConflictPolicy,
        reply: oneshot::Sender<()>,
    },
    Snapshot {
        reply: oneshot::Sender<Vec<TransferItem>>,
    },
    Control {
        id: TransferId,
        op: Control,
        reply: oneshot::Sender<Result<()>>,
    },
}

/// A control command on one item.
#[derive(Copy, Clone, Debug)]
pub(super) enum Control {
    Pause,
    Resume,
    Cancel,
    Retry,
    Resolve(Resolution),
}

/// What to do with an item holding a worker slot once its worker reports
/// back, set by a control command that arrived meanwhile.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Intent {
    Pause,
    Cancel,
    /// Paused, then resumed before the worker stopped: back to `Queued`.
    Requeue,
}

/// An item holding a worker slot (preparing or copying).
struct Running {
    cancel: CancellationToken,
    intent: Option<Intent>,
}

struct Site {
    endpoints: SiteEndpoints,
    /// Worker slots currently held by items of this site.
    busy: usize,
}

impl Site {
    fn free(&self) -> usize {
        self.endpoints.workers.saturating_sub(self.busy)
    }
}

/// `Queued` items of one site, in dispatch order: items not known to be
/// directories first, then directories, each by ascending id.
#[derive(Default)]
struct SiteQueue {
    files: BTreeSet<TransferId>,
    dirs: BTreeSet<TransferId>,
}

impl SiteQueue {
    fn push(&mut self, id: TransferId, is_dir: bool) {
        if is_dir {
            self.dirs.insert(id);
        } else {
            self.files.insert(id);
        }
    }

    fn remove(&mut self, id: TransferId) {
        self.files.remove(&id);
        self.dirs.remove(&id);
    }

    fn pop(&mut self) -> Option<TransferId> {
        self.files.pop_first().or_else(|| self.dirs.pop_first())
    }
}

/// Children of an expanded directory that are not terminal yet, and how
/// many of its finished children ended `Failed`.
struct DirProgress {
    pending: usize,
    failed: usize,
}

struct Actor {
    store: QueueStore,
    buf_size: usize,
    policy: ConflictPolicy,
    events: broadcast::Sender<Event>,
    items: BTreeMap<TransferId, TransferItem>,
    sites: HashMap<SiteId, Site>,
    /// Index of `Queued` items per site (registered or not).
    queued: HashMap<SiteId, SiteQueue>,
    /// `Active` expanded directories that still have non-terminal children.
    dirs: HashMap<TransferId, DirProgress>,
    /// Items holding a worker slot.
    running: HashMap<TransferId, Running>,
    /// One-shot conflict policy for an item's next run (from `resolve` or
    /// `resume`); consumed at dispatch, never persisted.
    overrides: HashMap<TransferId, ConflictPolicy>,
    tracker: ProgressTracker,
    worker_tx: mpsc::UnboundedSender<WorkerMsg>,
}

pub(super) async fn run(
    store: QueueStore,
    config: EngineConfig,
    mut commands: mpsc::Receiver<Command>,
    events: broadcast::Sender<Event>,
) {
    let (worker_tx, mut worker_rx) = mpsc::unbounded_channel();
    let mut actor = Actor {
        store,
        buf_size: config.buf_size,
        policy: config.default_policy,
        events,
        items: BTreeMap::new(),
        sites: HashMap::new(),
        queued: HashMap::new(),
        dirs: HashMap::new(),
        running: HashMap::new(),
        overrides: HashMap::new(),
        tracker: ProgressTracker::new(),
        worker_tx,
    };
    let mut ticker = tokio::time::interval(config.progress_interval.max(Duration::from_millis(1)));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            cmd = commands.recv() => match cmd {
                Some(cmd) => actor.handle_command(cmd),
                None => break,
            },
            Some(msg) = worker_rx.recv() => actor.handle_worker(msg),
            _ = ticker.tick() => actor.tick(),
        }
    }
}

impl Actor {
    fn emit(&self, event: Event) {
        let _ = self.events.send(event);
    }

    fn handle_command(&mut self, cmd: Command) {
        match cmd {
            Command::RegisterSite {
                site,
                mut endpoints,
                reply,
            } => {
                endpoints.workers = endpoints.workers.clamp(1, MAX_WORKERS);
                match self.sites.get_mut(&site) {
                    Some(existing) => existing.endpoints = endpoints,
                    None => {
                        self.sites.insert(site, Site { endpoints, busy: 0 });
                    }
                }
                let _ = reply.send(());
                self.dispatch();
            }
            Command::Enqueue { spec, reply } => {
                let id = match self.store.insert(None, &spec) {
                    Ok(id) => id,
                    Err(e) => {
                        let _ = reply.send(Err(e));
                        return;
                    }
                };
                self.add_item(TransferItem {
                    id,
                    parent: None,
                    spec,
                    is_dir: None,
                    size: None,
                    bytes_done: 0,
                    state: TransferState::Queued,
                });
                let _ = reply.send(Ok(id));
                self.dispatch();
            }
            Command::SetPolicy { policy, reply } => {
                self.policy = policy;
                let _ = reply.send(());
            }
            Command::Snapshot { reply } => {
                let _ = reply.send(self.items.values().cloned().collect());
            }
            Command::Control { id, op, reply } => {
                let result = match self.items.get(&id).map(|i| i.state.clone()) {
                    None => Err(unknown()),
                    Some(state) => self.control(id, &state, op),
                };
                let _ = reply.send(result);
                self.dispatch();
            }
        }
    }

    fn control(&mut self, id: TransferId, state: &TransferState, op: Control) -> Result<()> {
        match op {
            Control::Pause | Control::Cancel if state.is_terminal() => {
                Err(conflict("Transfer already finished"))
            }
            Control::Pause if *state == TransferState::Paused => Ok(()),
            Control::Pause => {
                self.stop_subtree(id, Intent::Pause);
                Ok(())
            }
            Control::Cancel => {
                self.stop_subtree(id, Intent::Cancel);
                Ok(())
            }
            Control::Resume if *state != TransferState::Paused => {
                Err(conflict("Transfer is not paused"))
            }
            Control::Resume => {
                self.restart_subtree(id, |s| *s == TransferState::Paused, Intent::Pause);
                Ok(())
            }
            Control::Retry if !is_retryable(state) => {
                Err(conflict("Transfer is not failed or cancelled"))
            }
            Control::Retry => {
                self.restart_subtree(id, is_retryable, Intent::Cancel);
                Ok(())
            }
            Control::Resolve(_) if !matches!(state, TransferState::AwaitingDecision(_)) => {
                Err(conflict("Transfer is not awaiting a decision"))
            }
            Control::Resolve(resolution) => {
                self.resolve(id, resolution);
                Ok(())
            }
        }
    }

    /// Child ids of every directory that has been listed into child items.
    fn children_index(&self) -> HashMap<TransferId, Vec<TransferId>> {
        let mut children: HashMap<TransferId, Vec<TransferId>> = HashMap::new();
        for item in self.items.values() {
            if let Some(parent) = item.parent {
                children.entry(parent).or_default().push(item.id);
            }
        }
        children
    }

    /// `id` and its descendants, children before their parent.
    fn subtree(children: &HashMap<TransferId, Vec<TransferId>>, id: TransferId) -> Vec<TransferId> {
        fn walk(
            id: TransferId,
            children: &HashMap<TransferId, Vec<TransferId>>,
            out: &mut Vec<TransferId>,
        ) {
            for &child in children.get(&id).into_iter().flatten() {
                walk(child, children, out);
            }
            out.push(id);
        }
        let mut out = Vec::new();
        walk(id, children, &mut out);
        out
    }

    /// Pauses or cancels `id` and its non-terminal descendants. Directories
    /// stop counting their children first, so the cascade never finalizes
    /// them; a cancelled directory becomes `Cancelled` directly.
    fn stop_subtree(&mut self, id: TransferId, intent: Intent) {
        let ids = Self::subtree(&self.children_index(), id);
        for &i in &ids {
            self.dirs.remove(&i);
        }
        for i in ids {
            self.stop_one(i, intent);
        }
    }

    fn stop_one(&mut self, id: TransferId, intent: Intent) {
        let Some(item) = self.items.get(&id) else {
            return;
        };
        if item.state.is_terminal()
            || (intent == Intent::Pause && item.state == TransferState::Paused)
        {
            return;
        }
        if let Some(running) = self.running.get_mut(&id) {
            // A pending cancel is never downgraded to a pause.
            if running.intent != Some(Intent::Cancel) {
                running.intent = Some(intent);
            }
            running.cancel.cancel();
            return;
        }
        self.overrides.remove(&id);
        let state = match intent {
            Intent::Cancel => TransferState::Cancelled,
            Intent::Pause | Intent::Requeue => TransferState::Paused,
        };
        self.transition(id, state);
    }

    /// Restarts every item of the subtree of `id` whose state matches
    /// `restartable`, children before their parent: an expanded directory
    /// becomes `Active` again (counting its current children, never listing
    /// it again), anything else becomes `Queued`. An item still stopping with
    /// the `stopping` intent (its worker has not reported back yet) is
    /// requeued when it does.
    fn restart_subtree(
        &mut self,
        id: TransferId,
        restartable: fn(&TransferState) -> bool,
        stopping: Intent,
    ) {
        let children = self.children_index();
        for i in Self::subtree(&children, id) {
            if let Some(running) = self.running.get_mut(&i) {
                if running.intent == Some(stopping) {
                    running.intent = Some(Intent::Requeue);
                }
                continue;
            }
            let Some(item) = self.items.get(&i) else {
                continue;
            };
            if !restartable(&item.state) {
                continue;
            }
            // Only the continuation of an interrupted copy forces `Resume`;
            // anything else runs under the global policy again.
            let resume_copy = item.state == TransferState::Paused
                && item.is_dir != Some(true)
                && matches!(self.store.paused_from(i), Ok(Some(PausedFrom::Active)));
            if resume_copy {
                self.overrides.insert(i, ConflictPolicy::Resume);
            } else {
                self.overrides.remove(&i);
            }
            if let Some(kids) = children.get(&i) {
                self.reactivate_dir(i, kids);
            } else {
                self.transition(i, TransferState::Queued);
            }
        }
    }

    /// Makes the expanded directory `id` `Active` again, counting its
    /// current children. Finishes it right away if none is pending.
    fn reactivate_dir(&mut self, id: TransferId, children: &[TransferId]) {
        let (mut pending, mut failed) = (0, 0);
        for child in children.iter().filter_map(|c| self.items.get(c)) {
            match child.state {
                TransferState::Failed(_) => failed += 1,
                ref s if !s.is_terminal() => pending += 1,
                _ => {}
            }
        }
        if pending == 0 {
            self.finish_dir(id, failed);
            return;
        }
        if self.transition(id, TransferState::Active) {
            self.dirs.insert(id, DirProgress { pending, failed });
        }
    }

    /// Applies `resolution` to `id`, or with `apply_to_all` to every item
    /// awaiting a decision, which also becomes the global policy.
    fn resolve(&mut self, id: TransferId, resolution: Resolution) {
        if !resolution.apply_to_all {
            self.resolve_one(id, resolution.action);
            return;
        }
        self.policy = match resolution.action {
            ResolveAction::Overwrite => ConflictPolicy::Overwrite,
            ResolveAction::Resume => ConflictPolicy::Resume,
            ResolveAction::Skip => ConflictPolicy::Skip,
        };
        let pending: Vec<TransferId> = self
            .items
            .values()
            .filter(|i| matches!(i.state, TransferState::AwaitingDecision(_)))
            .map(|i| i.id)
            .collect();
        for i in pending {
            self.resolve_one(i, resolution.action);
        }
    }

    fn resolve_one(&mut self, id: TransferId, action: ResolveAction) {
        let policy = match action {
            ResolveAction::Overwrite => ConflictPolicy::Overwrite,
            ResolveAction::Resume => ConflictPolicy::Resume,
            ResolveAction::Skip => {
                self.overrides.remove(&id);
                self.transition(id, TransferState::Skipped(USER_SKIP.to_string()));
                return;
            }
        };
        self.overrides.insert(id, policy);
        self.transition(id, TransferState::Queued);
    }

    /// Tracks a newly persisted item and emits `Added`.
    fn add_item(&mut self, item: TransferItem) {
        if item.state == TransferState::Queued {
            self.queued
                .entry(item.spec.site.clone())
                .or_default()
                .push(item.id, item.is_dir == Some(true));
        }
        self.items.insert(item.id, item.clone());
        self.emit(Event::Added(item));
    }

    /// Persists and applies `state`, then emits `StateChanged`. If persisting
    /// fails, the item becomes `Failed` with the store error instead.
    /// An item that becomes terminal is reported to its parent directory.
    /// Returns whether `state` itself was applied.
    fn transition(&mut self, id: TransferId, state: TransferState) -> bool {
        let (state, applied) = match self.store.set_state(id, &state) {
            Ok(()) => (state, true),
            Err(_) if matches!(state, TransferState::Failed(_)) => (state, false),
            Err(e) => {
                let failed = TransferState::Failed(e);
                let _ = self.store.set_state(id, &failed);
                (failed, false)
            }
        };
        let mut finished_child_of = None;
        if let Some(item) = self.items.get_mut(&id) {
            let was_queued = item.state == TransferState::Queued;
            let is_queued = state == TransferState::Queued;
            if was_queued != is_queued {
                let queue = self.queued.entry(item.spec.site.clone()).or_default();
                if is_queued {
                    queue.push(id, item.is_dir == Some(true));
                } else {
                    queue.remove(id);
                }
            }
            if !item.state.is_terminal() && state.is_terminal() {
                finished_child_of = item.parent;
            }
            item.state = state.clone();
        }
        let failed = matches!(state, TransferState::Failed(_));
        self.emit(Event::StateChanged { id, state });
        if let Some(parent) = finished_child_of {
            self.child_finished(parent, failed);
        }
        applied
    }

    /// Accounts for a child of `dir` that reached a terminal state. When it
    /// was the last one, `dir` finishes too (which recurses to its parent).
    /// A directory that is not counting (paused or finished) is left alone.
    fn child_finished(&mut self, dir: TransferId, failed: bool) {
        let Some(progress) = self.dirs.get_mut(&dir) else {
            return;
        };
        progress.pending = progress.pending.saturating_sub(1);
        if failed {
            progress.failed += 1;
        }
        if progress.pending > 0 {
            return;
        }
        let failed = progress.failed;
        self.dirs.remove(&dir);
        self.finish_dir(dir, failed);
    }

    /// Finishes a directory: `Done`, or `Failed` if `failed` children failed.
    /// Children that were skipped or cancelled do not count as failures.
    fn finish_dir(&mut self, dir: TransferId, failed: usize) {
        let state = if failed == 0 {
            TransferState::Done
        } else {
            TransferState::Failed(Error::new(ErrorKind::Io, format!("{failed} items failed")))
        };
        self.transition(dir, state);
    }

    /// Starts `Queued` items while their site has free workers: per site,
    /// items not known to be directories first, then directories, each
    /// lowest id first.
    fn dispatch(&mut self) {
        let ready: Vec<SiteId> = self
            .sites
            .iter()
            .filter(|(_, s)| s.free() > 0)
            .map(|(id, _)| id.clone())
            .collect();
        for site_id in ready {
            while self.sites.get(&site_id).is_some_and(|s| s.free() > 0) {
                let Some(id) = self.queued.get_mut(&site_id).and_then(SiteQueue::pop) else {
                    break;
                };
                let (Some(item), Some(site)) = (self.items.get(&id), self.sites.get(&site_id))
                else {
                    continue;
                };
                let route = Route::new(&item.spec, &site.endpoints);
                if !self.transition(id, TransferState::Active) {
                    continue;
                }
                if let Some(site) = self.sites.get_mut(&site_id) {
                    site.busy += 1;
                }
                self.running.insert(
                    id,
                    Running {
                        cancel: CancellationToken::new(),
                        intent: None,
                    },
                );
                let policy = self.overrides.remove(&id).unwrap_or(self.policy);
                worker::spawn_prepare(id, route, policy, self.worker_tx.clone());
            }
        }
    }

    /// Frees the worker slot held by `id`.
    fn free_slot(&mut self, id: TransferId) {
        self.running.remove(&id);
        if let Some(item) = self.items.get(&id)
            && let Some(site) = self.sites.get_mut(&item.spec.site)
        {
            site.busy = site.busy.saturating_sub(1);
        }
    }

    /// Moves an item that holds a worker slot to `state`, frees the slot and
    /// dispatches the next item.
    fn release(&mut self, id: TransferId, state: TransferState) {
        self.transition(id, state);
        self.free_slot(id);
        self.dispatch();
    }

    /// The pending control intent of the item holding a worker slot, if any.
    fn intent(&self, id: TransferId) -> Option<Intent> {
        self.running.get(&id).and_then(|r| r.intent)
    }

    fn handle_worker(&mut self, msg: WorkerMsg) {
        match msg {
            WorkerMsg::Prepared { id, route, result } => {
                let intent = self.intent(id);
                match (result.map(|p| *p), intent) {
                    (Ok(Prepared::Dir { entries }), intent) => {
                        self.free_slot(id);
                        self.expand(id, entries, intent);
                        self.dispatch();
                    }
                    // Stopped before copying anything: the prepare result is
                    // dropped and the item runs from scratch when restarted.
                    (_, Some(Intent::Pause)) => {
                        // Recorded as paused from `Queued`, so a resume
                        // applies the global policy (no copy to continue).
                        let _ = self.store.set_state(id, &TransferState::Queued);
                        self.release(id, TransferState::Paused);
                    }
                    (_, Some(Intent::Cancel)) => self.release(id, TransferState::Cancelled),
                    (_, Some(Intent::Requeue)) => self.release(id, TransferState::Queued),
                    (Ok(Prepared::File { src, dst, decision }), None) => {
                        self.on_file_prepared(id, route, src, dst, decision)
                    }
                    (Ok(Prepared::Symlink), None) => {
                        self.release(id, TransferState::Skipped(SYMLINK_SKIP.to_string()))
                    }
                    (Err(e), None) => self.release(id, TransferState::Failed(e)),
                }
            }
            WorkerMsg::Copied { id, bytes, result } => {
                let intent = self.intent(id);
                self.tracker.unregister(id);
                if let Some(item) = self.items.get_mut(&id) {
                    item.bytes_done = bytes;
                }
                let state = match (result, self.store.set_bytes_done(id, bytes)) {
                    (Ok(CopyOutcome::Completed), Ok(())) => TransferState::Done,
                    (Ok(CopyOutcome::Cancelled), Ok(())) => match intent {
                        Some(Intent::Pause) => TransferState::Paused,
                        Some(Intent::Requeue) => {
                            self.overrides.insert(id, ConflictPolicy::Resume);
                            TransferState::Queued
                        }
                        Some(Intent::Cancel) | None => TransferState::Cancelled,
                    },
                    (Ok(_), Err(store_err)) => TransferState::Failed(store_err),
                    (Err(e), Ok(())) => TransferState::Failed(e),
                    // Keep the copy's own error; the store error goes into its detail.
                    (Err(e), Err(store_err)) => {
                        TransferState::Failed(with_store_detail(e, &store_err))
                    }
                };
                self.release(id, state);
            }
        }
    }

    /// Inserts one child per entry of the listed directory `id`. The
    /// directory stays `Active` until its children finish, or becomes `Done`
    /// right away if none is pending. If it was paused or cancelled while
    /// listing, its children are created (and it ends) paused or cancelled.
    fn expand(&mut self, id: TransferId, entries: Vec<RemoteEntry>, intent: Option<Intent>) {
        let initial = match intent {
            Some(Intent::Pause) => TransferState::Paused,
            Some(Intent::Cancel) => TransferState::Cancelled,
            Some(Intent::Requeue) | None => TransferState::Queued,
        };
        if let Err(e) = self.store.set_meta(id, true, None) {
            self.transition(id, TransferState::Failed(e));
            return;
        }
        let Some(dir) = self.items.get_mut(&id) else {
            return;
        };
        dir.is_dir = Some(true);
        dir.size = None;
        let parent = dir.spec.clone();
        let children: Vec<NewChild> = entries
            .iter()
            .map(|e| NewChild {
                spec: TransferSpec {
                    site: parent.site.clone(),
                    direction: parent.direction,
                    local: parent.local.join(&e.name),
                    remote: parent.remote.join(&e.name),
                },
                is_dir: e.kind == EntryKind::Dir,
                size: (e.kind == EntryKind::File).then_some(e.size),
                state: if e.kind == EntryKind::Symlink {
                    TransferState::Skipped(SYMLINK_SKIP.to_string())
                } else {
                    initial.clone()
                },
            })
            .collect();
        let ids = match self.store.insert_children(id, &children) {
            Ok(ids) => ids,
            Err(e) => {
                self.transition(id, TransferState::Failed(e));
                return;
            }
        };
        let pending = children.iter().filter(|c| !c.state.is_terminal()).count();
        if pending > 0 && initial == TransferState::Queued {
            self.dirs.insert(id, DirProgress { pending, failed: 0 });
        }
        for (child_id, child) in ids.into_iter().zip(children) {
            self.add_item(TransferItem {
                id: child_id,
                parent: Some(id),
                spec: child.spec,
                is_dir: Some(child.is_dir),
                size: child.size,
                bytes_done: 0,
                state: child.state,
            });
        }
        if initial == TransferState::Cancelled {
            self.transition(id, TransferState::Cancelled);
        } else if pending == 0 {
            self.transition(id, TransferState::Done);
        } else if initial == TransferState::Paused {
            self.transition(id, TransferState::Paused);
        }
    }

    fn on_file_prepared(
        &mut self,
        id: TransferId,
        route: Route,
        src: RemoteEntry,
        dst: Option<RemoteEntry>,
        decision: Decision,
    ) {
        if let Err(e) = self.store.set_meta(id, false, Some(src.size)) {
            self.release(id, TransferState::Failed(e));
            return;
        }
        if let Some(item) = self.items.get_mut(&id) {
            item.is_dir = Some(false);
            item.size = Some(src.size);
        }
        match decision {
            Decision::Write { offset } => {
                let cancel = self
                    .running
                    .get(&id)
                    .map(|r| r.cancel.clone())
                    .unwrap_or_default();
                let counter = self.tracker.register(id, Some(src.size), offset);
                worker::spawn_copy(
                    id,
                    route,
                    src,
                    offset,
                    self.buf_size,
                    counter,
                    cancel,
                    self.worker_tx.clone(),
                );
            }
            Decision::Skip(reason) => self.release(id, TransferState::Skipped(reason)),
            Decision::Ask => {
                let info = ConflictInfo {
                    src_size: src.size,
                    dst_size: dst.as_ref().map_or(0, |d| d.size),
                    src_modified: src.modified,
                    dst_modified: dst.as_ref().and_then(|d| d.modified),
                };
                self.release(id, TransferState::AwaitingDecision(info));
            }
        }
    }

    /// The only place that emits `Event::Progress`. Byte counts are persisted
    /// best-effort here; the final count is persisted (strictly) at completion.
    fn tick(&mut self) {
        let updates = self.tracker.sample(Instant::now());
        if updates.is_empty() {
            return;
        }
        let mut batch = Vec::with_capacity(updates.len());
        for u in &updates {
            if let Some(item) = self.items.get_mut(&u.id) {
                item.bytes_done = u.bytes_done;
            }
            batch.push((u.id, u.bytes_done));
        }
        let _ = self.store.set_bytes_done_many(&batch);
        self.emit(Event::Progress(updates));
    }
}

fn is_retryable(state: &TransferState) -> bool {
    matches!(state, TransferState::Failed(_) | TransferState::Cancelled)
}

#[cfg(test)]
mod tests {
    use hoprs_core::ErrorKind;

    use super::*;

    #[test]
    fn store_error_goes_into_the_copy_error_detail() {
        let store_err = Error::new(ErrorKind::Io, "Queue database error").with_detail("disk full");
        let copy = Error::new(ErrorKind::Permission, "denied");
        let e = with_store_detail(copy, &store_err);
        assert_eq!(
            (e.kind, e.message.as_str()),
            (ErrorKind::Permission, "denied")
        );
        assert_eq!(
            e.detail.as_deref(),
            Some("queue store: Queue database error: disk full")
        );

        let copy = Error::new(ErrorKind::Io, "read failed").with_detail("eof");
        let e = with_store_detail(copy, &store_err);
        assert_eq!(
            e.detail.as_deref(),
            Some("eof; queue store: Queue database error: disk full")
        );
    }
}
