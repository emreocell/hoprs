//! Site watcher: tells the engine when the network of a site is back.

use std::sync::Arc;
use std::time::Duration;

use hoprs_core::{ConnectionState, SiteId};
use hoprs_remote::Connection;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::watch;
use tokio::task::{JoinHandle, JoinSet};

use crate::{Event, EventKind, TransferEngine};

/// How long a `SitePaused` on a still-`Connected` connection waits before the engine is told
/// the network is restored.
pub const RESTORE_GRACE: Duration = Duration::from_secs(2);

/// Grace restores a watcher issues before it gives up on a site whose connection stays up.
pub const MAX_GRACE_RESTORES: u32 = 3;

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
/// Grace restores are limited to [`MAX_GRACE_RESTORES`] since the last generation change: a
/// request that keeps failing with `Network` while the connection stays up (a data channel
/// the server never opens, say) would otherwise pause and restore the site forever. Past the
/// limit the site stays network-paused (its items stay queued, none fails) until the user
/// calls `resume_site` or the connection comes back with a new generation; either starts the
/// count over. The engine emits `SitePaused` only when the site's network pause turns on, so
/// a `SitePaused` that arrives after the watcher gave up proves that someone else (the user)
/// resumed the site; that is how a user resume is seen. While given up, a lagged receiver
/// does not restore either.
///
/// The task ends when the connection's state channel closes or the engine stops. Abort the returned handle to stop it earlier.
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
        // Grace restores since the last generation change (or user resume).
        let mut restores = 0u32;
        // The limit was reached: the site was left network-paused.
        let mut gave_up = false;
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
                        (restores, gave_up) = (0, false);
                        engine.network_restored(site.clone()).await;
                    }
                }
                event = events.recv() => match event {
                    Ok(Event {
                        kind: EventKind::SitePaused { site: paused, .. },
                        ..
                    }) if paused == site => {
                        if gave_up {
                            // Only a resume from elsewhere (the user) lets the site pause again.
                            (restores, gave_up) = (0, false);
                        }
                        if restores >= MAX_GRACE_RESTORES {
                            gave_up = true;
                        } else if is_connected(&state) {
                            restores += 1;
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
                        if !gave_up && is_connected(&state) {
                            engine.network_restored(site.clone()).await;
                        }
                    }
                    Ok(_) => {}
                    Err(RecvError::Closed) => break,
                },
                _ = engine.stopped() => break,
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
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::time::SystemTime;

    use hoprs_core::{ConnectionState, ErrorKind, RemotePath};
    use hoprs_remote::{FaultFs, MemFs, Op};
    use tokio::task::JoinHandle;

    use super::*;
    use crate::{
        Direction, EngineConfig, Event, QueueStore, SiteEndpoints, TransferId, TransferSpec,
        TransferState,
    };

    struct FakeConnection {
        tx: Mutex<Option<watch::Sender<ConnectionState>>>,
        generation: AtomicU64,
        /// While set, `generation()` blocks the calling thread (stalls the watcher).
        block: AtomicBool,
    }

    impl FakeConnection {
        fn new() -> Arc<Self> {
            let (tx, _) = watch::channel(ConnectionState::Connected);
            Arc::new(Self {
                tx: Mutex::new(Some(tx)),
                generation: AtomicU64::new(1),
                block: AtomicBool::new(false),
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
            while self.block.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(1));
            }
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

    fn upload_spec(site: &SiteId, name: &str) -> TransferSpec {
        TransferSpec {
            site: site.clone(),
            direction: Direction::Upload,
            local: RemotePath::new(&format!("/w/{name}")).unwrap(),
            remote: RemotePath::new(&format!("/srv/{name}")).unwrap(),
        }
    }

    /// Registers `site` on `engine`: a local `MemFs` holding `files` (name, size) under `/w`,
    /// and a remote `MemFs` with `/srv`. Returns the remote and its fault injector.
    async fn add_site(
        engine: &TransferEngine,
        site: &SiteId,
        files: &[(String, usize)],
    ) -> (MemFs, FaultFs<MemFs>) {
        let local = MemFs::new();
        let remote_mem = MemFs::new();
        let remote = FaultFs::new(remote_mem.clone());
        local.add_dir("/w");
        remote_mem.add_dir("/srv");
        for (name, size) in files {
            local.add_file(
                &format!("/w/{name}"),
                &vec![7u8; *size],
                SystemTime::UNIX_EPOCH,
            );
        }
        engine
            .register_site(
                site.clone(),
                SiteEndpoints {
                    local: Arc::new(local),
                    remote: Arc::new(remote.clone()),
                    workers: 1,
                },
            )
            .await;
        (remote_mem, remote)
    }

    fn engine() -> TransferEngine {
        TransferEngine::start(
            QueueStore::open_in_memory().unwrap(),
            EngineConfig::default(),
        )
        .unwrap()
    }

    /// An engine with one upload whose first attempt fails with a network error, watched by a
    /// site watcher over `conn` (started before the upload is queued, so no event is missed).
    async fn rig(conn: &Arc<FakeConnection>) -> (Rig, JoinHandle<()>) {
        let engine = engine();
        let rx = engine.subscribe();
        let (remote_mem, remote) = add_site(&engine, &site(), &[("a".into(), 1 << 20)]).await;
        remote.fail_io_after(200_000, ErrorKind::Network);
        let watcher = spawn_site_watcher(engine.clone(), site(), conn.clone());
        let id = engine.enqueue(upload_spec(&site(), "a")).await.unwrap();
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
            if let Ok(Event {
                kind: EventKind::SitePaused { .. },
                ..
            }) = rx.recv().await
            {
                return;
            }
        }
    }

    /// Polls the queue (progress events can overflow the broadcast buffer, so events may be lost).
    async fn wait_done(rig: &mut Rig) {
        wait_item_done(&rig.engine, rig.id).await;
    }

    async fn wait_item_done(engine: &TransferEngine, id: TransferId) {
        loop {
            match state_in(engine, id).await {
                TransferState::Done => return,
                s => assert!(!s.is_terminal(), "unexpected terminal state {s:?}"),
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    async fn state_in(engine: &TransferEngine, id: TransferId) -> TransferState {
        engine
            .snapshot()
            .await
            .into_iter()
            .find(|i| i.id == id)
            .unwrap()
            .state
    }

    async fn state_of(rig: &Rig) -> TransferState {
        state_in(&rig.engine, rig.id).await
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
        settle().await;
        // The grace timer is running (half elapsed) when the connection drops.
        tokio::time::advance(RESTORE_GRACE / 2).await;
        settle().await;
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

    #[tokio::test(start_paused = true)]
    async fn ends_when_engine_shuts_down() {
        let conn = FakeConnection::new();
        let (rig, watcher) = rig(&conn).await;
        rig.engine.clone().shutdown().await;
        tokio::time::timeout(Duration::from_secs(1), watcher)
            .await
            .expect("watcher ends with the engine")
            .unwrap();
        drop(conn);
    }

    #[tokio::test(start_paused = true)]
    async fn ignores_pause_of_another_site() {
        let conn = FakeConnection::new();
        let (rig, _watcher) = rig(&conn).await;
        let other = SiteId("other".to_string());
        let (_remote, fault) = add_site(&rig.engine, &other, &[("b".into(), 1 << 20)]).await;
        fault.fail_io_after(200_000, ErrorKind::Network);
        let mut rx = rig.engine.subscribe();
        let id = rig.engine.enqueue(upload_spec(&other, "b")).await.unwrap();
        loop {
            if let Ok(Event {
                kind: EventKind::SitePaused { site, .. },
                ..
            }) = rx.recv().await
                && site == other
            {
                break;
            }
        }
        settle().await;
        tokio::time::advance(RESTORE_GRACE * 5).await;
        settle().await;
        assert_ne!(
            state_in(&rig.engine, id).await,
            TransferState::Done,
            "the other site is not restored by this watcher"
        );
    }

    /// Counts the `SitePaused` of `site()` until `rx` stays quiet for `RESTORE_GRACE * 10`
    /// (paused time: the wait costs nothing). Gives up at 50.
    async fn count_pauses(rx: &mut tokio::sync::broadcast::Receiver<Event>) -> u32 {
        let mut pauses = 0;
        while pauses < 50 {
            match tokio::time::timeout(RESTORE_GRACE * 10, rx.recv()).await {
                Err(_) => return pauses,
                Ok(Ok(Event {
                    kind: EventKind::SitePaused { site: s, .. },
                    ..
                })) if s == site() => pauses += 1,
                Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) => return pauses,
                Ok(_) => {}
            }
        }
        pauses
    }

    /// A request that fails with `Network` every time while the connection stays up: the
    /// watcher restores the site `MAX_GRACE_RESTORES` times, then leaves it paused (the item
    /// stays queued, not failed). A user resume or a new generation allows restores again.
    #[tokio::test(start_paused = true)]
    async fn stops_restoring_after_repeated_failures() {
        let conn = FakeConnection::new();
        let engine = engine();
        let mut rx = engine.subscribe();
        let (_remote, fault) = add_site(&engine, &site(), &[("a".into(), 10)]).await;
        fault.fail_always(Op::OpenWrite, ErrorKind::Network);
        let _watcher = spawn_site_watcher(engine.clone(), site(), conn.clone());
        let id = engine.enqueue(upload_spec(&site(), "a")).await.unwrap();

        // The first failure, then one per restore.
        assert_eq!(count_pauses(&mut rx).await, 1 + MAX_GRACE_RESTORES);
        assert_eq!(state_in(&engine, id).await, TransferState::Queued);

        // The user resumes the site: the count starts over.
        engine.resume_site(site()).await;
        assert_eq!(count_pauses(&mut rx).await, 1 + MAX_GRACE_RESTORES);

        // A reconnection restores at once, and the count starts over too.
        conn.set(ConnectionState::Reconnecting { attempt: 1 });
        conn.reconnected();
        assert_eq!(count_pauses(&mut rx).await, 1 + MAX_GRACE_RESTORES);
        assert_eq!(state_in(&engine, id).await, TransferState::Queued);
    }

    /// Overflows the watcher's event receiver while it is stuck, so that a `SitePaused` is lost.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn restores_after_lagged_events_when_connected() {
        let conn = FakeConnection::new();
        let (mut rig, _watcher) = rig(&conn).await;
        // The watcher sees a state change and blocks inside `generation()`, so it stops
        // receiving events.
        conn.block.store(true, Ordering::SeqCst);
        conn.set(ConnectionState::Reconnecting { attempt: 1 });
        tokio::time::sleep(Duration::from_millis(100)).await;
        // The item fails (SitePaused is emitted, then lost), and another site floods events.
        wait_paused(&mut rig.rx).await;
        let other = SiteId("other".to_string());
        let files: Vec<(String, usize)> = (0..400).map(|i| (format!("f{i}"), 10)).collect();
        add_site(&rig.engine, &other, &files).await;
        for (name, _) in &files {
            rig.engine.enqueue(upload_spec(&other, name)).await.unwrap();
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
        // Connected again without a new generation: only the lag rule can restore.
        conn.set(ConnectionState::Connected);
        conn.block.store(false, Ordering::SeqCst);
        tokio::time::timeout(Duration::from_secs(1), wait_done(&mut rig))
            .await
            .expect("lag while connected restores the site");
    }
}
