//! Worker tasks: the I/O of one file transfer, run off the actor.
//!
//! A file goes through two phases, each a spawned task that reports back to
//! the actor: `prepare` (stat source and destination, apply the conflict
//! policy) and `copy` (stream the bytes). The actor holds the worker slot
//! across both. For a directory, `prepare` creates the destination and lists
//! the source; the actor frees the slot as soon as the listing arrives.
//! Both phases watch the item's cancellation token: `prepare` stops at once
//! (`Prepared::Stopped`); `copy` stops before its next block, and if a read
//! or write is pending it gets `STOP_GRACE` to do so before it is dropped
//! (`CopyOutcome::Cancelled`), so a stalled transfer stops at once on any
//! backend.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

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
pub(super) enum Prepared {
    /// A regular file, ready to copy (or skip, or ask about).
    File {
        src: RemoteEntry,
        dst: Option<RemoteEntry>,
        decision: Decision,
    },
    /// A directory: the destination exists and these are the source entries.
    Dir { entries: Vec<RemoteEntry> },
    /// A symlink, which is never followed.
    Symlink,
    /// Stopped by the item's cancellation token before the destination was
    /// opened for writing.
    Stopped,
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

/// How long a cancelled copy may take to stop on its own (before its next
/// block, shutting its writer down cleanly) before it is dropped. A clean
/// stop is preferred: a pipelined writer may only report at shutdown that
/// an earlier block failed, which makes the copy `Failed` (and its retry
/// start over) instead of `Cancelled`.
const STOP_GRACE: Duration = Duration::from_millis(200);

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

/// Runs `prepare` until it finishes or `cancel` fires (then: `Stopped`).
pub(super) fn spawn_prepare(
    id: TransferId,
    route: Route,
    policy: ConflictPolicy,
    cancel: CancellationToken,
    tx: mpsc::UnboundedSender<WorkerMsg>,
) {
    let route_on_panic = route.clone();
    spawn_phase(
        tx,
        async move {
            let result = tokio::select! {
                biased;
                () = cancel.cancelled() => Ok(Prepared::Stopped),
                result = prepare(&route, policy) => result,
            }
            .map(Box::new);
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
    match src.kind {
        EntryKind::File => {}
        EntryKind::Dir => return prepare_dir(route).await,
        EntryKind::Symlink => return Ok(Prepared::Symlink),
    }
    let dst = match route.dst.stat(&route.dst_path).await {
        Ok(entry) => Some(entry),
        Err(e) if e.kind == ErrorKind::NotFound => None,
        Err(e) => return Err(e),
    };
    if dst.as_ref().is_some_and(|d| d.kind != EntryKind::File) {
        return Err(
            Error::new(ErrorKind::Io, "Destination is not a regular file")
                .with_detail(route.dst_path.to_string()),
        );
    }
    let decision = decide(policy, &src, dst.as_ref());
    Ok(Prepared::File { src, dst, decision })
}

/// Creates the destination directory (an existing one is fine) and lists the source.
async fn prepare_dir(route: &Route) -> Result<Prepared> {
    if let Err(e) = route.dst.mkdir(&route.dst_path).await {
        match route.dst.stat(&route.dst_path).await {
            Ok(entry) if entry.kind == EntryKind::Dir => {}
            _ => return Err(e),
        }
    }
    let entries = route.src.list(&route.src_path).await?;
    // The actor joins these names onto paths, which must not panic.
    if let Some(bad) = entries.iter().find(|e| !is_plain_name(&e.name)) {
        return Err(
            Error::new(ErrorKind::Io, "Invalid name in directory listing")
                .with_detail(format!("{:?} in {}", bad.name, route.src_path)),
        );
    }
    Ok(Prepared::Dir { entries })
}

/// Whether `name` is a single path component accepted by `RemotePath::join`.
fn is_plain_name(name: &str) -> bool {
    !name.is_empty() && !name.contains('/') && name != "." && name != ".."
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
            let copy = copy_file(
                route.src.as_ref(),
                &src,
                route.dst.as_ref(),
                &route.dst_path,
                offset,
                buf_size,
                &counter,
                &cancel,
            );
            tokio::pin!(copy);
            let result = tokio::select! {
                biased;
                result = &mut copy => result,
                () = cancel.cancelled() => {
                    // Stopped while a read or write may be pending: let the
                    // copy stop cleanly if it can, else drop it mid-call.
                    tokio::time::timeout(STOP_GRACE, &mut copy)
                        .await
                        .unwrap_or(Ok(CopyOutcome::Cancelled))
                }
            };
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
