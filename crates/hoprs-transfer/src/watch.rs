//! Site watcher: tells the engine when the network of a site is back.

use std::sync::Arc;
use std::time::Duration;

use hoprs_core::{ConnectionState, SiteId};
use hoprs_remote::Connection;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::watch;
use tokio::task::{JoinHandle, JoinSet};

use crate::{Event, TransferEngine};

/// How long a `SitePaused` on a still-`Connected` connection waits before the engine is told
/// the network is restored.
pub const RESTORE_GRACE: Duration = Duration::from_secs(2);

/// Watches `conn` and tells `engine` when the network of `site` is back. The app wires one
/// watcher per connected site (any protocol):
///
/// - the state returns to `Connected` with a generation newer than the last one seen (a
///   reconnection): call `network_restored` at once;
/// - a `SitePaused` for `site` arrives while the state is `Connected` (a request failed,
///   e.g. timed out, but the connection never dropped, so no reconnection will follow):
///   after [`RESTORE_GRACE`], if the state is still `Connected`, call `network_restored`;
/// - the event receiver lagged (a `SitePaused` may have been lost): if the state is
///   `Connected`, call `network_restored` (it does nothing when the site is not paused).
///
/// The task ends when the connection's state channel closes or the engine's event channel
/// closes. Abort the returned handle to stop it earlier.
pub fn spawn_site_watcher(
    engine: TransferEngine,
    site: SiteId,
    conn: Arc<dyn Connection>,
) -> JoinHandle<()> {
    // Subscribe before looking at anything, so no event between here and the first poll is lost.
    let mut events = engine.subscribe();
    let mut state = conn.state();
    let mut generation = conn.generation();
    tokio::spawn(async move {
        // Grace timers; dropped (aborted) together with the watcher.
        let mut graces = JoinSet::new();
        loop {
            tokio::select! {
                changed = state.changed() => {
                    if changed.is_err() {
                        break;
                    }
                    let connected = *state.borrow_and_update() == ConnectionState::Connected;
                    let current = conn.generation();
                    if connected && current > generation {
                        generation = current;
                        engine.network_restored(site.clone()).await;
                    }
                }
                event = events.recv() => match event {
                    Ok(Event::SitePaused { site: paused, .. }) if paused == site => {
                        if is_connected(&state) {
                            let (engine, site, state) = (engine.clone(), site.clone(), state.clone());
                            graces.spawn(async move {
                                tokio::time::sleep(RESTORE_GRACE).await;
                                if is_connected(&state) {
                                    engine.network_restored(site).await;
                                }
                            });
                        }
                    }
                    Err(RecvError::Lagged(_)) => {
                        if is_connected(&state) {
                            engine.network_restored(site.clone()).await;
                        }
                    }
                    Ok(_) => {}
                    Err(RecvError::Closed) => break,
                },
                // Reap finished grace timers.
                Some(_) = graces.join_next(), if !graces.is_empty() => {}
            }
        }
    })
}

fn is_connected(state: &watch::Receiver<ConnectionState>) -> bool {
    *state.borrow() == ConnectionState::Connected
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::SystemTime;

    use hoprs_core::{ConnectionState, ErrorKind, RemotePath};
    use hoprs_remote::{FaultFs, MemFs};
    use tokio::task::JoinHandle;

    use super::*;
    use crate::{
        Direction, EngineConfig, Event, QueueStore, SiteEndpoints, TransferId, TransferSpec,
        TransferState,
    };

    struct FakeConnection {
        tx: Mutex<Option<watch::Sender<ConnectionState>>>,
        generation: AtomicU64,
    }

    impl FakeConnection {
        fn new() -> Arc<Self> {
            let (tx, _) = watch::channel(ConnectionState::Connected);
            Arc::new(Self {
                tx: Mutex::new(Some(tx)),
                generation: AtomicU64::new(1),
            })
        }

        fn set(&self, state: ConnectionState) {
            if let Some(tx) = self.tx.lock().unwrap().as_ref() {
                tx.send_replace(state);
            }
        }

        /// The connection is gone for good: the state channel closes.
        fn close(&self) {
            self.tx.lock().unwrap().take();
        }

        /// Reconnected: new generation, then `Connected`.
        fn reconnected(&self) {
            self.generation.fetch_add(1, Ordering::SeqCst);
            self.set(ConnectionState::Connected);
        }
    }

    impl Connection for FakeConnection {
        fn state(&self) -> watch::Receiver<ConnectionState> {
            match self.tx.lock().unwrap().as_ref() {
                Some(tx) => tx.subscribe(),
                None => watch::channel(ConnectionState::Connected).1,
            }
        }

        fn generation(&self) -> u64 {
            self.generation.load(Ordering::SeqCst)
        }
    }

    fn site() -> SiteId {
        SiteId("s".to_string())
    }

    struct Rig {
        engine: TransferEngine,
        rx: tokio::sync::broadcast::Receiver<Event>,
        id: TransferId,
        remote: MemFs,
    }

    /// An engine with one upload whose first attempt fails with a network error, watched by a
    /// site watcher over `conn` (started before the upload is queued, so no event is missed).
    async fn rig(conn: &Arc<FakeConnection>) -> (Rig, JoinHandle<()>) {
        let local = MemFs::new();
        let remote_mem = MemFs::new();
        let remote = FaultFs::new(remote_mem.clone());
        local.add_dir("/w");
        remote_mem.add_dir("/srv");
        local.add_file("/w/a", &vec![7u8; 1 << 20], SystemTime::UNIX_EPOCH);
        remote.fail_io_after(200_000, ErrorKind::Network);
        let store = QueueStore::open_in_memory().unwrap();
        let engine = TransferEngine::start(store, EngineConfig::default());
        let rx = engine.subscribe();
        engine
            .register_site(
                site(),
                SiteEndpoints {
                    local: Arc::new(local),
                    remote: Arc::new(remote),
                    workers: 1,
                },
            )
            .await;
        let watcher = spawn_site_watcher(engine.clone(), site(), conn.clone());
        let id = engine
            .enqueue(TransferSpec {
                site: site(),
                direction: Direction::Upload,
                local: RemotePath::new("/w/a").unwrap(),
                remote: RemotePath::new("/srv/a").unwrap(),
            })
            .await
            .unwrap();
        let rig = Rig {
            engine,
            rx,
            id,
            remote: remote_mem,
        };
        (rig, watcher)
    }

    async fn wait_paused(rx: &mut tokio::sync::broadcast::Receiver<Event>) {
        loop {
            if let Ok(Event::SitePaused { .. }) = rx.recv().await {
                return;
            }
        }
    }

    /// Polls the queue (progress events can overflow the broadcast buffer, so events may be lost).
    async fn wait_done(rig: &mut Rig) {
        loop {
            match state_of(rig).await {
                TransferState::Done => return,
                s => assert!(!s.is_terminal(), "unexpected terminal state {s:?}"),
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    async fn state_of(rig: &Rig) -> TransferState {
        rig.engine
            .snapshot()
            .await
            .into_iter()
            .find(|i| i.id == rig.id)
            .unwrap()
            .state
    }

    async fn settle() {
        for _ in 0..50 {
            tokio::task::yield_now().await;
        }
    }

    #[tokio::test(start_paused = true)]
    async fn restores_on_new_generation() {
        let conn = FakeConnection::new();
        let (mut rig, _watcher) = rig(&conn).await;
        wait_paused(&mut rig.rx).await;
        conn.set(ConnectionState::Reconnecting { attempt: 1 });
        conn.reconnected();
        tokio::time::timeout(Duration::from_secs(1), wait_done(&mut rig))
            .await
            .expect("item finishes right after the reconnection, before the grace");
        assert_eq!(
            rig.remote.read_file("/srv/a").map(|d| d.len()),
            Some(1 << 20)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn restores_after_grace_when_still_connected() {
        let conn = FakeConnection::new();
        let (mut rig, _watcher) = rig(&conn).await;
        wait_paused(&mut rig.rx).await;
        settle().await;
        tokio::time::advance(RESTORE_GRACE - Duration::from_millis(100)).await;
        settle().await;
        assert_ne!(state_of(&rig).await, TransferState::Done);
        assert!(
            rig.remote
                .read_file("/srv/a")
                .is_none_or(|d| d.len() < 1 << 20)
        );
        tokio::time::advance(Duration::from_millis(100)).await;
        wait_done(&mut rig).await;
    }

    #[tokio::test(start_paused = true)]
    async fn no_restore_while_reconnecting() {
        let conn = FakeConnection::new();
        let (mut rig, _watcher) = rig(&conn).await;
        wait_paused(&mut rig.rx).await;
        conn.set(ConnectionState::Reconnecting { attempt: 1 });
        tokio::time::advance(RESTORE_GRACE * 5).await;
        settle().await;
        assert_ne!(state_of(&rig).await, TransferState::Done);
        conn.reconnected();
        wait_done(&mut rig).await;
    }

    #[tokio::test(start_paused = true)]
    async fn ends_when_connection_closes() {
        let conn = FakeConnection::new();
        let (_rig, watcher) = rig(&conn).await;
        conn.close();
        tokio::time::timeout(Duration::from_secs(1), watcher)
            .await
            .expect("watcher ends")
            .unwrap();
    }
}
