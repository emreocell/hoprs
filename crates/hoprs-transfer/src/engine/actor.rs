//! The engine actor: owns the queue store, the in-memory items, the per-site
//! worker slots and the progress tracker. Never awaits I/O itself: workers
//! run as spawned tasks and report back through `WorkerMsg`.
//!
//! This module holds the actor's state, its loop, dispatch and the handling
//! of worker results. Control commands live in `control`, directory
//! bookkeeping in `dirs` and start-up recovery in `recovery`.
//!
//! Files are dispatched before directories, so a large tree starts copying
//! before it has been listed completely.
//!
//! A network error pauses the item's whole site: the item goes back to
//! `Queued` (a file continues from its partial destination later) and the
//! site dispatches nothing until `resume_site`. Other sites keep running.
//!
//! At start, items left unfinished by a previous run are loaded as `Paused`
//! (see `recovery::recover`); nothing resumes automatically.
//!
//! The conflict policy an item's next run needs (its *continuation*) is
//! persisted as the item's `next_policy` while it is unfinished: the
//! override a run consumed at dispatch, replaced by the policy that
//! continues its copy once the copy is decided (`Resume`, or `Overwrite`
//! when it replaces an existing file). Pausing, requeueing or recovering an
//! item carries it forward; an item with none runs under the global policy
//! (its destination was never opened for writing).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::{Duration, Instant};

use hoprs_core::{Error, ErrorKind, RemoteEntry, Result, SiteId};
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::time::MissedTickBehavior;
use tokio_util::sync::CancellationToken;

use super::control::Control;
use super::dirs::DirProgress;
use super::worker::{self, Prepared, Route, WorkerMsg};
use super::{EngineConfig, Event, SiteEndpoints};
use crate::conflict::{Decision, decide};
use crate::copy::CopyOutcome;
use crate::model::{
    ConflictInfo, ConflictPolicy, TransferId, TransferItem, TransferSpec, TransferState,
};
use crate::progress::ProgressTracker;
use crate::store::QueueStore;

const MAX_WORKERS: usize = 16;
pub(super) const SYMLINK_SKIP: &str = "symlinks are not followed";
/// How long a stopping engine waits for its cancelled workers to report back.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

fn unknown() -> Error {
    Error::new(ErrorKind::NotFound, "Unknown transfer")
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
    PauseSite {
        site: SiteId,
        reply: oneshot::Sender<()>,
    },
    ResumeSite {
        site: SiteId,
        reply: oneshot::Sender<()>,
    },
    /// Stops the actor; `reply` is answered once it has stopped.
    Shutdown { reply: oneshot::Sender<()> },
}

/// What to do with an item holding a worker slot once its worker reports
/// back, set by a control command that arrived meanwhile.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(super) enum Intent {
    Pause,
    Cancel,
    /// Paused, then resumed before the worker stopped: back to `Queued`.
    Requeue,
}

/// An item holding a worker slot (preparing or copying).
pub(super) struct Running {
    pub(super) cancel: CancellationToken,
    pub(super) intent: Option<Intent>,
    /// Policy for the item's next run if this one is interrupted: the
    /// override this run consumed, until the copy is decided; then the
    /// policy that continues that copy. Mirrors the stored `next_policy`.
    pub(super) retry: Option<ConflictPolicy>,
}

pub(super) struct Site {
    endpoints: SiteEndpoints,
    /// Worker slots currently held by items of this site.
    busy: usize,
    /// Set by `pause_site` or a network error: nothing new is dispatched.
    paused: bool,
}

impl Site {
    fn free(&self) -> usize {
        self.endpoints.workers.saturating_sub(self.busy)
    }
}

/// `Queued` items of one site, in dispatch order: items not known to be
/// directories first, then directories, each by ascending id.
#[derive(Default)]
pub(super) struct SiteQueue {
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

pub(super) struct Actor {
    pub(super) store: QueueStore,
    pub(super) buf_size: usize,
    pub(super) policy: ConflictPolicy,
    pub(super) events: broadcast::Sender<Event>,
    pub(super) items: BTreeMap<TransferId, TransferItem>,
    pub(super) sites: HashMap<SiteId, Site>,
    /// Index of `Queued` items per site (registered or not).
    pub(super) queued: HashMap<SiteId, SiteQueue>,
    /// `Active` expanded directories that still have non-terminal children.
    pub(super) dirs: HashMap<TransferId, DirProgress>,
    /// Items holding a worker slot.
    pub(super) running: HashMap<TransferId, Running>,
    /// One-shot conflict policy for a `Queued` item's next run, consumed at
    /// dispatch. Equal to its stored `next_policy`, except that a `resolve`
    /// answer is only stored once the item is dispatched.
    pub(super) overrides: HashMap<TransferId, ConflictPolicy>,
    pub(super) tracker: ProgressTracker,
    pub(super) worker_tx: mpsc::UnboundedSender<WorkerMsg>,
}

pub(super) async fn run(
    store: QueueStore,
    recovered: Vec<TransferItem>,
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
        items: recovered.into_iter().map(|i| (i.id, i)).collect(),
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

    let shutdown = loop {
        tokio::select! {
            cmd = commands.recv() => match cmd {
                Some(Command::Shutdown { reply }) => break Some(reply),
                Some(cmd) => actor.handle_command(cmd),
                // Every handle was dropped.
                None => break None,
            },
            Some(msg) = worker_rx.recv() => actor.handle_worker(msg),
            _ = ticker.tick() => actor.tick(),
        }
    };
    actor.stop_workers(&mut worker_rx).await;
    // Closes the store before acknowledging.
    drop(actor);
    if let Some(reply) = shutdown {
        let _ = reply.send(());
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
                        self.sites.insert(
                            site,
                            Site {
                                endpoints,
                                busy: 0,
                                paused: false,
                            },
                        );
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
            Command::PauseSite { site, reply } => {
                if let Some(site) = self.sites.get_mut(&site) {
                    site.paused = true;
                }
                let _ = reply.send(());
            }
            Command::ResumeSite { site, reply } => {
                if let Some(site) = self.sites.get_mut(&site) {
                    site.paused = false;
                }
                let _ = reply.send(());
                self.dispatch();
            }
            // `run` takes it out of the command stream, since it ends the actor.
            Command::Shutdown { .. } => unreachable!("shutdown is handled by run"),
        }
    }

    /// Cancels every running worker and waits (at most `SHUTDOWN_GRACE`)
    /// until each has reported back, so that no cancelled copy is still
    /// writing its partial destination once the engine has stopped. What
    /// they report is ignored: stored states are left as they are, for the
    /// next start to recover.
    async fn stop_workers(&mut self, worker_rx: &mut mpsc::UnboundedReceiver<WorkerMsg>) {
        for running in self.running.values() {
            running.cancel.cancel();
        }
        let drain = async {
            while !self.running.is_empty() {
                let Some(msg) = worker_rx.recv().await else {
                    break;
                };
                let (WorkerMsg::Prepared { id, .. } | WorkerMsg::Copied { id, .. }) = msg;
                self.running.remove(&id);
            }
        };
        let _ = tokio::time::timeout(SHUTDOWN_GRACE, drain).await;
    }

    /// Tracks a newly persisted item and emits `Added`.
    pub(super) fn add_item(&mut self, item: TransferItem) {
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
    pub(super) fn transition(&mut self, id: TransferId, state: TransferState) -> bool {
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
        let mut reopened_child_of = None;
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
            if item.state.is_terminal() && !state.is_terminal() {
                let was_failed = matches!(item.state, TransferState::Failed(_));
                reopened_child_of = item.parent.map(|p| (p, was_failed));
            }
            item.state = state.clone();
        }
        let failed = matches!(state, TransferState::Failed(_));
        self.emit(Event::StateChanged { id, state });
        if let Some(parent) = finished_child_of {
            self.child_finished(parent, failed);
        }
        if let Some((parent, was_failed)) = reopened_child_of {
            self.child_reopened(parent, was_failed);
        }
        applied
    }

    /// Starts `Queued` items while their site has free workers: per site,
    /// items not known to be directories first, then directories, each
    /// lowest id first.
    fn dispatch(&mut self) {
        let ready: Vec<SiteId> = self
            .sites
            .iter()
            .filter(|(_, s)| !s.paused && s.free() > 0)
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
                let policy_override = self.overrides.remove(&id);
                // Stored with the `Active` row, so that it survives an
                // interruption before the copy is decided.
                if policy_override.is_some()
                    && let Err(e) = self.store.set_next_policy(id, policy_override)
                {
                    self.transition(id, TransferState::Failed(e));
                    continue;
                }
                self.running.insert(
                    id,
                    Running {
                        cancel: CancellationToken::new(),
                        intent: None,
                        retry: policy_override,
                    },
                );
                let policy = policy_override.unwrap_or(self.policy);
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

    /// Puts an item whose worker hit a network error back in the queue,
    /// under its continuation, and pauses its site. `SitePaused` is emitted
    /// only if the site was not paused already.
    fn network_requeue(&mut self, id: TransferId, error: Error) {
        let site_id = self.items.get(&id).map(|i| i.spec.site.clone());
        self.requeue(id);
        self.free_slot(id);
        if let Some(site_id) = site_id
            && let Some(site) = self.sites.get_mut(&site_id)
            && !site.paused
        {
            site.paused = true;
            self.emit(Event::SitePaused {
                site: site_id,
                error,
            });
        }
        self.dispatch();
    }

    /// Moves an item that holds a worker slot back to `Queued`, with its
    /// continuation (still stored, as `Active` keeps it) as the override of
    /// its next run. Does not free the slot.
    fn requeue(&mut self, id: TransferId) {
        let retry = self.running.get(&id).and_then(|r| r.retry);
        if self.transition(id, TransferState::Queued)
            && let Some(policy) = retry
        {
            self.overrides.insert(id, policy);
        }
    }

    /// Moves an item whose run was stopped by the control `intent` (or
    /// cancelled without one) to its next state, frees its slot and
    /// dispatches. A pause or requeue keeps its stored continuation.
    fn release_stopped(&mut self, id: TransferId, intent: Option<Intent>) {
        match intent {
            Some(Intent::Pause) => self.release(id, TransferState::Paused),
            Some(Intent::Requeue) => {
                self.requeue(id);
                self.free_slot(id);
                self.dispatch();
            }
            Some(Intent::Cancel) | None => self.release(id, TransferState::Cancelled),
        }
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
                    // dropped and the next run prepares again, under the
                    // override this run consumed.
                    (_, Some(intent)) => self.release_stopped(id, Some(intent)),
                    (Ok(Prepared::File { src, dst, decision }), None) => {
                        self.on_file_prepared(id, route, src, dst, decision)
                    }
                    (Ok(Prepared::Symlink), None) => {
                        self.release(id, TransferState::Skipped(SYMLINK_SKIP.to_string()))
                    }
                    (Err(e), None) if e.kind == ErrorKind::Network => self.network_requeue(id, e),
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
                    (Ok(CopyOutcome::Cancelled), Ok(())) => {
                        self.release_stopped(id, intent);
                        return;
                    }
                    // A pending control command wins over the network error.
                    (Err(e), Ok(())) if e.kind == ErrorKind::Network && intent.is_some() => {
                        self.release_stopped(id, intent);
                        return;
                    }
                    (Err(e), Ok(())) if e.kind == ErrorKind::Network => {
                        self.network_requeue(id, e);
                        return;
                    }
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
        // Prepared under `Ask` before the user chose a policy for all
        // conflicts: apply that policy instead of asking again.
        let decision = match decision {
            Decision::Ask if self.policy != ConflictPolicy::Ask => {
                decide(self.policy, &src, dst.as_ref())
            }
            decision => decision,
        };
        match decision {
            Decision::Write { offset } => {
                // The continuation of this copy, stored before the copy can
                // open the destination: the next run (after a pause, a
                // network error or a restart) continues from the partial
                // destination; but replacing an existing file starts over,
                // as the interruption may have come before the destination
                // was truncated.
                let retry = if offset == 0 && dst.is_some() {
                    ConflictPolicy::Overwrite
                } else {
                    ConflictPolicy::Resume
                };
                if let Err(e) = self.store.set_next_policy(id, Some(retry)) {
                    self.release(id, TransferState::Failed(e));
                    return;
                }
                let cancel = match self.running.get_mut(&id) {
                    Some(running) => {
                        running.retry = Some(retry);
                        running.cancel.clone()
                    }
                    None => CancellationToken::new(),
                };
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
