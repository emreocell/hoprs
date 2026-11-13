//! The transfer queue as the panel shows it: the engine's items, kept from a snapshot plus
//! the events that come after it. Pure state and its update; `ui::queue` draws it.
//!
//! Review Focus 2: the engine numbers its events (`seq`). The queue applies an event only
//! if its `seq` is the next one after the last applied. An event at or below it is stale
//! (the snapshot already holds it) and is dropped, so an old `StateChanged` never
//! overwrites a newer state. A gap (events lost to a lagging receiver) or a `Lagged` from
//! the receiver starts a resync: a new snapshot is asked for, events are buffered until it
//! comes, and then only the buffered events past the snapshot's `seq` are applied. Only the
//! latest snapshot request counts; an older one coming back late is dropped.

use std::collections::{BTreeMap, HashMap, HashSet};

use hoprs_core::{Error, SiteId};
use hoprs_transfer::{
    ConflictPolicy, Event, EventKind, ResolveAction, TransferId, TransferItem, TransferState,
};

use super::files::RequestId;
use crate::backend::{ControlOp, TransferSnapshot};

/// Events buffered while a snapshot is on its way; past this the resync starts over (the
/// next snapshot covers what was dropped).
const MAX_BUFFERED: usize = 4096;

/// The queue panel's filter tabs.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum QueueTab {
    /// Running, paused, or waiting for a conflict decision.
    Active,
    Queued,
    Failed,
    /// Done, skipped (by the engine, or a failed row the user skipped) or cancelled.
    Done,
}

impl QueueTab {
    pub const ALL: [QueueTab; 4] = [
        QueueTab::Active,
        QueueTab::Queued,
        QueueTab::Failed,
        QueueTab::Done,
    ];
}

/// One row: an engine item and its last measured speed.
#[derive(Clone, Debug, PartialEq)]
pub struct QueueRow {
    pub item: TransferItem,
    /// From the last `Progress`; 0 once the item is no longer active.
    pub bytes_per_sec: u64,
    /// A `Progress` came for it since the last snapshot.
    pub progressed: bool,
}

impl QueueRow {
    /// Done so far, 0 to 1, when the size is known.
    pub fn fraction(&self) -> Option<f32> {
        let size = self.item.size?;
        if size == 0 {
            return Some(if self.item.state == TransferState::Done {
                1.0
            } else {
                0.0
            });
        }
        Some((self.item.bytes_done.min(size) as f64 / size as f64) as f32)
    }
}

/// A button of a row.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RowAction {
    Pause,
    Resume,
    Cancel,
    Retry,
    /// A failed row: out of the Failed tab, into Done (the engine keeps the item failed;
    /// Retry still works from there).
    Dismiss,
    /// The answers to a conflict.
    Resolve(ResolveAction),
}

/// What a row's status column shows.
#[derive(Clone, Debug, PartialEq)]
pub enum RowStatus<'a> {
    /// A refused control, or the failure itself: in red.
    Error(&'a Error),
    /// A file of known size: the bar, the percent and the speed.
    Progress {
        fraction: f32,
        bytes_per_sec: u64,
    },
    /// A file of unknown size: the bytes so far and the speed.
    Bytes {
        done: u64,
        bytes_per_sec: u64,
    },
    /// A folder whose children are being copied.
    Transferring,
    /// Active, but no byte has moved yet.
    Starting,
    Waiting,
    Paused,
    Conflict,
    Done,
    Skipped(&'a str),
    /// A failed row the user dismissed.
    Dismissed,
    Cancelled,
}

/// What the queue's widgets send.
#[derive(Clone, Debug)]
pub enum QueueMsg {
    SelectTab(QueueTab),
    Action(TransferId, RowAction),
    /// The "Apply to all" box of a conflict row.
    ToggleApplyAll(TransferId),
    ClearFinished,
    DismissError,
    Scrolled {
        offset_y: f32,
    },
}

/// Where the queue stands with the engine's event stream.
#[derive(Clone, Debug)]
enum Sync {
    /// Applying events as they come.
    Live,
    /// Waiting for the snapshot of `request` (none asked yet at start); events wait in
    /// `buffer`.
    Resyncing {
        request: Option<RequestId>,
        buffer: Vec<Event>,
    },
}

/// The queue panel's state.
#[derive(Clone, Debug)]
pub struct Queue {
    rows: BTreeMap<TransferId, QueueRow>,
    /// The `seq` of the last event applied (or of the snapshot).
    last_seq: u64,
    sync: Sync,
    pub tab: QueueTab,
    /// Failed rows the user skipped: shown under Done.
    skipped: HashSet<TransferId>,
    /// Conflict rows whose "Apply to all" is ticked.
    apply_all: HashSet<TransferId>,
    /// A control the engine refused, shown in its row until the row changes.
    row_errors: HashMap<TransferId, Error>,
    /// Queueing, clearing or a drop failed: shown above the rows until dismissed.
    pub error: Option<Error>,
    /// Sites whose transfers stopped on a lost connection, until one runs again.
    paused_sites: HashMap<SiteId, Error>,
    /// "To all" answers sent and not yet acknowledged, by item.
    pending_policy: HashMap<TransferId, ResolveAction>,
    /// The policy an acknowledged "to all" answer set for the rest of the session.
    session_policy: Option<ConflictPolicy>,
    /// Transfer workers per site, from the last snapshot.
    pub workers: usize,
    /// The list's scroll offset.
    pub offset_y: f32,
}

impl Default for Queue {
    fn default() -> Self {
        Self::new()
    }
}

impl Queue {
    /// Empty, waiting for its first snapshot.
    pub fn new() -> Self {
        Self {
            rows: BTreeMap::new(),
            last_seq: 0,
            sync: Sync::Resyncing {
                request: None,
                buffer: Vec::new(),
            },
            tab: QueueTab::Active,
            skipped: HashSet::new(),
            apply_all: HashSet::new(),
            row_errors: HashMap::new(),
            error: None,
            paused_sites: HashMap::new(),
            pending_policy: HashMap::new(),
            session_policy: None,
            workers: 0,
            offset_y: 0.0,
        }
    }

    /// The tab a row belongs to.
    pub fn tab_of(&self, row: &QueueRow) -> QueueTab {
        match &row.item.state {
            TransferState::Active | TransferState::Paused | TransferState::AwaitingDecision(_) => {
                QueueTab::Active
            }
            TransferState::Queued => QueueTab::Queued,
            TransferState::Failed(_) if self.skipped.contains(&row.item.id) => QueueTab::Done,
            TransferState::Failed(_) => QueueTab::Failed,
            TransferState::Done | TransferState::Skipped(_) | TransferState::Cancelled => {
                QueueTab::Done
            }
        }
    }

    /// The rows of `tab`, by id (the order they were queued).
    pub fn rows_in(&self, tab: QueueTab) -> Vec<&QueueRow> {
        self.rows
            .values()
            .filter(|row| self.tab_of(row) == tab)
            .collect()
    }

    /// How many rows each tab holds, in [`QueueTab::ALL`] order.
    pub fn counts(&self) -> [usize; 4] {
        let mut counts = [0; 4];
        for row in self.rows.values() {
            let i = QueueTab::ALL
                .iter()
                .position(|&t| t == self.tab_of(row))
                .unwrap_or(0);
            counts[i] += 1;
        }
        counts
    }

    #[cfg(test)]
    pub fn row(&self, id: TransferId) -> Option<&QueueRow> {
        self.rows.get(&id)
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// The site's unfinished items, leaving out those under an unfinished folder of the
    /// queue (cancelling the folder takes its pending children with it), by id.
    pub fn unfinished_of(&self, site: &SiteId) -> Vec<TransferId> {
        let unfinished = |id: &TransferId| {
            self.rows
                .get(id)
                .is_some_and(|row| !row.item.state.is_terminal())
        };
        self.rows
            .values()
            .filter(|row| &row.item.spec.site == site && !row.item.state.is_terminal())
            .filter(|row| !row.item.parent.as_ref().is_some_and(unfinished))
            .map(|row| row.item.id)
            .collect()
    }

    /// The sum of the active rows' speeds, in bytes per second.
    pub fn total_speed(&self) -> u64 {
        self.rows
            .values()
            .filter(|row| row.item.state == TransferState::Active)
            .map(|row| row.bytes_per_sec)
            .sum()
    }

    /// Whether anything is terminal (done, skipped, cancelled or failed): what "Clear
    /// finished and failed" removes. The engine's `clear_finished` takes failed items too.
    pub fn has_finished(&self) -> bool {
        self.rows.values().any(|row| row.item.state.is_terminal())
    }

    pub fn apply_all(&self, id: TransferId) -> bool {
        self.apply_all.contains(&id)
    }

    #[cfg(test)]
    pub fn row_error(&self, id: TransferId) -> Option<&Error> {
        self.row_errors.get(&id)
    }

    /// Why the site's transfers stopped, while they are stopped.
    pub fn site_paused(&self, site: &SiteId) -> Option<&Error> {
        self.paused_sites.get(site)
    }

    /// The site connected again (its tab did): its transfers will run.
    pub fn site_restored(&mut self, site: &SiteId) {
        self.paused_sites.remove(site);
    }

    /// Whether the queue waits for a snapshot.
    pub fn resyncing(&self) -> bool {
        matches!(self.sync, Sync::Resyncing { .. })
    }

    /// What the row's status column shows (pure: the view only draws it).
    pub fn status<'a>(&'a self, row: &'a QueueRow) -> RowStatus<'a> {
        let item = &row.item;
        if let Some(error) = self.row_errors.get(&item.id) {
            return RowStatus::Error(error);
        }
        match &item.state {
            TransferState::Active if item.is_dir == Some(true) => RowStatus::Transferring,
            TransferState::Active if item.bytes_done == 0 && !row.progressed => RowStatus::Starting,
            TransferState::Active => match row.fraction() {
                Some(fraction) => RowStatus::Progress {
                    fraction,
                    bytes_per_sec: row.bytes_per_sec,
                },
                None => RowStatus::Bytes {
                    done: item.bytes_done,
                    bytes_per_sec: row.bytes_per_sec,
                },
            },
            TransferState::Queued => RowStatus::Waiting,
            TransferState::Paused => RowStatus::Paused,
            TransferState::AwaitingDecision(_) => RowStatus::Conflict,
            TransferState::Failed(_) if self.skipped.contains(&item.id) => RowStatus::Dismissed,
            TransferState::Failed(error) => RowStatus::Error(error),
            TransferState::Done => RowStatus::Done,
            TransferState::Skipped(reason) => RowStatus::Skipped(reason),
            TransferState::Cancelled => RowStatus::Cancelled,
        }
    }

    /// The conflict policy the engine now runs under for this session because an answer
    /// went "to all" (the engine makes it its global policy), if any.
    pub fn session_policy(&self) -> Option<ConflictPolicy> {
        self.session_policy
    }

    /// Sets (or, with `None`, clears) the policy shown as "this session": the settings
    /// page clears it when it gives the engine a new default policy.
    pub fn set_session_policy(&mut self, policy: Option<ConflictPolicy>) {
        self.session_policy = policy;
    }

    /// The buttons of a row, by its state (a dismissed failure keeps Retry).
    pub fn actions(&self, row: &QueueRow) -> Vec<RowAction> {
        use RowAction::*;
        match &row.item.state {
            TransferState::Active | TransferState::Queued => vec![Pause, Cancel],
            TransferState::Paused => vec![Resume, Cancel],
            TransferState::AwaitingDecision(_) => vec![
                Resolve(ResolveAction::Overwrite),
                Resolve(ResolveAction::Resume),
                Resolve(ResolveAction::Skip),
            ],
            TransferState::Failed(_) if self.skipped.contains(&row.item.id) => vec![Retry],
            TransferState::Failed(_) => vec![Retry, Dismiss],
            TransferState::Cancelled => vec![Retry],
            TransferState::Done | TransferState::Skipped(_) => Vec::new(),
        }
    }

    /// The engine operation for a button, if it takes one. Dismiss on a failed row only
    /// moves it out of the Failed tab; a button the row doesn't show does nothing.
    pub fn control(&mut self, id: TransferId, action: RowAction) -> Option<ControlOp> {
        let row = self.rows.get(&id)?;
        if !self.actions(row).contains(&action) {
            return None;
        }
        self.row_errors.remove(&id);
        Some(match action {
            RowAction::Pause => ControlOp::Pause,
            RowAction::Resume => ControlOp::Resume,
            RowAction::Cancel => ControlOp::Cancel,
            RowAction::Retry => {
                self.skipped.remove(&id);
                ControlOp::Retry
            }
            RowAction::Dismiss => {
                self.skipped.insert(id);
                return None;
            }
            RowAction::Resolve(action) => {
                let apply_to_all = self.apply_all.contains(&id);
                if apply_to_all {
                    self.pending_policy.insert(id, action);
                }
                ControlOp::Resolve(hoprs_transfer::Resolution {
                    action,
                    apply_to_all,
                })
            }
        })
    }

    pub fn toggle_apply_all(&mut self, id: TransferId) {
        if !self.apply_all.remove(&id) {
            self.apply_all.insert(id);
        }
    }

    /// A control's answer from the engine. A refusal shows in the row; an accepted
    /// "to all" answer becomes the session's policy (the engine switched to it).
    pub fn control_done(&mut self, id: TransferId, result: Result<(), Error>) {
        let pending = self.pending_policy.remove(&id);
        match result {
            Ok(()) => {
                if let Some(action) = pending {
                    self.session_policy = Some(match action {
                        ResolveAction::Overwrite => ConflictPolicy::Overwrite,
                        ResolveAction::Resume => ConflictPolicy::Resume,
                        ResolveAction::Skip => ConflictPolicy::Skip,
                    });
                }
            }
            Err(error) => {
                if self.rows.contains_key(&id) {
                    self.row_errors.insert(id, error);
                }
            }
        }
    }

    /// Starts a resync whose snapshot is `request`: the events from now on wait for it,
    /// and any earlier request no longer counts. The events buffered so far are dropped:
    /// they came before this request, so its snapshot holds them.
    pub fn begin_resync(&mut self, request: RequestId) {
        self.sync = Sync::Resyncing {
            request: Some(request),
            buffer: Vec::new(),
        };
    }

    /// An engine event. Returns `true` if the queue must resync (a gap, an unknown item,
    /// or too many events waiting for a snapshot).
    #[must_use]
    pub fn on_event(&mut self, event: Event) -> bool {
        match &mut self.sync {
            Sync::Resyncing { buffer, .. } => {
                if buffer.len() >= MAX_BUFFERED {
                    return true;
                }
                buffer.push(event);
                false
            }
            Sync::Live => self.apply(event),
        }
    }

    /// The snapshot of `request` came. Applied only if it is the latest request, and not
    /// older than what the queue already applied (an equal `seq` is accepted: "Clear
    /// finished" removes rows without an event). An older one cannot come from a running
    /// engine (a stopped one answers `(0, [])`): it is dropped without asking again, so a
    /// stopped engine never makes the queue ask in a loop. The buffered events past its
    /// `seq` are applied after it. Returns `true` if the queue must resync again.
    #[must_use]
    pub fn on_snapshot(&mut self, request: RequestId, snapshot: TransferSnapshot) -> bool {
        let Sync::Resyncing {
            request: Some(pending),
            buffer,
        } = &mut self.sync
        else {
            return false;
        };
        if *pending != request {
            return false;
        }
        if snapshot.seq < self.last_seq {
            return false;
        }
        let buffer = std::mem::take(buffer);
        self.sync = Sync::Live;
        self.last_seq = snapshot.seq;
        self.workers = snapshot.workers;
        self.rows = snapshot
            .items
            .into_iter()
            .map(|item| {
                (
                    item.id,
                    QueueRow {
                        item,
                        bytes_per_sec: 0,
                        progressed: false,
                    },
                )
            })
            .collect();
        let rows = &self.rows;
        // A site with a running item is not paused (any more). The engine's snapshot
        // doesn't say which sites are paused, so none is added here.
        self.paused_sites.retain(|site, _| {
            !rows
                .values()
                .any(|r| &r.item.spec.site == site && r.item.state == TransferState::Active)
        });
        self.skipped.retain(|id| rows.contains_key(id));
        self.apply_all.retain(|id| rows.contains_key(id));
        self.row_errors.retain(|id, _| rows.contains_key(id));
        for event in buffer {
            if self.apply(event) {
                return true;
            }
        }
        false
    }

    /// Applies one event in order; see the module docs.
    fn apply(&mut self, event: Event) -> bool {
        if event.seq <= self.last_seq {
            return false;
        }
        if event.seq != self.last_seq + 1 {
            return true;
        }
        self.last_seq = event.seq;
        match event.kind {
            EventKind::Added(item) => {
                self.rows.insert(
                    item.id,
                    QueueRow {
                        item,
                        bytes_per_sec: 0,
                        progressed: false,
                    },
                );
            }
            EventKind::StateChanged { id, state } => {
                let Some(row) = self.rows.get_mut(&id) else {
                    return true;
                };
                if state == TransferState::Active {
                    self.paused_sites.remove(&row.item.spec.site);
                } else {
                    row.bytes_per_sec = 0;
                }
                if !matches!(state, TransferState::AwaitingDecision(_)) {
                    self.apply_all.remove(&id);
                }
                if !matches!(state, TransferState::Failed(_)) {
                    self.skipped.remove(&id);
                }
                self.row_errors.remove(&id);
                row.item.state = state;
            }
            EventKind::Progress(updates) => {
                for update in updates {
                    if let Some(row) = self.rows.get_mut(&update.id) {
                        row.item.bytes_done = update.bytes_done;
                        row.progressed = true;
                        if update.total.is_some() {
                            row.item.size = update.total;
                        }
                        row.bytes_per_sec = if row.item.state == TransferState::Active {
                            update.bytes_per_sec
                        } else {
                            0
                        };
                    }
                }
            }
            EventKind::SitePaused { site, error } => {
                self.paused_sites.insert(site, error);
            }
        }
        false
    }
}
