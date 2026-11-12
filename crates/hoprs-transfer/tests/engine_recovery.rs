mod common;

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use common::*;
use hoprs_core::{ErrorKind, SiteId};
use hoprs_remote::{FaultFs, MemFs};
use hoprs_transfer::{
    Direction, EngineConfig, Event, QueueStore, SiteEndpoints, TransferEngine, TransferId,
    TransferItem, TransferSpec, TransferState,
};
use tokio::sync::broadcast::Receiver;
use tokio::sync::broadcast::error::RecvError;

const MIB: usize = 1 << 20;

fn site_named(name: &str) -> SiteId {
    SiteId(name.to_string())
}

fn upload(site: &SiteId, local: &str, remote: &str) -> TransferSpec {
    TransferSpec {
        site: site.clone(),
        direction: Direction::Upload,
        local: path(local),
        remote: path(remote),
    }
}

fn endpoints(local: Arc<MemFs>, remote: Arc<FaultFs<MemFs>>, workers: usize) -> SiteEndpoints {
    SiteEndpoints {
        local,
        remote,
        workers,
    }
}

fn find(snap: &[TransferItem], id: TransferId) -> &TransferItem {
    snap.iter().find(|i| i.id == id).expect("item in snapshot")
}

/// Waits until every id in `ids` reached a terminal state; returns their states.
async fn wait_all_terminal(
    rx: &mut Receiver<Event>,
    ids: &[TransferId],
) -> HashMap<TransferId, TransferState> {
    let mut done = HashMap::new();
    while done.len() < ids.len() {
        if let Event::StateChanged { id, state } = wait_for(rx, |e| {
            matches!(e, Event::StateChanged { id, state } if ids.contains(id) && state.is_terminal())
        })
        .await
        {
            done.insert(id, state);
        }
    }
    done
}

/// Collects every event received during `window` (the one timing-based
/// negative check: nothing must happen).
async fn events_during(rx: &mut Receiver<Event>, window: Duration) -> Vec<Event> {
    let mut out = Vec::new();
    let _ = tokio::time::timeout(window, async {
        loop {
            match rx.recv().await {
                Ok(ev) => out.push(ev),
                Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => break,
            }
        }
    })
    .await;
    out
}

fn is_failed(e: &Event) -> bool {
    matches!(
        e,
        Event::StateChanged {
            state: TransferState::Failed(_),
            ..
        }
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn network_error_pauses_site() {
    let local = MemFs::new();
    let remote_mem = MemFs::new();
    let remote = FaultFs::new(remote_mem.clone());
    let (engine, mut rx) = engine_with(
        Arc::new(local.clone()),
        Arc::new(remote.clone()),
        2,
        EngineConfig::default(),
    )
    .await;
    remote_mem.add_dir("/srv");
    let mut files = Vec::new();
    for (i, name) in ["a", "b", "c"].into_iter().enumerate() {
        let bytes = data(MIB + i * 1000);
        local.add_file(&format!("/w/{name}"), &bytes, mtime());
        files.push((name, bytes));
    }
    remote.fail_io_after(200_000, ErrorKind::Network);

    let mut ids = Vec::new();
    for (name, _) in &files {
        let id = engine
            .enqueue(spec(
                Direction::Upload,
                &format!("/w/{name}"),
                &format!("/srv/{name}"),
            ))
            .await
            .unwrap();
        ids.push(id);
    }

    let mut before = Vec::new();
    let paused = wait_for(&mut rx, |e| {
        before.push(e.clone());
        matches!(e, Event::SitePaused { .. })
    })
    .await;
    match paused {
        Event::SitePaused { site: s, error } => {
            assert_eq!(s, site());
            assert_eq!(error.kind, ErrorKind::Network);
        }
        _ => unreachable!(),
    }
    let after = events_during(&mut rx, Duration::from_millis(300)).await;
    assert!(
        !after.iter().any(|e| matches!(
            e,
            Event::StateChanged {
                state: TransferState::Active,
                ..
            }
        )),
        "no item is dispatched while the site is paused: {after:?}"
    );
    assert!(!before.iter().chain(&after).any(is_failed));
    assert!(
        !after.iter().any(|e| matches!(e, Event::SitePaused { .. })),
        "exactly one SitePaused"
    );
    assert!(
        engine
            .snapshot()
            .await
            .iter()
            .all(|i| !matches!(i.state, TransferState::Failed(_)))
    );

    // An item already running when the site paused may have finished meanwhile.
    let mut done: HashMap<TransferId, TransferState> = before
        .iter()
        .chain(&after)
        .filter_map(|e| match e {
            Event::StateChanged { id, state } if state.is_terminal() => Some((*id, state.clone())),
            _ => None,
        })
        .collect();
    let pending: Vec<TransferId> = ids
        .iter()
        .copied()
        .filter(|id| !done.contains_key(id))
        .collect();
    assert!(!pending.is_empty());

    engine.resume_site(site()).await;
    done.extend(wait_all_terminal(&mut rx, &pending).await);
    for id in &ids {
        assert_eq!(done.get(id), Some(&TransferState::Done));
    }
    for (name, bytes) in &files {
        assert_eq!(
            remote_mem.read_file(&format!("/srv/{name}")).as_ref(),
            Some(bytes)
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn other_sites_keep_running() {
    let (a, b) = (site_named("a"), site_named("b"));
    let (engine, mut rx) = start_engine(EngineConfig::default());
    let local = MemFs::new();
    let remote_a = MemFs::new();
    let remote_b = MemFs::new();
    let fault_a = FaultFs::new(remote_a.clone());
    let fault_b = FaultFs::new(remote_b.clone());
    remote_a.add_dir("/srv");
    remote_b.add_dir("/srv");
    for name in ["x", "y", "z"] {
        local.add_file(&format!("/w/{name}"), &data(MIB), mtime());
    }
    engine
        .register_site(
            a.clone(),
            endpoints(Arc::new(local.clone()), Arc::new(fault_a.clone()), 1),
        )
        .await;
    engine
        .register_site(
            b.clone(),
            endpoints(Arc::new(local.clone()), Arc::new(fault_b), 2),
        )
        .await;
    fault_a.fail_io_after(200_000, ErrorKind::Network);

    let mut ids_a = Vec::new();
    for name in ["x", "y", "z"] {
        let spec = upload(&a, &format!("/w/{name}"), &format!("/srv/{name}"));
        ids_a.push(engine.enqueue(spec).await.unwrap());
    }
    wait_for(
        &mut rx,
        |e| matches!(e, Event::SitePaused { site, .. } if *site == a),
    )
    .await;

    let mut ids_b = Vec::new();
    for name in ["x", "y", "z"] {
        let spec = upload(&b, &format!("/w/{name}"), &format!("/srv/{name}"));
        ids_b.push(engine.enqueue(spec).await.unwrap());
    }
    let done = wait_all_terminal(&mut rx, &ids_b).await;
    for id in &ids_b {
        assert_eq!(done.get(id), Some(&TransferState::Done));
    }
    for name in ["x", "y", "z"] {
        assert_eq!(
            remote_b.read_file(&format!("/srv/{name}")),
            local.read_file(&format!("/w/{name}"))
        );
    }

    let snap = engine.snapshot().await;
    for id in &ids_a {
        assert_eq!(
            find(&snap, *id).state,
            TransferState::Queued,
            "site a stays paused"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn manual_pause_site_holds_dispatch_until_resume() {
    let (engine, local, remote, mut rx) = engine_with_memfs(2).await;
    remote.add_dir("/srv");
    local.add_file("/w/a", &data(10_000), mtime());

    engine.pause_site(site()).await;
    let id = engine
        .enqueue(spec(Direction::Upload, "/w/a", "/srv/a"))
        .await
        .unwrap();
    let during = events_during(&mut rx, Duration::from_millis(300)).await;
    assert!(
        !during
            .iter()
            .any(|e| matches!(e, Event::StateChanged { .. } | Event::SitePaused { .. })),
        "a manually paused site dispatches nothing and emits no SitePaused: {during:?}"
    );

    // Unknown or unpaused sites are no-ops.
    engine.pause_site(site_named("nope")).await;
    engine.resume_site(site_named("nope")).await;

    engine.resume_site(site()).await;
    assert_eq!(wait_terminal(&mut rx, id).await, TransferState::Done);
    engine.resume_site(site()).await;
}

/// Starts an engine over the queue database at `db`, registered as site `"s"`.
async fn engine_on_file(
    db: &Path,
    local: Arc<FaultFs<MemFs>>,
    remote: MemFs,
    workers: usize,
    config: EngineConfig,
) -> (TransferEngine, Receiver<Event>) {
    let store = QueueStore::open(db).unwrap();
    let engine = TransferEngine::start(store, config);
    let rx = engine.subscribe();
    engine
        .register_site(
            site(),
            SiteEndpoints {
                local,
                remote: Arc::new(remote),
                workers,
            },
        )
        .await;
    (engine, rx)
}

/// Waits for a `Progress` update reporting bytes copied for `id`.
async fn wait_bytes(rx: &mut Receiver<Event>, id: TransferId) {
    wait_for(
        rx,
        |e| matches!(e, Event::Progress(us) if us.iter().any(|u| u.id == id && u.bytes_done > 0)),
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn restart_recovers_as_paused() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("queue.db");
    let local = MemFs::new();
    let remote = MemFs::new();
    let bytes = data(4 * MIB);
    local.add_file("/w/slow.bin", &bytes, mtime());
    local.add_file("/w/ask.bin", &data(3_000), mtime());
    remote.add_file("/srv/ask.bin", b"old contents", mtime());

    let slow = FaultFs::new(local.clone());
    slow.set_stream_latency(Duration::from_millis(20));
    let (engine, mut rx) = engine_on_file(
        &db,
        Arc::new(slow),
        remote.clone(),
        2,
        EngineConfig::default(),
    )
    .await;
    let active = engine
        .enqueue(spec(Direction::Upload, "/w/slow.bin", "/srv/slow.bin"))
        .await
        .unwrap();
    let asking = engine
        .enqueue(spec(Direction::Upload, "/w/ask.bin", "/srv/ask.bin"))
        .await
        .unwrap();
    wait_for(&mut rx, |e| {
        matches!(e, Event::StateChanged { id, state: TransferState::AwaitingDecision(_) } if *id == asking)
    })
    .await;
    wait_bytes(&mut rx, active).await;
    engine.shutdown().await;

    let fault = FaultFs::new(local.clone());
    let (engine, mut rx) = engine_on_file(
        &db,
        Arc::new(fault.clone()),
        remote.clone(),
        2,
        EngineConfig::default(),
    )
    .await;
    let snap = engine.snapshot().await;
    assert_eq!(snap.len(), 2);
    assert_eq!(find(&snap, active).state, TransferState::Paused);
    assert_eq!(find(&snap, asking).state, TransferState::Paused);

    engine.resume(active).await.unwrap();
    assert_eq!(wait_terminal(&mut rx, active).await, TransferState::Done);
    assert_eq!(remote.read_file("/srv/slow.bin").unwrap(), bytes);
    let offsets = fault.read_offsets();
    assert!(
        offsets.len() == 1 && offsets[0] > 0,
        "resumed from the partial destination: {offsets:?}"
    );

    engine.resume(asking).await.unwrap();
    wait_for(&mut rx, |e| {
        matches!(e, Event::StateChanged { id, state: TransferState::AwaitingDecision(_) } if *id == asking)
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn restart_keeps_expanded_folder_and_pauses_queued_items() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("queue.db");
    let local = MemFs::new();
    let remote = MemFs::new();
    remote.add_dir("/srv");
    for i in 0..6 {
        local.add_file(&format!("/w/d/f{i}.bin"), &data(MIB), mtime());
    }
    let config = EngineConfig {
        buf_size: 64 * 1024,
        ..EngineConfig::default()
    };

    let slow = FaultFs::new(local.clone());
    slow.set_stream_latency(Duration::from_millis(5));
    let (engine, mut rx) =
        engine_on_file(&db, Arc::new(slow), remote.clone(), 1, config.clone()).await;
    let root = engine
        .enqueue(spec(Direction::Upload, "/w/d", "/srv/d"))
        .await
        .unwrap();
    // The first child finishes while the others wait in the queue.
    wait_for(
        &mut rx,
        |e| matches!(e, Event::StateChanged { id, state: TransferState::Done } if *id != root),
    )
    .await;
    engine.shutdown().await;

    let (engine, mut rx) = engine_on_file(
        &db,
        Arc::new(FaultFs::new(local.clone())),
        remote.clone(),
        2,
        config,
    )
    .await;
    let snap = engine.snapshot().await;
    assert_eq!(snap.len(), 7, "the folder and its six children");
    assert!(
        snap.iter()
            .all(|i| i.state == TransferState::Paused || i.state == TransferState::Done),
        "{snap:?}"
    );
    assert!(snap.iter().any(|i| i.state == TransferState::Done));

    engine.resume(root).await.unwrap();
    assert_eq!(wait_terminal(&mut rx, root).await, TransferState::Done);
    let snap = engine.snapshot().await;
    assert_eq!(snap.len(), 7, "the folder is not listed again");
    assert!(snap.iter().all(|i| i.state == TransferState::Done));
    for i in 0..6 {
        assert_eq!(
            remote.read_file(&format!("/srv/d/f{i}.bin")),
            local.read_file(&format!("/w/d/f{i}.bin"))
        );
    }
}
