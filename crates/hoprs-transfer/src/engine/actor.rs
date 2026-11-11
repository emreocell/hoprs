//! The engine actor: owns the queue store, the in-memory items, the per-site
//! worker slots and the progress tracker. Never awaits I/O itself: workers
//! run as spawned tasks and report back through `WorkerMsg`.

use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant};

use hoprs_core::{Result, SiteId};
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
use crate::store::QueueStore;

const MAX_WORKERS: usize = 16;

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

struct Actor {
    store: QueueStore,
    buf_size: usize,
    policy: ConflictPolicy,
    events: broadcast::Sender<Event>,
    items: BTreeMap<TransferId, TransferItem>,
    sites: HashMap<SiteId, Site>,
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
                let item = TransferItem {
                    id,
                    parent: None,
                    spec,
                    is_dir: None,
                    size: None,
                    bytes_done: 0,
                    state: TransferState::Queued,
                };
                self.items.insert(id, item.clone());
                self.emit(Event::Added(item));
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

    /// Persists and applies `state`, then emits `StateChanged`. If persisting
    /// fails, the item becomes `Failed` with the store error instead.
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
        if let Some(item) = self.items.get_mut(&id) {
            item.state = state.clone();
        }
        self.emit(Event::StateChanged { id, state });
        applied
    }

    /// Starts `Queued` items, lowest id first, while their site has free workers.
    fn dispatch(&mut self) {
        let candidates: Vec<TransferId> = self
            .items
            .values()
            .filter(|i| i.state == TransferState::Queued)
            .map(|i| i.id)
            .collect();
        for id in candidates {
            if !self.sites.values().any(|s| s.free() > 0) {
                break;
            }
            let spec = &self.items[&id].spec;
            let Some(site) = self.sites.get(&spec.site) else {
                continue;
            };
            if site.free() == 0 {
                continue;
            }
            let route = Route::new(spec, &site.endpoints);
            let site_id = spec.site.clone();
            if !self.transition(id, TransferState::Active) {
                continue;
            }
            if let Some(site) = self.sites.get_mut(&site_id) {
                site.busy += 1;
            }
            worker::spawn_prepare(id, route, self.policy, self.worker_tx.clone());
        }
    }

    /// Moves an item that holds a worker slot to `state`, frees the slot and
    /// dispatches the next item.
    fn release(&mut self, id: TransferId, state: TransferState) {
        self.transition(id, state);
        if let Some(item) = self.items.get(&id)
            && let Some(site) = self.sites.get_mut(&item.spec.site)
        {
            site.busy = site.busy.saturating_sub(1);
        }
        self.dispatch();
    }

    fn handle_worker(&mut self, msg: WorkerMsg) {
        match msg {
            WorkerMsg::Prepared { id, route, result } => match result {
                Ok(prepared) => self.on_prepared(id, route, *prepared),
                Err(e) => self.release(id, TransferState::Failed(e)),
            },
            WorkerMsg::Copied { id, bytes, result } => {
                self.tracker.unregister(id);
                if let Some(item) = self.items.get_mut(&id) {
                    item.bytes_done = bytes;
                }
                let state = match self.store.set_bytes_done(id, bytes) {
                    Err(e) => TransferState::Failed(e),
                    Ok(()) => match result {
                        Ok(CopyOutcome::Completed) => TransferState::Done,
                        Ok(CopyOutcome::Cancelled) => TransferState::Cancelled,
                        Err(e) => TransferState::Failed(e),
                    },
                };
                self.release(id, state);
            }
        }
    }

    fn on_prepared(&mut self, id: TransferId, route: Route, prepared: Prepared) {
        let Prepared { src, dst, decision } = prepared;
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
        for u in &updates {
            if let Some(item) = self.items.get_mut(&u.id) {
                item.bytes_done = u.bytes_done;
            }
            let _ = self.store.set_bytes_done(u.id, u.bytes_done);
        }
        self.emit(Event::Progress(updates));
    }
}
