//! Worker tasks: the I/O of one file transfer, run off the actor.
//!
//! A file goes through two phases, each a spawned task that reports back to
//! the actor: `prepare` (stat source and destination, apply the conflict
//! policy) and `copy` (stream the bytes). The actor holds the worker slot
//! across both.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use hoprs_core::{EntryKind, Error, ErrorKind, RemoteEntry, RemotePath, Result};
use hoprs_remote::RemoteFs;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::SiteEndpoints;
use crate::conflict::{Decision, decide};
use crate::copy::{CopyOutcome, copy_file};
use crate::model::{ConflictPolicy, Direction, TransferId, TransferSpec};

/// Source and destination of one transfer, resolved from its direction.
#[derive(Clone)]
pub(super) struct Route {
    pub src: Arc<dyn RemoteFs>,
    pub src_path: RemotePath,
    pub dst: Arc<dyn RemoteFs>,
    pub dst_path: RemotePath,
}

impl Route {
    pub fn new(spec: &TransferSpec, endpoints: &SiteEndpoints) -> Route {
        let local = Arc::clone(&endpoints.local);
        let remote = Arc::clone(&endpoints.remote);
        match spec.direction {
            Direction::Upload => Route {
                src: local,
                src_path: spec.local.clone(),
                dst: remote,
                dst_path: spec.remote.clone(),
            },
            Direction::Download => Route {
                src: remote,
                src_path: spec.remote.clone(),
                dst: local,
                dst_path: spec.local.clone(),
            },
        }
    }
}

/// Result of the `prepare` phase.
pub(super) struct Prepared {
    pub src: RemoteEntry,
    pub dst: Option<RemoteEntry>,
    pub decision: Decision,
}

/// Messages from worker tasks to the actor.
pub(super) enum WorkerMsg {
    Prepared {
        id: TransferId,
        route: Route,
        result: Result<Box<Prepared>>,
    },
    Copied {
        id: TransferId,
        /// Absolute byte position reached (offset + bytes copied).
        bytes: u64,
        result: Result<CopyOutcome>,
    },
}

fn panicked() -> Error {
    Error::new(ErrorKind::Io, "Worker panicked")
}

/// Runs one worker phase as its own task and always reports to the actor:
/// the phase's own message, or `on_panic()` if the phase panicked (so its
/// slot is never leaked).
fn spawn_phase<F, P>(tx: mpsc::UnboundedSender<WorkerMsg>, phase: F, on_panic: P)
where
    F: Future<Output = WorkerMsg> + Send + 'static,
    P: FnOnce() -> WorkerMsg + Send + 'static,
{
    tokio::spawn(async move {
        let msg = tokio::spawn(phase).await.unwrap_or_else(|_| on_panic());
        let _ = tx.send(msg);
    });
}

pub(super) fn spawn_prepare(
    id: TransferId,
    route: Route,
    policy: ConflictPolicy,
    tx: mpsc::UnboundedSender<WorkerMsg>,
) {
    let route_on_panic = route.clone();
    spawn_phase(
        tx,
        async move {
            let result = prepare(&route, policy).await.map(Box::new);
            WorkerMsg::Prepared { id, route, result }
        },
        move || WorkerMsg::Prepared {
            id,
            route: route_on_panic,
            result: Err(panicked()),
        },
    );
}

async fn prepare(route: &Route, policy: ConflictPolicy) -> Result<Prepared> {
    let src = route.src.stat(&route.src_path).await?;
    if src.kind != EntryKind::File {
        return Err(Error::new(ErrorKind::Io, "Source is not a regular file")
            .with_detail(route.src_path.to_string()));
    }
    let dst = match route.dst.stat(&route.dst_path).await {
        Ok(entry) => Some(entry),
        Err(e) if e.kind == ErrorKind::NotFound => None,
        Err(e) => return Err(e),
    };
    let decision = decide(policy, &src, dst.as_ref());
    Ok(Prepared { src, dst, decision })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn spawn_copy(
    id: TransferId,
    route: Route,
    src: RemoteEntry,
    offset: u64,
    buf_size: usize,
    counter: Arc<AtomicU64>,
    cancel: CancellationToken,
    tx: mpsc::UnboundedSender<WorkerMsg>,
) {
    let counter_on_panic = Arc::clone(&counter);
    spawn_phase(
        tx,
        async move {
            let result = copy_file(
                route.src.as_ref(),
                &src,
                route.dst.as_ref(),
                &route.dst_path,
                offset,
                buf_size,
                &counter,
                &cancel,
            )
            .await;
            let bytes = counter.load(Ordering::Relaxed);
            WorkerMsg::Copied { id, bytes, result }
        },
        move || WorkerMsg::Copied {
            id,
            bytes: counter_on_panic.load(Ordering::Relaxed),
            result: Err(panicked()),
        },
    );
}
