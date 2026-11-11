//! Shared helpers for engine integration tests.
#![allow(dead_code)]

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use hoprs_core::{RemotePath, SiteId};
use hoprs_remote::{MemFs, RemoteFs};
use hoprs_transfer::{
    Direction, EngineConfig, Event, QueueStore, SiteEndpoints, TransferEngine, TransferSpec,
};
use tokio::sync::broadcast::Receiver;
use tokio::sync::broadcast::error::RecvError;

const WAIT_TIMEOUT: Duration = Duration::from_secs(5);

pub fn site() -> SiteId {
    SiteId("s".to_string())
}

pub fn path(s: &str) -> RemotePath {
    RemotePath::new(s).expect("valid test path")
}

pub fn mtime() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000)
}

/// Deterministic pseudo-random bytes.
pub fn data(len: usize) -> Vec<u8> {
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect()
}

pub fn spec(direction: Direction, local: &str, remote: &str) -> TransferSpec {
    TransferSpec {
        site: site(),
        direction,
        local: path(local),
        remote: path(remote),
    }
}

/// Starts an engine on an in-memory store and subscribes before anything happens.
pub fn start_engine(config: EngineConfig) -> (TransferEngine, Receiver<Event>) {
    let store = QueueStore::open_in_memory().expect("in-memory store");
    let engine = TransferEngine::start(store, config);
    let rx = engine.subscribe();
    (engine, rx)
}

/// Engine with caller-supplied endpoints, registered as site `"s"`.
pub async fn engine_with(
    local: Arc<dyn RemoteFs>,
    remote: Arc<dyn RemoteFs>,
    workers: usize,
    config: EngineConfig,
) -> (TransferEngine, Receiver<Event>) {
    let (engine, rx) = start_engine(config);
    engine
        .register_site(
            site(),
            SiteEndpoints {
                local,
                remote,
                workers,
            },
        )
        .await;
    (engine, rx)
}

/// Engine over two fresh `MemFs` (local, remote), registered as site `"s"`.
///
/// Async because `register_site` is async.
pub async fn engine_with_memfs(workers: usize) -> (TransferEngine, MemFs, MemFs, Receiver<Event>) {
    let local = MemFs::new();
    let remote = MemFs::new();
    let (engine, rx) = engine_with(
        Arc::new(local.clone()),
        Arc::new(remote.clone()),
        workers,
        EngineConfig::default(),
    )
    .await;
    (engine, local, remote, rx)
}

/// Returns the first event matching `f`; panics after 5 s.
pub async fn wait_for<F: FnMut(&Event) -> bool>(rx: &mut Receiver<Event>, mut f: F) -> Event {
    let fut = async {
        loop {
            match rx.recv().await {
                Ok(ev) if f(&ev) => return ev,
                Ok(_) | Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => panic!("event channel closed"),
            }
        }
    };
    tokio::time::timeout(WAIT_TIMEOUT, fut)
        .await
        .expect("timed out waiting for event")
}
