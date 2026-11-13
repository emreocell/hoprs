//! Engine surface used by the UI: event sequence numbers, snapshots that
//! bound stale events, fallible start, clearing finished items, the final
//! progress event and immediate cancellation.

mod common;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::*;
use hoprs_core::SiteId;
use hoprs_remote::{FaultFs, MemFs};
use hoprs_transfer::{
    Direction, EngineConfig, Event, EventKind, QueueStore, TransferEngine, TransferId,
    TransferSpec, TransferState,
};
use tokio::sync::broadcast::Receiver;
use tokio::sync::broadcast::error::RecvError;

const MIB: usize = 1 << 20;
const WAIT: Duration = Duration::from_secs(10);

/// Collects every event until each id in `ids` has reached a terminal state.
async fn collect_until_terminal(rx: &mut Receiver<Event>, ids: &[TransferId]) -> Vec<Event> {
    let mut out = Vec::new();
    let mut finished = std::collections::HashSet::new();
    let fut = async {
        while finished.len() < ids.len() {
            match rx.recv().await {
                Ok(ev) => {
                    if let EventKind::StateChanged { id, state } = &ev.kind
                        && ids.contains(id)
                    {
                        if state.is_terminal() {
                            finished.insert(*id);
                        } else {
                            finished.remove(id);
                        }
                    }
                    out.push(ev);
                }
                Err(RecvError::Lagged(n)) => panic!("lagged by {n} events"),
                Err(RecvError::Closed) => panic!("event channel closed"),
            }
        }
    };
    tokio::time::timeout(WAIT, fut)
        .await
        .expect("timed out waiting for terminal states");
    out
}

#[tokio::test]
async fn events_have_increasing_seq() {
    let (engine, local, remote, mut rx) = engine_with_memfs(2).await;
    local.add_file("/w/a.bin", &data(64 * 1024), mtime());
    local.add_dir("/w/d");
    local.add_file("/w/d/x.bin", &data(10), mtime());
    local.add_file("/w/d/y.bin", &data(20), mtime());
    remote.add_dir("/srv");

    let a = engine
        .enqueue(spec(Direction::Upload, "/w/a.bin", "/srv/a.bin"))
        .await
        .unwrap();
    let d = engine
        .enqueue(spec(Direction::Upload, "/w/d", "/srv/d"))
        .await
        .unwrap();
    let events = collect_until_terminal(&mut rx, &[a, d]).await;

    assert!(events.len() > 4, "got {} events", events.len());
    // Subscribed before anything happened, and never lagged: every event,
    // numbered from 1 without gaps.
    let seqs: Vec<u64> = events.iter().map(|e| e.seq).collect();
    let expected: Vec<u64> = (1..=events.len() as u64).collect();
    assert_eq!(seqs, expected);
}

#[tokio::test(flavor = "multi_thread")]
async fn snapshot_seq_bounds_stale_events() {
    let (engine, local, remote, mut rx) = engine_with_memfs(2).await;
    remote.add_dir("/srv");
    let mut ids = Vec::new();
    for i in 0..30 {
        local.add_file(&format!("/w/f{i}.bin"), &data(32 * 1024), mtime());
        ids.push(
            engine
                .enqueue(spec(
                    Direction::Upload,
                    &format!("/w/f{i}.bin"),
                    &format!("/srv/f{i}.bin"),
                ))
                .await
                .unwrap(),
        );
    }
    let (snap_seq, snap) = engine.snapshot_with_seq().await;
    let events = collect_until_terminal(&mut rx, &ids).await;

    // Every `Added` was emitted before `enqueue` returned, so before the
    // snapshot.
    for ev in &events {
        if let EventKind::Added(item) = &ev.kind {
            assert!(
                ev.seq <= snap_seq,
                "Added {:?} seq {} > {snap_seq}",
                item.id,
                ev.seq
            );
        }
    }

    // The snapshot already reflects every event up to its seq: the last
    // state change at or below it is the snapshot's state.
    let mut before: HashMap<TransferId, TransferState> = HashMap::new();
    for ev in events.iter().filter(|e| e.seq <= snap_seq) {
        if let EventKind::StateChanged { id, state } = &ev.kind {
            before.insert(*id, state.clone());
        }
    }
    for item in &snap {
        if let Some(state) = before.get(&item.id) {
            assert_eq!(&item.state, state, "item {:?}", item.id);
        }
    }

    // Applying only the events after the snapshot gives the final state.
    let mut states: HashMap<TransferId, TransferState> =
        snap.iter().map(|i| (i.id, i.state.clone())).collect();
    for ev in events.iter().filter(|e| e.seq > snap_seq) {
        match &ev.kind {
            EventKind::Added(item) => {
                states.insert(item.id, item.state.clone());
            }
            EventKind::StateChanged { id, state } => {
                states.insert(*id, state.clone());
            }
            EventKind::Progress(_) | EventKind::SitePaused { .. } => {}
        }
    }
    let (_, last) = engine.snapshot_with_seq().await;
    let final_states: HashMap<TransferId, TransferState> =
        last.iter().map(|i| (i.id, i.state.clone())).collect();
    assert_eq!(states, final_states);
    assert!(final_states.values().all(|s| *s == TransferState::Done));
}

#[tokio::test]
async fn start_reports_recover_error() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("queue.db");
    {
        let mut store = QueueStore::open(&db).unwrap();
        store
            .insert(None, &spec(Direction::Upload, "/w/a.bin", "/srv/a.bin"))
            .unwrap();
    }
    {
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute("UPDATE transfers SET state = 'bogus'", [])
            .unwrap();
    }

    let result = TransferEngine::start(QueueStore::open(&db).unwrap(), EngineConfig::default());
    assert!(result.is_err(), "a corrupt queue must fail the start");

    // The failed start wrote nothing over the stored queue.
    let conn = rusqlite::Connection::open(&db).unwrap();
    let state: String = conn
        .query_row("SELECT state FROM transfers", [], |r| r.get(0))
        .unwrap();
    assert_eq!(state, "bogus");
}

#[tokio::test]
async fn clear_finished_prunes_actor_maps() {
    let (engine, local, remote, mut rx) = engine_with_memfs(2).await;
    local.add_file("/w/a.bin", &data(1000), mtime());
    local.add_dir("/w/d");
    local.add_dir("/w/d/sub");
    local.add_file("/w/d/x.bin", &data(10), mtime());
    local.add_file("/w/d/sub/y.bin", &data(20), mtime());
    remote.add_dir("/srv");

    let a = engine
        .enqueue(spec(Direction::Upload, "/w/a.bin", "/srv/a.bin"))
        .await
        .unwrap();
    let d = engine
        .enqueue(spec(Direction::Upload, "/w/d", "/srv/d"))
        .await
        .unwrap();
    // A site that is never registered: this item stays queued.
    let waiting = engine
        .enqueue(TransferSpec {
            site: SiteId("elsewhere".to_string()),
            direction: Direction::Upload,
            local: path("/w/a.bin"),
            remote: path("/srv/other.bin"),
        })
        .await
        .unwrap();
    collect_until_terminal(&mut rx, &[a, d]).await;
    assert_eq!(engine.snapshot().await.len(), 6, "a, d, sub, x, y, waiting");

    assert_eq!(engine.clear_finished().await.unwrap(), 2);
    let snap = engine.snapshot().await;
    let ids: Vec<TransferId> = snap.iter().map(|i| i.id).collect();
    assert_eq!(
        ids,
        vec![waiting],
        "finished roots and their children are gone"
    );

    // Nothing left to clear.
    assert_eq!(engine.clear_finished().await.unwrap(), 0);
    assert_eq!(engine.snapshot().await.len(), 1);
}

#[tokio::test]
async fn final_progress_before_done() {
    // No periodic progress sample during the copy: only the final one.
    let config = EngineConfig {
        progress_interval: Duration::from_secs(3600),
        ..EngineConfig::default()
    };
    let local = MemFs::new();
    let remote = MemFs::new();
    let size = MIB + 123;
    local.add_file("/w/a.bin", &data(size), mtime());
    remote.add_dir("/srv");
    let (engine, mut rx) = engine_with(Arc::new(local), Arc::new(remote.clone()), 1, config).await;

    let id = engine
        .enqueue(spec(Direction::Upload, "/w/a.bin", "/srv/a.bin"))
        .await
        .unwrap();
    let events = collect_until_terminal(&mut rx, &[id]).await;

    let done_at = events
        .iter()
        .position(|e| matches!(&e.kind, EventKind::StateChanged { id: i, state } if *i == id && *state == TransferState::Done))
        .expect("Done event");
    let EventKind::Progress(updates) = &events[done_at - 1].kind else {
        panic!(
            "the event before Done is not Progress: {:?}",
            events[done_at - 1]
        );
    };
    let update = updates
        .iter()
        .find(|u| u.id == id)
        .expect("progress for the item");
    assert_eq!(update.bytes_done, size as u64);
    assert_eq!(update.total, Some(size as u64));
}

#[tokio::test(flavor = "multi_thread")]
async fn cancel_during_stalled_read_is_immediate() {
    let local = MemFs::new();
    local.add_file("/w/a.bin", &data(MIB), mtime());
    let fault = FaultFs::new(local);
    // Every chunk read waits 10 s: the copy stalls on its first read.
    fault.set_stream_latency(Duration::from_secs(10));
    let remote = MemFs::new();
    remote.add_dir("/srv");
    let (engine, mut rx) = engine_with(
        Arc::new(fault.clone()),
        Arc::new(remote),
        1,
        EngineConfig::default(),
    )
    .await;

    let id = engine
        .enqueue(spec(Direction::Upload, "/w/a.bin", "/srv/a.bin"))
        .await
        .unwrap();
    // Wait until the copy has opened its source (it is then reading).
    tokio::time::timeout(WAIT, async {
        while fault.read_offsets().is_empty() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("copy started");
    tokio::time::sleep(Duration::from_millis(50)).await;

    let started = Instant::now();
    engine.cancel(id).await.unwrap();
    let state = wait_terminal(&mut rx, id).await;
    let elapsed = started.elapsed();
    assert_eq!(state, TransferState::Cancelled);
    assert!(elapsed < Duration::from_secs(1), "cancel took {elapsed:?}");
}
