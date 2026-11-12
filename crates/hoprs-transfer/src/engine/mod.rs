//! The transfer engine: a single actor task that owns the queue and
//! dispatches per-site workers. `TransferEngine` is a cheap, cloneable handle.

mod actor;
mod control;
mod dirs;
mod recovery;
mod worker;

use std::sync::Arc;
use std::time::Duration;

use hoprs_core::{Error, ErrorKind, Result, SiteId};
use hoprs_remote::RemoteFs;
use tokio::sync::{broadcast, mpsc, oneshot};

use crate::model::{ConflictPolicy, TransferId, TransferItem, TransferSpec, TransferState};
use crate::progress::ProgressUpdate;
use crate::store::QueueStore;

use actor::Command;
use control::Control;

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
    ///
    /// Items left unfinished by a previous run are recovered as `Paused`
    /// (see `QueueStore::recover`); nothing resumes automatically.
    pub fn start(mut store: QueueStore, config: EngineConfig) -> TransferEngine {
        let recovered = recovery::recover(&mut store);
        let (commands, command_rx) = mpsc::channel(COMMAND_CAPACITY);
        let (events, _) = broadcast::channel(EVENT_CAPACITY);
        tokio::spawn(actor::run(
            store,
            recovered,
            config,
            command_rx,
            events.clone(),
        ));
        TransferEngine { commands, events }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    /// Registers (or replaces) the endpoints of a site; its queued items become dispatchable.
    pub async fn register_site(&self, site: SiteId, endpoints: SiteEndpoints) {
        self.request(|reply| Command::RegisterSite {
            site,
            endpoints,
            reply,
        })
        .await;
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
    /// file paused while copying continues from its partial destination (one
    /// that was replacing an existing file starts over); one paused before
    /// its copy started runs under the global policy.
    pub async fn resume(&self, id: TransferId) -> Result<()> {
        self.control(id, Control::Resume).await
    }

    /// Cancels a transfer (a folder: with its pending descendants). Partial
    /// destination files are kept.
    pub async fn cancel(&self, id: TransferId) -> Result<()> {
        self.control(id, Control::Cancel).await
    }

    /// Requeues a failed or cancelled transfer (a folder: with its failed or
    /// cancelled descendants). Each item runs under its stored continuation
    /// when it has one, never skipped because of its partial destination:
    /// - a cancelled item keeps the continuation of the run it stopped:
    ///   `Resume` (it continues its partial destination), or `Overwrite`
    ///   when that run was replacing an existing file (it starts over);
    /// - a failed item whose run held any continuation (its copy had been
    ///   decided, or it ran under an override such as a user `resolve`)
    ///   gets `Overwrite`: it starts over;
    /// - any other item runs under the global policy.
    ///
    /// A child that failed because its name repeats an earlier entry of
    /// its folder's listing is never retried: retrying it returns
    /// `Conflict`, and retrying the folder leaves it failed.
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
        self.request(|reply| Command::SetPolicy { policy, reply })
            .await;
    }

    /// Stops dispatching the items of `site` (a user pause); transfers
    /// already running continue. Has no effect on an unknown site. A network
    /// failure on a user-paused site still emits `SitePaused`.
    pub async fn pause_site(&self, site: SiteId) {
        self.request(|reply| Command::PauseSite { site, reply })
            .await;
    }

    /// Dispatches the items of `site` again: clears both its user pause and
    /// its network pause (an explicit "go" from the user; during a real
    /// outage the next network error pauses the site again). Has no effect
    /// on an unknown site.
    pub async fn resume_site(&self, site: SiteId) {
        self.request(|reply| Command::ResumeSite { site, reply })
            .await;
    }

    /// Clears the network pause of `site` (set by a network error, announced
    /// by `Event::SitePaused`). The site dispatches again unless the user
    /// also paused it (`pause_site`). Has no effect on an unknown site.
    ///
    /// Integration: run [`spawn_site_watcher`](crate::spawn_site_watcher) for every connected
    /// site (any protocol). It calls this when the connection returns with a new generation,
    /// after a grace period when a request failed but the connection stayed up, and when the
    /// event receiver lags.
    ///
    /// Calling it when the site is not network-paused does nothing, so the
    /// task never needs to know whether the site is paused.
    pub async fn network_restored(&self, site: SiteId) {
        self.request(|reply| Command::NetworkRestored { site, reply })
            .await;
    }

    /// Stops the engine: running transfers are cancelled, their items keep
    /// their stored state (so the next start recovers them), and this waits
    /// for the actor to finish. Other handles stop working.
    pub async fn shutdown(self) {
        self.request(|reply| Command::Shutdown { reply }).await;
    }

    /// Sends a command whose only reply is an acknowledgement, and waits for
    /// it. Does nothing if the engine has stopped.
    async fn request(&self, command: impl FnOnce(oneshot::Sender<()>) -> Command) {
        let (reply, rx) = oneshot::channel();
        if self.commands.send(command(reply)).await.is_ok() {
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
