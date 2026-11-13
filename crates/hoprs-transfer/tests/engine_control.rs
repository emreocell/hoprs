mod common;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use common::*;
use hoprs_core::ErrorKind;
use hoprs_remote::{FaultFs, MemFs, Op, RemoteFs};
use hoprs_transfer::{
    Direction, EngineConfig, Event, EventKind, Resolution, ResolveAction, TransferEngine,
    TransferId, TransferItem, TransferState,
};
use tokio::sync::broadcast::Receiver;

const MIB: usize = 1 << 20;

/// Waits until `id` reaches exactly `state`.
async fn wait_state(rx: &mut Receiver<Event>, id: TransferId, state: TransferState) {
    wait_for(
        rx,
        |e| matches!(e, EventKind::StateChanged { id: i, state: s } if *i == id && *s == state),
    )
    .await;
}

/// Waits until `id` is `AwaitingDecision`.
async fn wait_awaiting(rx: &mut Receiver<Event>, id: TransferId) {
    wait_for(rx, |e| {
        matches!(e, EventKind::StateChanged { id: i, state: TransferState::AwaitingDecision(_) } if *i == id)
    })
    .await;
}

/// Waits for the first `Progress` event.
async fn wait_progress(rx: &mut Receiver<Event>) {
    wait_for(rx, |e| matches!(e, EventKind::Progress(_))).await;
}

/// Waits until `n` distinct items reached a terminal state; returns their states.
async fn wait_terminal_count(
    rx: &mut Receiver<Event>,
    n: usize,
) -> HashMap<TransferId, TransferState> {
    let mut done = HashMap::new();
    while done.len() < n {
        if let EventKind::StateChanged { id, state } = wait_for(
            rx,
            |e| matches!(e, EventKind::StateChanged { state, .. } if state.is_terminal()),
        )
        .await
        {
            done.insert(id, state);
        }
    }
    done
}

fn find(snap: &[TransferItem], id: TransferId) -> &TransferItem {
    snap.iter().find(|i| i.id == id).expect("item in snapshot")
}

fn resolution(action: ResolveAction, apply_to_all: bool) -> Resolution {
    Resolution {
        action,
        apply_to_all,
    }
}

struct Slow {
    engine: TransferEngine,
    /// The local side, as seen by the engine.
    fault: FaultFs<MemFs>,
    /// The same local filesystem, without faults.
    local: MemFs,
    remote: MemFs,
    rx: Receiver<Event>,
}

/// Engine whose local side is a `FaultFs` with `stream_latency` per chunk.
async fn slow_upload_engine(
    workers: usize,
    stream_latency: Duration,
    config: EngineConfig,
) -> Slow {
    let local = MemFs::new();
    let fault = FaultFs::new(local.clone());
    fault.set_stream_latency(stream_latency);
    let remote = MemFs::new();
    let (engine, rx) = engine_with(
        Arc::new(fault.clone()),
        Arc::new(remote.clone()),
        workers,
        config,
    )
    .await;
    Slow {
        engine,
        fault,
        local,
        remote,
        rx,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn pause_and_resume_continues_from_offset() {
    let Slow {
        engine,
        fault,
        local,
        remote,
        mut rx,
    } = slow_upload_engine(1, Duration::from_millis(20), EngineConfig::default()).await;
    let bytes = data(4 * MIB);
    local.add_file("/w/a.bin", &bytes, mtime());
    remote.add_dir("/srv");

    let id = engine
        .enqueue(spec(Direction::Upload, "/w/a.bin", "/srv/a.bin"))
        .await
        .unwrap();
    wait_progress(&mut rx).await;
    engine.pause(id).await.unwrap();
    wait_state(&mut rx, id, TransferState::Paused).await;

    let paused = find(&engine.snapshot().await, id).clone();
    assert!(
        paused.bytes_done > 0 && paused.bytes_done < bytes.len() as u64,
        "paused at {}",
        paused.bytes_done
    );
    assert_eq!(
        remote.read_file("/srv/a.bin").map(|f| f.len() as u64),
        Some(paused.bytes_done),
        "the partial destination is kept"
    );

    engine.resume(id).await.unwrap();
    assert_eq!(wait_terminal(&mut rx, id).await, TransferState::Done);
    assert_eq!(remote.read_file("/srv/a.bin").unwrap(), bytes);
    let offsets = fault.read_offsets();
    assert!(offsets.iter().any(|&o| o > 0), "offsets {offsets:?}");
    assert_eq!(offsets, vec![0, paused.bytes_done]);
}

#[tokio::test(flavor = "multi_thread")]
async fn cancel_cascades_in_folder() {
    let config = EngineConfig {
        buf_size: 64 * 1024,
        ..EngineConfig::default()
    };
    let Slow {
        engine,
        local,
        remote,
        mut rx,
        ..
    } = slow_upload_engine(4, Duration::from_millis(30), config).await;
    for i in 0..20 {
        local.add_file(&format!("/w/d/f{i:02}.bin"), &data(MIB), mtime());
    }
    remote.add_dir("/srv");

    let root = engine
        .enqueue(spec(Direction::Upload, "/w/d", "/srv/d"))
        .await
        .unwrap();
    wait_progress(&mut rx).await;
    engine.cancel(root).await.unwrap();
    let done = wait_terminal_count(&mut rx, 21).await;
    assert_eq!(done.get(&root), Some(&TransferState::Cancelled));

    let snap = engine.snapshot().await;
    assert_eq!(snap.len(), 21);
    for item in &snap {
        assert_eq!(item.state, TransferState::Cancelled, "{}", item.spec.local);
    }
    // Partial destinations of the files that were copying are not deleted.
    let partial = remote.list(&path("/srv/d")).await.unwrap();
    assert!(partial.iter().any(|e| e.size > 0));
    assert!(partial.iter().all(|e| e.size < MIB as u64));
}

#[tokio::test(flavor = "multi_thread")]
async fn retry_failed() {
    let local = MemFs::new();
    let remote_mem = MemFs::new();
    let remote = FaultFs::new(remote_mem.clone());
    let (engine, mut rx) = engine_with(
        Arc::new(local.clone()),
        Arc::new(remote.clone()),
        1,
        EngineConfig::default(),
    )
    .await;
    let bytes = data(10_000);
    local.add_file("/w/a.bin", &bytes, mtime());
    remote_mem.add_dir("/srv");
    remote.fail_next(Op::OpenWrite, ErrorKind::Permission);

    let id = engine
        .enqueue(spec(Direction::Upload, "/w/a.bin", "/srv/a.bin"))
        .await
        .unwrap();
    match wait_terminal(&mut rx, id).await {
        TransferState::Failed(e) => assert_eq!(e.kind, ErrorKind::Permission),
        other => panic!("expected Failed, got {other:?}"),
    }

    engine.retry(id).await.unwrap();
    assert_eq!(wait_terminal(&mut rx, id).await, TransferState::Done);
    assert_eq!(remote_mem.read_file("/srv/a.bin").unwrap(), bytes);
}

#[tokio::test(flavor = "multi_thread")]
async fn ask_then_resolve_overwrite() {
    let (engine, local, remote, mut rx) = engine_with_memfs(1).await;
    let bytes = data(3_000);
    local.add_file("/w/a.bin", &bytes, mtime());
    remote.add_file("/srv/a.bin", b"old contents", mtime());

    let id = engine
        .enqueue(spec(Direction::Upload, "/w/a.bin", "/srv/a.bin"))
        .await
        .unwrap();
    wait_awaiting(&mut rx, id).await;

    engine
        .resolve(id, resolution(ResolveAction::Overwrite, false))
        .await
        .unwrap();
    assert_eq!(wait_terminal(&mut rx, id).await, TransferState::Done);
    assert_eq!(remote.read_file("/srv/a.bin").unwrap(), bytes);
}

#[tokio::test(flavor = "multi_thread")]
async fn apply_to_all_resolves_pending() {
    let (engine, local, remote, mut rx) = engine_with_memfs(2).await;
    for name in ["a", "b", "c", "d"] {
        local.add_file(&format!("/w/{name}"), &data(1_000), mtime());
        remote.add_file(&format!("/srv/{name}"), b"old", mtime());
    }

    let mut ids = Vec::new();
    for name in ["a", "b", "c"] {
        let local = format!("/w/{name}");
        let remote = format!("/srv/{name}");
        ids.push(
            engine
                .enqueue(spec(Direction::Upload, &local, &remote))
                .await
                .unwrap(),
        );
    }
    let mut awaiting = HashSet::new();
    while awaiting.len() < ids.len() {
        if let EventKind::StateChanged { id, .. } = wait_for(&mut rx, |e| {
            matches!(
                e,
                EventKind::StateChanged {
                    state: TransferState::AwaitingDecision(_),
                    ..
                }
            )
        })
        .await
        {
            awaiting.insert(id);
        }
    }

    engine
        .resolve(ids[0], resolution(ResolveAction::Skip, true))
        .await
        .unwrap();
    for &id in &ids {
        assert_eq!(
            find(&engine.snapshot().await, id).state,
            TransferState::Skipped("skipped by user".to_string())
        );
    }

    let d = engine
        .enqueue(spec(Direction::Upload, "/w/d", "/srv/d"))
        .await
        .unwrap();
    let ev = wait_for(&mut rx, |e| {
        matches!(e, EventKind::StateChanged { id, state }
            if *id == d && (state.is_terminal() || matches!(state, TransferState::AwaitingDecision(_))))
    })
    .await;
    let EventKind::StateChanged { state, .. } = ev else {
        unreachable!()
    };
    assert_eq!(state, TransferState::Skipped("exists".to_string()));
    for name in ["a", "b", "c", "d"] {
        assert_eq!(remote.read_file(&format!("/srv/{name}")).unwrap(), b"old");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn cancel_while_awaiting_decision() {
    let (engine, local, remote, mut rx) = engine_with_memfs(1).await;
    local.add_file("/w/a.bin", &data(3_000), mtime());
    remote.add_file("/srv/a.bin", b"old", mtime());

    let id = engine
        .enqueue(spec(Direction::Upload, "/w/a.bin", "/srv/a.bin"))
        .await
        .unwrap();
    wait_awaiting(&mut rx, id).await;
    engine.cancel(id).await.unwrap();
    assert_eq!(wait_terminal(&mut rx, id).await, TransferState::Cancelled);
    assert_eq!(remote.read_file("/srv/a.bin").unwrap(), b"old");
}

#[tokio::test(flavor = "multi_thread")]
async fn unknown_id_errors() {
    let (engine, _local, _remote, _rx) = engine_with_memfs(1).await;
    let err = engine.pause(TransferId(999)).await.unwrap_err();
    assert_eq!(err.kind, ErrorKind::NotFound);
    assert_eq!(err.message, "Unknown transfer");

    let id = TransferId(999);
    assert_eq!(
        engine.resume(id).await.unwrap_err().kind,
        ErrorKind::NotFound
    );
    assert_eq!(
        engine.cancel(id).await.unwrap_err().kind,
        ErrorKind::NotFound
    );
    assert_eq!(
        engine.retry(id).await.unwrap_err().kind,
        ErrorKind::NotFound
    );
    let r = resolution(ResolveAction::Skip, false);
    assert_eq!(
        engine.resolve(id, r).await.unwrap_err().kind,
        ErrorKind::NotFound
    );
}

// ---- Rulings beyond the brief's list ----

/// A conflict paused while awaiting a decision asks again on resume (R5).
#[tokio::test(flavor = "multi_thread")]
async fn resumed_conflict_asks_again() {
    let (engine, local, remote, mut rx) = engine_with_memfs(1).await;
    local.add_file("/w/a.bin", &data(3_000), mtime());
    remote.add_file("/srv/a.bin", b"old", mtime());

    let id = engine
        .enqueue(spec(Direction::Upload, "/w/a.bin", "/srv/a.bin"))
        .await
        .unwrap();
    wait_awaiting(&mut rx, id).await;
    engine.pause(id).await.unwrap();
    wait_state(&mut rx, id, TransferState::Paused).await;
    // Pausing a paused item is a no-op.
    engine.pause(id).await.unwrap();

    engine.resume(id).await.unwrap();
    wait_awaiting(&mut rx, id).await;
    assert_eq!(remote.read_file("/srv/a.bin").unwrap(), b"old");
}

/// Control commands on items in the wrong state fail with `Conflict`.
#[tokio::test(flavor = "multi_thread")]
async fn commands_in_wrong_state_conflict() {
    let (engine, local, remote, mut rx) = engine_with_memfs(1).await;
    local.add_file("/w/a.bin", &data(100), mtime());
    remote.add_dir("/srv");

    let id = engine
        .enqueue(spec(Direction::Upload, "/w/a.bin", "/srv/a.bin"))
        .await
        .unwrap();
    assert_eq!(wait_terminal(&mut rx, id).await, TransferState::Done);

    let kinds = [
        engine.pause(id).await.unwrap_err().kind,
        engine.resume(id).await.unwrap_err().kind,
        engine.cancel(id).await.unwrap_err().kind,
        engine.retry(id).await.unwrap_err().kind,
        engine
            .resolve(id, resolution(ResolveAction::Overwrite, false))
            .await
            .unwrap_err()
            .kind,
    ];
    assert_eq!(kinds, [ErrorKind::Conflict; 5]);
}

/// Pausing a folder pauses its pending children; resuming finishes it
/// without listing it again (no duplicate children).
#[tokio::test(flavor = "multi_thread")]
async fn pause_and_resume_folder_does_not_relist() {
    let config = EngineConfig {
        buf_size: 64 * 1024,
        ..EngineConfig::default()
    };
    let Slow {
        engine,
        local,
        remote,
        mut rx,
        ..
    } = slow_upload_engine(1, Duration::from_millis(20), config).await;
    let files: Vec<(String, Vec<u8>)> = (0..4)
        .map(|i| (format!("f{i}.bin"), data(MIB + i)))
        .collect();
    for (name, bytes) in &files {
        local.add_file(&format!("/w/d/{name}"), bytes, mtime());
    }
    remote.add_dir("/srv");

    let root = engine
        .enqueue(spec(Direction::Upload, "/w/d", "/srv/d"))
        .await
        .unwrap();
    wait_progress(&mut rx).await;
    engine.pause(root).await.unwrap();
    wait_state(&mut rx, root, TransferState::Paused).await;
    // Wait for the in-flight file to stop.
    wait_for(
        &mut rx,
        |e| matches!(e, EventKind::StateChanged { id, state: TransferState::Paused } if *id != root),
    )
    .await;
    let snap = engine.snapshot().await;
    assert_eq!(snap.len(), 5);
    assert!(
        snap.iter().all(|i| i.state == TransferState::Paused),
        "{snap:?}"
    );

    engine.resume(root).await.unwrap();
    assert_eq!(wait_terminal(&mut rx, root).await, TransferState::Done);
    let snap = engine.snapshot().await;
    assert_eq!(snap.len(), 5, "the folder was not listed again");
    assert!(snap.iter().all(|i| i.state == TransferState::Done));
    for (name, bytes) in &files {
        assert_eq!(
            remote.read_file(&format!("/srv/d/{name}")).as_ref(),
            Some(bytes)
        );
    }
}

/// Pausing a folder whose listing is in flight creates its children paused.
#[tokio::test(flavor = "multi_thread")]
async fn pause_folder_while_listing() {
    let local_mem = MemFs::new();
    let local = FaultFs::new(local_mem.clone());
    let remote = MemFs::new();
    let (engine, mut rx) = engine_with(
        Arc::new(local.clone()),
        Arc::new(remote.clone()),
        2,
        EngineConfig::default(),
    )
    .await;
    local_mem.add_file("/w/d/a", b"aaa", mtime());
    local_mem.add_file("/w/d/b", b"bbb", mtime());
    remote.add_dir("/srv");
    local.set_latency(Duration::from_millis(100));

    let root = engine
        .enqueue(spec(Direction::Upload, "/w/d", "/srv/d"))
        .await
        .unwrap();
    wait_state(&mut rx, root, TransferState::Active).await;
    engine.pause(root).await.unwrap();
    wait_state(&mut rx, root, TransferState::Paused).await;
    let snap = engine.snapshot().await;
    assert_eq!(snap.len(), 3);
    assert!(snap.iter().all(|i| i.state == TransferState::Paused));

    local.set_latency(Duration::ZERO);
    engine.resume(root).await.unwrap();
    assert_eq!(wait_terminal(&mut rx, root).await, TransferState::Done);
    assert_eq!(engine.snapshot().await.len(), 3);
    assert_eq!(remote.read_file("/srv/d/a").unwrap(), b"aaa");
    assert_eq!(remote.read_file("/srv/d/b").unwrap(), b"bbb");
}

/// A child cancelled by the user does not make its folder fail (R18).
#[tokio::test(flavor = "multi_thread")]
async fn cancelled_child_does_not_fail_folder() {
    let (engine, local, remote, mut rx) = engine_with_memfs(1).await;
    local.add_file("/w/d/a", &data(100), mtime());
    local.add_file("/w/d/b", &data(100), mtime());
    remote.add_file("/srv/d/a", b"old", mtime());

    let root = engine
        .enqueue(spec(Direction::Upload, "/w/d", "/srv/d"))
        .await
        .unwrap();
    let ev = wait_for(&mut rx, |e| {
        matches!(
            e,
            EventKind::StateChanged {
                state: TransferState::AwaitingDecision(_),
                ..
            }
        )
    })
    .await;
    let EventKind::StateChanged { id: a, .. } = ev else {
        unreachable!()
    };
    engine.cancel(a).await.unwrap();
    assert_eq!(wait_terminal(&mut rx, root).await, TransferState::Done);
    assert_eq!(
        find(&engine.snapshot().await, a).state,
        TransferState::Cancelled
    );
}

/// Retrying a failed folder requeues only its failed children, without
/// listing it again.
#[tokio::test(flavor = "multi_thread")]
async fn retry_failed_folder() {
    let local = MemFs::new();
    let remote_mem = MemFs::new();
    let remote = FaultFs::new(remote_mem.clone());
    let (engine, mut rx) = engine_with(
        Arc::new(local.clone()),
        Arc::new(remote.clone()),
        1,
        EngineConfig::default(),
    )
    .await;
    local.add_file("/w/d/a", &data(100), mtime());
    local.add_file("/w/d/b", &data(200), mtime());
    remote_mem.add_dir("/srv");
    remote.fail_next(Op::OpenWrite, ErrorKind::Permission);

    let root = engine
        .enqueue(spec(Direction::Upload, "/w/d", "/srv/d"))
        .await
        .unwrap();
    match wait_terminal(&mut rx, root).await {
        TransferState::Failed(e) => assert_eq!(e.message, "1 items failed"),
        other => panic!("expected Failed, got {other:?}"),
    }

    engine.retry(root).await.unwrap();
    assert_eq!(wait_terminal(&mut rx, root).await, TransferState::Done);
    let snap = engine.snapshot().await;
    assert_eq!(snap.len(), 3, "the folder was not listed again");
    assert!(snap.iter().all(|i| i.state == TransferState::Done));
    assert_eq!(remote_mem.read_file("/srv/d/a").unwrap(), data(100));
    assert_eq!(remote_mem.read_file("/srv/d/b").unwrap(), data(200));
}

/// Resuming a folder before its copying file has stopped still finishes
/// every file, from where it stopped.
#[tokio::test(flavor = "multi_thread")]
async fn resume_folder_while_child_is_stopping() {
    let config = EngineConfig {
        buf_size: 64 * 1024,
        ..EngineConfig::default()
    };
    let Slow {
        engine,
        local,
        remote,
        mut rx,
        ..
    } = slow_upload_engine(1, Duration::from_millis(20), config).await;
    let files: Vec<(String, Vec<u8>)> = (0..2)
        .map(|i| (format!("f{i}.bin"), data(MIB + i)))
        .collect();
    for (name, bytes) in &files {
        local.add_file(&format!("/w/d/{name}"), bytes, mtime());
    }
    remote.add_dir("/srv");

    let root = engine
        .enqueue(spec(Direction::Upload, "/w/d", "/srv/d"))
        .await
        .unwrap();
    wait_progress(&mut rx).await;
    engine.pause(root).await.unwrap();
    engine.resume(root).await.unwrap();
    assert_eq!(wait_terminal(&mut rx, root).await, TransferState::Done);
    let snap = engine.snapshot().await;
    assert_eq!(snap.len(), 3);
    assert!(snap.iter().all(|i| i.state == TransferState::Done));
    for (name, bytes) in &files {
        assert_eq!(
            remote.read_file(&format!("/srv/d/{name}")).as_ref(),
            Some(bytes)
        );
    }
}

/// Engine with a slow-streaming local `FaultFs` and a remote `FaultFs`.
async fn faulty_engine(
    workers: usize,
    stream_latency: Duration,
) -> (
    TransferEngine,
    MemFs,
    FaultFs<MemFs>,
    MemFs,
    Receiver<Event>,
) {
    let local = MemFs::new();
    let local_fault = FaultFs::new(local.clone());
    local_fault.set_stream_latency(stream_latency);
    let remote = MemFs::new();
    let remote_fault = FaultFs::new(remote.clone());
    let config = EngineConfig {
        buf_size: 64 * 1024,
        ..EngineConfig::default()
    };
    let (engine, rx) = engine_with(
        Arc::new(local_fault),
        Arc::new(remote_fault.clone()),
        workers,
        config,
    )
    .await;
    (engine, local, remote_fault, remote, rx)
}

/// Retrying a failed child while a sibling is still copying keeps the
/// folder's counters right: it ends `Done` only after every child is done.
#[tokio::test(flavor = "multi_thread")]
async fn retry_failed_child_while_sibling_copies() {
    let (engine, local, remote_fault, remote, mut rx) =
        faulty_engine(1, Duration::from_millis(20)).await;
    let files: Vec<(String, Vec<u8>)> = ["a", "b", "c"]
        .iter()
        .enumerate()
        .map(|(i, n)| (n.to_string(), data(MIB / 2 + i)))
        .collect();
    for (name, bytes) in &files {
        local.add_file(&format!("/w/d/{name}"), bytes, mtime());
    }
    remote.add_dir("/srv");
    remote_fault.fail_next(Op::OpenWrite, ErrorKind::Permission);

    let root = engine
        .enqueue(spec(Direction::Upload, "/w/d", "/srv/d"))
        .await
        .unwrap();
    let ev = wait_for(&mut rx, |e| {
        matches!(
            e,
            EventKind::StateChanged {
                state: TransferState::Failed(_),
                ..
            }
        )
    })
    .await;
    let EventKind::StateChanged { id: a, .. } = ev else {
        unreachable!()
    };
    assert_ne!(a, root);
    engine.retry(a).await.unwrap();

    assert_eq!(wait_terminal(&mut rx, root).await, TransferState::Done);
    let snap = engine.snapshot().await;
    assert!(
        snap.iter().all(|i| i.state == TransferState::Done),
        "{snap:?}"
    );
    for (name, bytes) in &files {
        assert_eq!(
            remote.read_file(&format!("/srv/d/{name}")).as_ref(),
            Some(bytes)
        );
    }
}

/// Retrying the failed child of a folder that already finished `Failed`
/// reopens the folder, which then ends `Done`.
#[tokio::test(flavor = "multi_thread")]
async fn retry_failed_child_of_finished_folder() {
    let (engine, local, remote_fault, remote, mut rx) = faulty_engine(1, Duration::ZERO).await;
    local.add_file("/w/d/a", &data(100), mtime());
    local.add_file("/w/d/b", &data(200), mtime());
    remote.add_dir("/srv");
    remote_fault.fail_next(Op::OpenWrite, ErrorKind::Permission);

    let root = engine
        .enqueue(spec(Direction::Upload, "/w/d", "/srv/d"))
        .await
        .unwrap();
    assert!(matches!(
        wait_terminal(&mut rx, root).await,
        TransferState::Failed(_)
    ));
    let snap = engine.snapshot().await;
    let a = snap
        .iter()
        .find(|i| matches!(i.state, TransferState::Failed(_)) && i.id != root)
        .expect("a failed child")
        .id;

    engine.retry(a).await.unwrap();
    assert_eq!(wait_terminal(&mut rx, root).await, TransferState::Done);
    let snap = engine.snapshot().await;
    assert!(
        snap.iter().all(|i| i.state == TransferState::Done),
        "{snap:?}"
    );
    assert_eq!(remote.read_file("/srv/d/a").unwrap(), data(100));
}

/// "Apply to all" also covers an item whose prepare was already in flight
/// when the user answered: it does not ask again.
#[tokio::test(flavor = "multi_thread")]
async fn apply_to_all_covers_in_flight_prepare() {
    let local = MemFs::new();
    let remote = MemFs::new();
    let remote_fault = FaultFs::new(remote.clone());
    let (engine, mut rx) = engine_with(
        Arc::new(local.clone()),
        Arc::new(remote_fault.clone()),
        2,
        EngineConfig::default(),
    )
    .await;
    for name in ["a", "b"] {
        local.add_file(&format!("/w/{name}"), &data(1_000), mtime());
        remote.add_file(&format!("/srv/{name}"), b"old", mtime());
    }

    let a = engine
        .enqueue(spec(Direction::Upload, "/w/a", "/srv/a"))
        .await
        .unwrap();
    wait_awaiting(&mut rx, a).await;

    // b's prepare (its destination stat) is slow, so it is still in flight
    // when the user answers "skip all".
    remote_fault.set_latency(Duration::from_millis(300));
    let b = engine
        .enqueue(spec(Direction::Upload, "/w/b", "/srv/b"))
        .await
        .unwrap();
    wait_state(&mut rx, b, TransferState::Active).await;
    engine
        .resolve(a, resolution(ResolveAction::Skip, true))
        .await
        .unwrap();

    let ev = wait_for(&mut rx, |e| {
        matches!(e, EventKind::StateChanged { id, state }
            if *id == b && (state.is_terminal() || matches!(state, TransferState::AwaitingDecision(_))))
    })
    .await;
    let EventKind::StateChanged { state, .. } = ev else {
        unreachable!()
    };
    assert_eq!(state, TransferState::Skipped("exists".to_string()));
    assert_eq!(
        find(&engine.snapshot().await, a).state,
        TransferState::Skipped("skipped by user".to_string())
    );
    assert_eq!(remote.read_file("/srv/b").unwrap(), b"old");
}

/// Cancelling an item whose prepare is stuck (a destination `stat` that
/// takes 10 s) does not wait for it.
#[tokio::test(flavor = "multi_thread")]
async fn cancel_during_prepare_is_prompt() {
    let local = MemFs::new();
    let remote_mem = MemFs::new();
    let remote = FaultFs::new(remote_mem.clone());
    let (engine, mut rx) = engine_with(
        Arc::new(local.clone()),
        Arc::new(remote.clone()),
        1,
        EngineConfig::default(),
    )
    .await;
    local.add_file("/w/a.bin", &data(1_000), mtime());
    remote_mem.add_dir("/srv");
    remote.set_latency(Duration::from_secs(10));

    let id = engine
        .enqueue(spec(Direction::Upload, "/w/a.bin", "/srv/a.bin"))
        .await
        .unwrap();
    wait_state(&mut rx, id, TransferState::Active).await;
    let start = std::time::Instant::now();
    engine.cancel(id).await.unwrap();
    assert_eq!(wait_terminal(&mut rx, id).await, TransferState::Cancelled);
    assert!(
        start.elapsed() < Duration::from_secs(1),
        "took {:?}",
        start.elapsed()
    );
    assert!(remote_mem.read_file("/srv/a.bin").is_none());
}

/// A copy that failed partway (not a network error) is retried from the
/// start, even under `Skip`, which would otherwise keep the partial file.
#[tokio::test(flavor = "multi_thread")]
async fn retry_after_write_failure_overwrites_partial_destination() {
    retry_after_write_failure_under(hoprs_transfer::ConflictPolicy::Skip).await;
}

/// The same under `OverwriteIfNewer`: the partial destination is newer than
/// the source, which would otherwise skip it.
#[tokio::test(flavor = "multi_thread")]
async fn retry_after_write_failure_overwrites_partial_destination_if_newer() {
    retry_after_write_failure_under(hoprs_transfer::ConflictPolicy::OverwriteIfNewer).await;
}

/// Fails an upload partway with a write error, sets `policy` and retries
/// it: the item copies the whole source again (it never continues the
/// partial destination) and ends `Done`.
async fn retry_after_write_failure_under(policy: hoprs_transfer::ConflictPolicy) {
    let local = MemFs::new();
    let local_fault = FaultFs::new(local.clone());
    let remote_mem = MemFs::new();
    let remote = FaultFs::new(remote_mem.clone());
    let config = EngineConfig {
        buf_size: 64 * 1024,
        ..EngineConfig::default()
    };
    let (engine, mut rx) = engine_with(
        Arc::new(local_fault.clone()),
        Arc::new(remote.clone()),
        1,
        config,
    )
    .await;
    let bytes = data(MIB);
    local.add_file("/w/a.bin", &bytes, mtime());
    remote_mem.add_dir("/srv");
    remote.fail_io_after(300_000, ErrorKind::Io);

    let id = engine
        .enqueue(spec(Direction::Upload, "/w/a.bin", "/srv/a.bin"))
        .await
        .unwrap();
    match wait_terminal(&mut rx, id).await {
        TransferState::Failed(e) => assert_eq!(e.kind, ErrorKind::Io),
        other => panic!("expected Failed, got {other:?}"),
    }
    assert_eq!(
        remote_mem.read_file("/srv/a.bin").map(|f| f.len()),
        Some(300_000)
    );

    engine.set_policy(policy).await;
    engine.retry(id).await.unwrap();
    assert_eq!(wait_terminal(&mut rx, id).await, TransferState::Done);
    assert!(remote_mem.read_file("/srv/a.bin").unwrap() == bytes);
    assert_eq!(
        local_fault.read_offsets(),
        vec![0, 0],
        "the retry starts over"
    );
}

/// A copy cancelled partway continues from its partial destination when
/// retried, even under `Skip`.
#[tokio::test(flavor = "multi_thread")]
async fn retry_after_cancel_resumes_partial_destination() {
    let Slow {
        engine,
        fault,
        local,
        remote,
        mut rx,
    } = slow_upload_engine(1, Duration::from_millis(20), EngineConfig::default()).await;
    let bytes = data(4 * MIB);
    local.add_file("/w/a.bin", &bytes, mtime());
    remote.add_dir("/srv");

    let id = engine
        .enqueue(spec(Direction::Upload, "/w/a.bin", "/srv/a.bin"))
        .await
        .unwrap();
    wait_progress(&mut rx).await;
    engine.cancel(id).await.unwrap();
    wait_state(&mut rx, id, TransferState::Cancelled).await;
    let partial = remote.read_file("/srv/a.bin").map_or(0, |f| f.len()) as u64;
    assert!(partial > 0 && partial < bytes.len() as u64, "{partial}");

    engine
        .set_policy(hoprs_transfer::ConflictPolicy::Skip)
        .await;
    engine.retry(id).await.unwrap();
    assert_eq!(wait_terminal(&mut rx, id).await, TransferState::Done);
    assert!(remote.read_file("/srv/a.bin").unwrap() == bytes);
    assert_eq!(fault.read_offsets(), vec![0, partial]);
}

/// A write error that only surfaces when the destination is shut down (as
/// with a pipelined writer) fails a paused copy instead of pausing it: the
/// destination may hold misplaced data below its size, so it must never be
/// continued. A retry under `Skip` rewrites it from the start.
#[tokio::test(flavor = "multi_thread")]
async fn pause_with_deferred_write_error_fails_and_retry_overwrites() {
    let local = MemFs::new();
    let local_fault = FaultFs::new(local.clone());
    local_fault.set_stream_latency(Duration::from_millis(20));
    let remote_mem = MemFs::new();
    let remote = FaultFs::new(remote_mem.clone());
    let config = EngineConfig {
        buf_size: 64 * 1024,
        ..EngineConfig::default()
    };
    let (engine, mut rx) = engine_with(
        Arc::new(local_fault.clone()),
        Arc::new(remote.clone()),
        1,
        config,
    )
    .await;
    let bytes = data(4 * MIB);
    local.add_file("/w/a.bin", &bytes, mtime());
    remote_mem.add_dir("/srv");
    remote.lose_write_after(100_000, ErrorKind::Io);

    let id = engine
        .enqueue(spec(Direction::Upload, "/w/a.bin", "/srv/a.bin"))
        .await
        .unwrap();
    // Wait until a chunk written after the lost one has landed.
    tokio::time::timeout(Duration::from_secs(5), async {
        while remote_mem
            .read_file("/srv/a.bin")
            .is_none_or(|f| f.len() < 100_000)
        {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("chunks after the lost one land");
    engine.pause(id).await.unwrap();
    let stopped = wait_for(&mut rx, |e| {
        matches!(e, EventKind::StateChanged { id: i, state } if *i == id
            && (state.is_terminal() || *state == TransferState::Paused))
    })
    .await;
    match stopped {
        EventKind::StateChanged {
            state: TransferState::Failed(e),
            ..
        } => assert_eq!(e.kind, ErrorKind::Io),
        other => panic!("expected Failed, got {other:?}"),
    }

    engine
        .set_policy(hoprs_transfer::ConflictPolicy::Skip)
        .await;
    engine.retry(id).await.unwrap();
    assert_eq!(wait_terminal(&mut rx, id).await, TransferState::Done);
    assert!(remote_mem.read_file("/srv/a.bin").unwrap() == bytes);
    assert_eq!(
        local_fault.read_offsets(),
        vec![0, 0],
        "the retry starts over"
    );
}

#[tokio::test]
async fn stopped_resolves_after_shutdown_of_another_handle() {
    let (engine, _local, _remote, _rx) = engine_with_memfs(1).await;
    let waiter = engine.clone();
    let stopped = tokio::spawn(async move { waiter.stopped().await });
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!stopped.is_finished(), "still running");
    engine.clone().shutdown().await;
    tokio::time::timeout(Duration::from_secs(1), stopped)
        .await
        .expect("stopped resolves")
        .unwrap();
    // Already stopped: resolves at once.
    tokio::time::timeout(Duration::from_secs(1), engine.stopped())
        .await
        .expect("stopped resolves again");
}

/// Progress contract (Plan 6, ruling R-a): when the last `Progress` is less than an
/// interval old, a file's final count rides with the next one, after its `Done`. Every
/// file's final count still arrives, before the engine goes quiet.
#[tokio::test(flavor = "multi_thread")]
async fn final_counts_arrive_after_done_within_an_interval() {
    const INTERVAL: Duration = Duration::from_millis(200);
    let config = EngineConfig {
        progress_interval: INTERVAL,
        ..EngineConfig::default()
    };
    let local = MemFs::new();
    let remote = MemFs::new();
    remote.add_dir("/srv");
    let sizes: HashMap<String, usize> = (0..24).map(|i| (format!("f{i:02}"), 1000 + i)).collect();
    for (name, size) in &sizes {
        local.add_file(&format!("/w/tree/{name}"), &data(*size), mtime());
    }
    let (engine, mut rx) = engine_with(Arc::new(local), Arc::new(remote), 4, config).await;
    let root = engine
        .enqueue(spec(Direction::Upload, "/w/tree", "/srv/tree"))
        .await
        .unwrap();

    let mut files: HashMap<TransferId, u64> = HashMap::new();
    let mut done: HashSet<TransferId> = HashSet::new();
    // Final counts (bytes == size), and whether each came after its `Done`.
    let mut finals: HashMap<TransferId, bool> = HashMap::new();
    let mut quiet_from = None;
    loop {
        let wait = match quiet_from {
            // After the root is done: a final count can still come with the next tick.
            Some(_) => INTERVAL * 3,
            None => Duration::from_secs(5),
        };
        let Ok(event) = tokio::time::timeout(wait, rx.recv()).await else {
            break;
        };
        match event.unwrap().kind {
            EventKind::Added(item) if item.parent == Some(root) => {
                let name = item.spec.local.file_name().unwrap().to_owned();
                files.insert(item.id, sizes[&name] as u64);
            }
            EventKind::StateChanged {
                id,
                state: TransferState::Done,
            } => {
                done.insert(id);
                if id == root {
                    quiet_from = Some(());
                }
            }
            EventKind::Progress(updates) => {
                for u in updates {
                    if files.get(&u.id) == Some(&u.bytes_done) {
                        finals.insert(u.id, done.contains(&u.id));
                    }
                }
            }
            _ => {}
        }
    }
    assert!(done.contains(&root), "the folder finished");
    assert_eq!(files.len(), sizes.len());
    for id in files.keys() {
        assert!(finals.contains_key(id), "no final count for {id:?}");
    }
    assert!(
        finals.values().any(|after| *after),
        "some final counts came after their Done: {finals:?}"
    );
    engine.shutdown().await;
}

/// A paused copy's final count is pending (the last `Progress` is recent) when the user
/// resumes it: the new run drops it, so it never follows the new run's own counts.
#[tokio::test(flavor = "multi_thread")]
async fn resumed_run_drops_the_pending_final_count() {
    let config = EngineConfig {
        buf_size: 16 * 1024,
        progress_interval: Duration::from_secs(1),
        ..EngineConfig::default()
    };
    let Slow {
        engine,
        local,
        remote,
        mut rx,
        ..
    } = slow_upload_engine(1, Duration::from_millis(10), config).await;
    // About 3 s of copying: ticks come while the resumed run copies.
    let bytes = data(5 * MIB);
    local.add_file("/w/a.bin", &bytes, mtime());
    remote.add_dir("/srv");
    let id = engine
        .enqueue(spec(Direction::Upload, "/w/a.bin", "/srv/a.bin"))
        .await
        .unwrap();
    // A sampled `Progress` goes out at once: the final count of the pause waits.
    wait_progress(&mut rx).await;
    engine.pause(id).await.unwrap();
    wait_state(&mut rx, id, TransferState::Paused).await;
    // The paused run's final count: the new run starts from it and only reports counts
    // past it, so an update with exactly this count after the resume is the stale one.
    let paused = find(&engine.snapshot().await, id).bytes_done;
    assert!(
        paused > 0 && paused < bytes.len() as u64,
        "paused at {paused}"
    );
    engine.resume(id).await.unwrap();
    wait_state(&mut rx, id, TransferState::Active).await;

    let mut stale = Vec::new();
    loop {
        match wait_for(&mut rx, |_| true).await {
            EventKind::Progress(updates) => stale.extend(
                updates
                    .into_iter()
                    .filter(|u| u.id == id && u.bytes_done == paused),
            ),
            EventKind::StateChanged { id: i, state } if i == id && state.is_terminal() => {
                assert_eq!(state, TransferState::Done);
                break;
            }
            _ => {}
        }
    }
    assert!(
        stale.is_empty(),
        "the paused run's count followed: {stale:?}"
    );
    assert_eq!(remote.read_file("/srv/a.bin").unwrap(), bytes);
}

/// The files of a folder this run creates skip the conflict check (the folder is empty):
/// one `stat` less per file than into an existing folder.
#[tokio::test(flavor = "multi_thread")]
async fn new_folder_skips_the_conflict_check() {
    async fn remote_ops(existing: bool) -> usize {
        let local = MemFs::new();
        for i in 0..10 {
            local.add_file(&format!("/w/tree/f{i}"), &data(100), mtime());
        }
        let mem = MemFs::new();
        mem.add_dir("/srv");
        if existing {
            mem.add_dir("/srv/tree");
        }
        let remote = FaultFs::new(mem);
        let (engine, mut rx) = engine_with(
            Arc::new(local),
            Arc::new(remote.clone()),
            2,
            EngineConfig::default(),
        )
        .await;
        let root = engine
            .enqueue(spec(Direction::Upload, "/w/tree", "/srv/tree"))
            .await
            .unwrap();
        assert_eq!(wait_terminal(&mut rx, root).await, TransferState::Done);
        engine.shutdown().await;
        remote.ops()
    }
    assert_eq!(remote_ops(true).await - remote_ops(false).await, 10);
}

/// A run resumed under a folder this run created still checks the destination: the
/// partial file is found and the copy continues from its end.
#[tokio::test(flavor = "multi_thread")]
async fn resumed_file_in_new_folder_continues_from_offset() {
    let Slow {
        engine,
        fault,
        local,
        remote,
        mut rx,
    } = slow_upload_engine(1, Duration::from_millis(20), EngineConfig::default()).await;
    let bytes = data(4 * MIB);
    local.add_file("/w/tree/a.bin", &bytes, mtime());
    remote.add_dir("/srv");
    let root = engine
        .enqueue(spec(Direction::Upload, "/w/tree", "/srv/tree"))
        .await
        .unwrap();
    wait_progress(&mut rx).await;
    let child = engine
        .snapshot()
        .await
        .into_iter()
        .find(|i| i.parent == Some(root))
        .unwrap()
        .id;
    engine.pause(child).await.unwrap();
    wait_state(&mut rx, child, TransferState::Paused).await;
    let paused = find(&engine.snapshot().await, child).bytes_done;
    assert!(
        paused > 0 && paused < bytes.len() as u64,
        "paused at {paused}"
    );

    engine.resume(child).await.unwrap();
    assert_eq!(wait_terminal(&mut rx, child).await, TransferState::Done);
    assert_eq!(remote.read_file("/srv/tree/a.bin").unwrap(), bytes);
    assert_eq!(fault.read_offsets(), vec![0, paused]);
}

/// Two items write into the same new folder: the tree `/a/tree` into `/srv/tree` (it
/// creates the folder) and the file `/b/x` into `/srv/tree/x`, enqueued while the folder
/// is being prepared. The folder has two writers, so its files keep their conflict check:
/// `/b/x` goes first (lower id), and the tree's `x` then meets the conflict policy (`Ask`:
/// waits for a decision; `Skip`: skipped). It never overwrites `/b/x`'s copy.
#[tokio::test(flavor = "multi_thread")]
async fn shared_new_folder_keeps_the_conflict_check() {
    use hoprs_transfer::ConflictPolicy;
    for policy in [ConflictPolicy::Ask, ConflictPolicy::Skip] {
        let local = MemFs::new();
        let (a, b) = (data(3000), data(5000));
        local.add_file("/a/tree/x", &a, mtime());
        local.add_file("/b/x", &b, mtime());
        let mem = MemFs::new();
        mem.add_dir("/srv");
        let remote = FaultFs::new(mem.clone());
        // The folder's preparation outlasts the second enqueue.
        remote.set_latency(Duration::from_millis(50));
        let config = EngineConfig {
            default_policy: policy,
            ..EngineConfig::default()
        };
        let (engine, mut rx) = engine_with(Arc::new(local), Arc::new(remote), 1, config).await;
        let tree = engine
            .enqueue(spec(Direction::Upload, "/a/tree", "/srv/tree"))
            .await
            .unwrap();
        let file = engine
            .enqueue(spec(Direction::Upload, "/b/x", "/srv/tree/x"))
            .await
            .unwrap();
        assert_eq!(wait_terminal(&mut rx, file).await, TransferState::Done);
        // The tree's own `x`.
        let x = wait_for(&mut rx, |e| {
            matches!(e, EventKind::StateChanged { id, state }
                if *id != tree && *id != file
                    && (state.is_terminal() || matches!(state, TransferState::AwaitingDecision(_))))
        })
        .await;
        let EventKind::StateChanged { state, .. } = x else {
            unreachable!()
        };
        match policy {
            ConflictPolicy::Ask => assert!(
                matches!(state, TransferState::AwaitingDecision(_)),
                "{state:?}"
            ),
            _ => assert!(matches!(state, TransferState::Skipped(_)), "{state:?}"),
        }
        assert_eq!(
            mem.read_file("/srv/tree/x").unwrap(),
            b,
            "{policy:?}: overwritten"
        );
        engine.shutdown().await;
    }
}
