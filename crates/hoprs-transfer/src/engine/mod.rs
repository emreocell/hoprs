//! The transfer engine: a single actor task that owns the queue and
//! dispatches per-site workers. `TransferEngine` is a cheap, cloneable handle.

mod actor;
mod worker;

use std::sync::Arc;
use std::time::Duration;

use hoprs_core::{Error, ErrorKind, Result, SiteId};
use hoprs_remote::RemoteFs;
use tokio::sync::{broadcast, mpsc, oneshot};

use crate::model::{ConflictPolicy, TransferId, TransferItem, TransferSpec, TransferState};
use crate::progress::ProgressUpdate;
use crate::store::QueueStore;

use actor::{Command, Control};

const EVENT_CAPACITY: usize = 1024;
const COMMAND_CAPACITY: usize = 64;

/// Engine tuning knobs.
#[derive(Clone, Debug)]
pub struct EngineConfig {
    pub buf_size: usize,
    pub progress_interval: Duration,
    pub default_policy: ConflictPolicy,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            buf_size: 256 * 1024,
            progress_interval: Duration::from_millis(100),
            default_policy: ConflictPolicy::Ask,
        }
    }
}

/// Filesystems and worker count of one site. `workers` is clamped to `1..=16`.
#[derive(Clone)]
pub struct SiteEndpoints {
    pub local: Arc<dyn RemoteFs>,
    pub remote: Arc<dyn RemoteFs>,
    pub workers: usize,
}

/// User answer to a conflict.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ResolveAction {
    Overwrite,
    Skip,
    Resume,
}

/// A conflict resolution, optionally applied to every pending conflict.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Resolution {
    pub action: ResolveAction,
    pub apply_to_all: bool,
}

/// Notifications broadcast by the engine.
#[derive(Clone, Debug)]
pub enum Event {
    Added(TransferItem),
    StateChanged {
        id: TransferId,
        state: TransferState,
    },
    Progress(Vec<ProgressUpdate>),
    SitePaused {
        site: SiteId,
        error: Error,
    },
}

/// Handle to the engine actor.
#[derive(Clone)]
pub struct TransferEngine {
    commands: mpsc::Sender<Command>,
    events: broadcast::Sender<Event>,
}

fn stopped() -> Error {
    Error::new(ErrorKind::Io, "Transfer engine stopped")
}

impl TransferEngine {
    /// Spawns the engine actor. Must be called inside a tokio runtime.
    pub fn start(store: QueueStore, config: EngineConfig) -> TransferEngine {
        let (commands, command_rx) = mpsc::channel(COMMAND_CAPACITY);
        let (events, _) = broadcast::channel(EVENT_CAPACITY);
        tokio::spawn(actor::run(store, config, command_rx, events.clone()));
        TransferEngine { commands, events }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    /// Registers (or replaces) the endpoints of a site; its queued items become dispatchable.
    pub async fn register_site(&self, site: SiteId, endpoints: SiteEndpoints) {
        let (reply, rx) = oneshot::channel();
        if self
            .commands
            .send(Command::RegisterSite {
                site,
                endpoints,
                reply,
            })
            .await
            .is_ok()
        {
            let _ = rx.await;
        }
    }

    /// Adds a transfer to the queue and returns its id.
    pub async fn enqueue(&self, spec: TransferSpec) -> Result<TransferId> {
        let (reply, rx) = oneshot::channel();
        self.commands
            .send(Command::Enqueue { spec, reply })
            .await
            .map_err(|_| stopped())?;
        rx.await.map_err(|_| stopped())?
    }

    /// Pauses a transfer (a folder: with its pending descendants). An active
    /// file stops after its current block and keeps its partial destination.
    pub async fn pause(&self, id: TransferId) -> Result<()> {
        self.control(id, Control::Pause).await
    }

    /// Resumes a paused transfer (a folder: with its paused descendants). A
    /// file paused while copying continues from its partial destination.
    pub async fn resume(&self, id: TransferId) -> Result<()> {
        self.control(id, Control::Resume).await
    }

    /// Cancels a transfer (a folder: with its pending descendants). Partial
    /// destination files are kept.
    pub async fn cancel(&self, id: TransferId) -> Result<()> {
        self.control(id, Control::Cancel).await
    }

    /// Requeues a failed or cancelled transfer (a folder: with its failed or
    /// cancelled descendants).
    pub async fn retry(&self, id: TransferId) -> Result<()> {
        self.control(id, Control::Retry).await
    }

    /// Answers the conflict of an item awaiting a decision.
    pub async fn resolve(&self, id: TransferId, resolution: Resolution) -> Result<()> {
        self.control(id, Control::Resolve(resolution)).await
    }

    async fn control(&self, id: TransferId, op: Control) -> Result<()> {
        let (reply, rx) = oneshot::channel();
        self.commands
            .send(Command::Control { id, op, reply })
            .await
            .map_err(|_| stopped())?;
        rx.await.map_err(|_| stopped())?
    }

    /// Sets the conflict policy used by items dispatched from now on.
    pub async fn set_policy(&self, policy: ConflictPolicy) {
        let (reply, rx) = oneshot::channel();
        if self
            .commands
            .send(Command::SetPolicy { policy, reply })
            .await
            .is_ok()
        {
            let _ = rx.await;
        }
    }

    /// All items, ordered by id.
    pub async fn snapshot(&self) -> Vec<TransferItem> {
        let (reply, rx) = oneshot::channel();
        if self
            .commands
            .send(Command::Snapshot { reply })
            .await
            .is_err()
        {
            return Vec::new();
        }
        rx.await.unwrap_or_default()
    }
}
