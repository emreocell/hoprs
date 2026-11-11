//! The engine actor: owns the queue store, the in-memory items, the per-site
//! worker slots and the progress tracker. Never awaits I/O itself: workers
//! run as spawned tasks and report back through `WorkerMsg`.
//!
//! Directories expand lazily: a worker lists a directory when it is
//! dispatched, and its entries join the queue as child items. Files are
//! dispatched before directories, so a large tree starts copying before it
//! has been listed completely. A directory stays `Active` (without holding a
//! worker slot) until all of its children are terminal.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::{Duration, Instant};

use hoprs_core::{EntryKind, Error, ErrorKind, RemoteEntry, Result, SiteId};
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::time::MissedTickBehavior;
use tokio_util::sync::CancellationToken;

use super::worker::{self, Prepared, Route, WorkerMsg};
use super::{EngineConfig, Event, SiteEndpoints};
use crate::conflict::Decision;
use crate::copy::CopyOutcome;
use crate::model::{
    ConflictInfo, ConflictPolicy, TransferId, TransferItem, TransferSpec, TransferState,
};
use crate::progress::ProgressTracker;
use crate::store::{NewChild, QueueStore};

const MAX_WORKERS: usize = 16;
const SYMLINK_SKIP: &str = "symlinks are not followed";

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
/// many of its finished children did not end `Done` or `Skipped`.
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
    /// Expanded directories that still have non-terminal children.
    dirs: HashMap<TransferId, DirProgress>,
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
        }
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
        let ok = matches!(state, TransferState::Done | TransferState::Skipped(_));
        self.emit(Event::StateChanged { id, state });
        if let Some(parent) = finished_child_of {
            self.child_finished(parent, ok);
        }
        applied
    }

    /// Accounts for a child of `dir` that reached a terminal state. When it
    /// was the last one, `dir` finishes too (which recurses to its parent).
    fn child_finished(&mut self, dir: TransferId, ok: bool) {
        let Some(progress) = self.dirs.get_mut(&dir) else {
            return;
        };
        progress.pending = progress.pending.saturating_sub(1);
        if !ok {
            progress.failed += 1;
        }
        if progress.pending > 0 {
            return;
        }
        let failed = progress.failed;
        self.dirs.remove(&dir);
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
                worker::spawn_prepare(id, route, self.policy, self.worker_tx.clone());
            }
        }
    }

    /// Frees the worker slot held by `id`.
    fn free_slot(&mut self, id: TransferId) {
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

    fn handle_worker(&mut self, msg: WorkerMsg) {
        match msg {
            WorkerMsg::Prepared { id, route, result } => match result.map(|p| *p) {
                Ok(Prepared::File { src, dst, decision }) => {
                    self.on_file_prepared(id, route, src, dst, decision)
                }
                Ok(Prepared::Dir { entries }) => {
                    self.free_slot(id);
                    self.expand(id, entries);
                    self.dispatch();
                }
                Ok(Prepared::Symlink) => {
                    self.release(id, TransferState::Skipped(SYMLINK_SKIP.to_string()))
                }
                Err(e) => self.release(id, TransferState::Failed(e)),
            },
            WorkerMsg::Copied { id, bytes, result } => {
                self.tracker.unregister(id);
                if let Some(item) = self.items.get_mut(&id) {
                    item.bytes_done = bytes;
                }
                let state = match (result, self.store.set_bytes_done(id, bytes)) {
                    (Ok(CopyOutcome::Completed), Ok(())) => TransferState::Done,
                    (Ok(CopyOutcome::Cancelled), Ok(())) => TransferState::Cancelled,
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
    /// right away if none is pending.
    fn expand(&mut self, id: TransferId, entries: Vec<RemoteEntry>) {
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
                    TransferState::Queued
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
        if pending > 0 {
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
        if pending == 0 {
            self.transition(id, TransferState::Done);
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
                let counter = self.tracker.register(id, Some(src.size), offset);
                worker::spawn_copy(
                    id,
                    route,
                    src,
                    offset,
                    self.buf_size,
                    counter,
                    CancellationToken::new(),
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
