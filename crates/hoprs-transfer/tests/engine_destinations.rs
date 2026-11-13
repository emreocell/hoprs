//! One writer per destination: two queued items never write the same file at once.

mod common;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use common::*;
use hoprs_core::SiteId;
use hoprs_remote::{FaultFs, MemFs};
use hoprs_transfer::{
    ConflictPolicy, Direction, EngineConfig, EventKind, SiteEndpoints, TransferId, TransferSpec,
    TransferState,
};
use tokio::sync::broadcast::Receiver;

use hoprs_transfer::Event;

const LONG: Duration = Duration::from_secs(20);

fn config(policy: ConflictPolicy) -> EngineConfig {
    EngineConfig {
        default_policy: policy,
        ..EngineConfig::default()
    }
}

/// A file transfer slow enough to overlap with others.
fn slow(mem: &MemFs) -> FaultFs<MemFs> {
    let fs = FaultFs::new(mem.clone());
    fs.set_stream_latency(Duration::from_millis(2));
    fs
}

fn bytes(fill: u8, len: usize) -> Vec<u8> {
    vec![fill; len]
}

/// Collects the final (terminal, or awaiting-decision) state of every id in `ids`,
/// and whether any of them was `Active` while another one also was.
async fn settle(
    rx: &mut Receiver<Event>,
    ids: &[TransferId],
) -> HashMap<TransferId, TransferState> {
    let mut done = HashMap::new();
    let fut = async {
        while done.len() < ids.len() {
            match rx.recv().await {
                Ok(ev) => {
                    if let EventKind::StateChanged { id, state } = ev.kind
                        && ids.contains(&id)
                        && (state.is_terminal()
                            || matches!(state, TransferState::AwaitingDecision(_)))
                    {
                        done.insert(id, state);
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(e) => panic!("{e}"),
            }
        }
    };
    tokio::time::timeout(LONG, fut)
        .await
        .expect("items never settled");
    done
}

/// Every id of the same destination: exactly one `Done`, the others met the policy, none
/// failed, and the file holds exactly the `Done` one's bytes.
fn assert_one_writer(
    states: &HashMap<TransferId, TransferState>,
    policy: ConflictPolicy,
    contents: &HashMap<TransferId, Vec<u8>>,
    actual: &[u8],
) {
    let done: Vec<_> = states
        .iter()
        .filter(|(_, s)| matches!(s, TransferState::Done))
        .map(|(id, _)| *id)
        .collect();
    assert_eq!(done.len(), 1, "exactly one writer: {states:?}");
    for (id, state) in states {
        if *id == done[0] {
            continue;
        }
        match policy {
            ConflictPolicy::Ask => assert!(
                matches!(state, TransferState::AwaitingDecision(_)),
                "{id:?}: {state:?}"
            ),
            _ => assert!(
                matches!(state, TransferState::Skipped(_)),
                "{id:?}: {state:?}"
            ),
        }
    }
    assert!(
        actual == contents[&done[0]].as_slice(),
        "the file mixes writers' bytes"
    );
}

/// Two trees uploaded into one new folder, each with `x`: the second `x` meets the
/// policy, and the folder's `x` is exactly the first one's bytes.
#[tokio::test(flavor = "multi_thread")]
async fn two_trees_into_one_new_folder_never_mix_bytes() {
    for policy in [ConflictPolicy::Ask, ConflictPolicy::Skip] {
        let local = MemFs::new();
        let (a, b) = (bytes(1, 300_000), bytes(2, 200_000));
        local.add_file("/a/tree/x", &a, mtime());
        local.add_file("/b/tree/x", &b, mtime());
        let mem = MemFs::new();
        mem.add_dir("/srv");
        let (engine, mut rx) =
            engine_with(Arc::new(local), Arc::new(slow(&mem)), 2, config(policy)).await;
        engine
            .enqueue(spec(Direction::Upload, "/a/tree", "/srv/tree"))
            .await
            .unwrap();
        engine
            .enqueue(spec(Direction::Upload, "/b/tree", "/srv/tree"))
            .await
            .unwrap();
        // Find the two `x` children.
        let mut xs = Vec::new();
        let fut = async {
            while xs.len() < 2 {
                if let Ok(ev) = rx.recv().await
                    && let EventKind::Added(item) = ev.kind
                    && item.spec.remote.as_str() == "/srv/tree/x"
                {
                    xs.push((item.id, item.spec.local.as_str().to_string()));
                }
            }
        };
        tokio::time::timeout(LONG, fut)
            .await
            .expect("children never added");
        let ids: Vec<_> = xs.iter().map(|(id, _)| *id).collect();
        let contents: HashMap<_, _> = xs
            .iter()
            .map(|(id, src)| {
                (
                    *id,
                    if src.starts_with("/a/") {
                        a.clone()
                    } else {
                        b.clone()
                    },
                )
            })
            .collect();
        let states = settle(&mut rx, &ids).await;
        assert_one_writer(
            &states,
            policy,
            &contents,
            &mem.read_file("/srv/tree/x").unwrap(),
        );
        engine.shutdown().await;
    }
}

/// Two single files sent to the same remote path at once.
#[tokio::test(flavor = "multi_thread")]
async fn two_files_to_one_destination_never_mix_bytes() {
    let local = MemFs::new();
    let (a, b) = (bytes(1, 300_000), bytes(2, 200_000));
    local.add_file("/a/x", &a, mtime());
    local.add_file("/b/x", &b, mtime());
    let mem = MemFs::new();
    mem.add_dir("/srv");
    let (engine, mut rx) = engine_with(
        Arc::new(local),
        Arc::new(slow(&mem)),
        2,
        config(ConflictPolicy::Skip),
    )
    .await;
    let first = engine
        .enqueue(spec(Direction::Upload, "/a/x", "/srv/x"))
        .await
        .unwrap();
    let second = engine
        .enqueue(spec(Direction::Upload, "/b/x", "/srv/x"))
        .await
        .unwrap();
    let states = settle(&mut rx, &[first, second]).await;
    let contents = HashMap::from([(first, a), (second, b)]);
    assert_one_writer(
        &states,
        ConflictPolicy::Skip,
        &contents,
        &mem.read_file("/srv/x").unwrap(),
    );
    engine.shutdown().await;
}

fn site_spec(site: &str, direction: Direction, local: &str, remote: &str) -> TransferSpec {
    TransferSpec {
        site: SiteId(site.to_string()),
        direction,
        local: path(local),
        remote: path(remote),
    }
}

async fn register(
    engine: &hoprs_transfer::TransferEngine,
    site: &str,
    local: &MemFs,
    remote: FaultFs<MemFs>,
    workers: usize,
) {
    engine
        .register_site(
            SiteId(site.to_string()),
            SiteEndpoints {
                local: Arc::new(local.clone()),
                remote: Arc::new(remote),
                workers,
            },
        )
        .await;
}

/// Two sites downloading to the same local file: one local destination, one writer.
#[tokio::test(flavor = "multi_thread")]
async fn downloads_from_two_sites_to_one_local_file_are_serialized() {
    let local = MemFs::new();
    local.add_dir("/dl");
    let (a, b) = (bytes(1, 300_000), bytes(2, 200_000));
    let (r1, r2) = (MemFs::new(), MemFs::new());
    r1.add_file("/x", &a, mtime());
    r2.add_file("/x", &b, mtime());
    let (engine, mut rx) = start_engine(config(ConflictPolicy::Skip));
    register(&engine, "s1", &local, slow(&r1), 2).await;
    register(&engine, "s2", &local, slow(&r2), 2).await;
    let first = engine
        .enqueue(site_spec("s1", Direction::Download, "/dl/x", "/x"))
        .await
        .unwrap();
    let second = engine
        .enqueue(site_spec("s2", Direction::Download, "/dl/x", "/x"))
        .await
        .unwrap();
    let states = settle(&mut rx, &[first, second]).await;
    let contents = HashMap::from([(first, a), (second, b)]);
    assert_one_writer(
        &states,
        ConflictPolicy::Skip,
        &contents,
        &local.read_file("/dl/x").unwrap(),
    );
    engine.shutdown().await;
}

/// Uploads to the same path on two different servers are independent: both run at once.
#[tokio::test(flavor = "multi_thread")]
async fn uploads_to_the_same_path_on_two_sites_run_in_parallel() {
    let local = MemFs::new();
    local.add_file("/a/x", &bytes(1, 400_000), mtime());
    let (r1, r2) = (MemFs::new(), MemFs::new());
    r1.add_dir("/srv");
    r2.add_dir("/srv");
    let (engine, mut rx) = start_engine(config(ConflictPolicy::Skip));
    register(&engine, "s1", &local, slow(&r1), 2).await;
    register(&engine, "s2", &local, slow(&r2), 2).await;
    let first = engine
        .enqueue(site_spec("s1", Direction::Upload, "/a/x", "/srv/x"))
        .await
        .unwrap();
    let second = engine
        .enqueue(site_spec("s2", Direction::Upload, "/a/x", "/srv/x"))
        .await
        .unwrap();
    let mut active = Vec::new();
    let fut = async {
        loop {
            if let Ok(ev) = rx.recv().await
                && let EventKind::StateChanged { id, state } = ev.kind
            {
                match state {
                    TransferState::Active => active.push(id),
                    s if s.is_terminal() => break,
                    _ => {}
                }
            }
        }
    };
    tokio::time::timeout(LONG, fut)
        .await
        .expect("never settled");
    assert!(
        active.contains(&first) && active.contains(&second),
        "both were active before either finished: {active:?}"
    );
    engine.shutdown().await;
}

/// A deferred item stays queued while the owner of its destination runs, and starts
/// once the owner pauses, is cancelled or fails.
#[tokio::test(flavor = "multi_thread")]
async fn deferred_item_starts_when_the_owner_pauses_fails_or_is_cancelled() {
    for how in ["pause", "cancel", "fail"] {
        let local = MemFs::new();
        local.add_file("/a/x", &bytes(1, 4_000_000), mtime());
        local.add_file("/b/x", &bytes(2, 1_000), mtime());
        let mem = MemFs::new();
        mem.add_dir("/srv");
        let remote = FaultFs::new(mem.clone());
        // Slow enough that the owner is still copying when the second item arrives.
        remote.set_stream_latency(Duration::from_millis(50));
        if how == "fail" {
            remote.fail_io_after(200_000, hoprs_core::ErrorKind::Permission);
        }
        let (engine, mut rx) = engine_with(
            Arc::new(local),
            Arc::new(remote),
            2,
            config(ConflictPolicy::Skip),
        )
        .await;
        let owner = engine
            .enqueue(spec(Direction::Upload, "/a/x", "/srv/x"))
            .await
            .unwrap();
        // The owner is copying before the second item exists.
        wait_for(&mut rx, |e| matches!(e, EventKind::Progress(_))).await;
        let deferred = engine
            .enqueue(spec(Direction::Upload, "/b/x", "/srv/x"))
            .await
            .unwrap();
        match how {
            "pause" => engine.pause(owner).await.unwrap(),
            "cancel" => engine.cancel(owner).await.unwrap(),
            _ => {}
        }
        // The deferred item never becomes Active while the owner still is.
        let mut owner_active = true;
        let fut = async {
            loop {
                if let Ok(ev) = rx.recv().await
                    && let EventKind::StateChanged { id, state } = ev.kind
                {
                    if id == owner && !matches!(state, TransferState::Active) {
                        owner_active = false;
                    }
                    if id == deferred && matches!(state, TransferState::Active) {
                        assert!(!owner_active, "{how}: deferred ran beside its owner");
                    }
                    if id == deferred && state.is_terminal() {
                        return state;
                    }
                }
            }
        };
        let end = tokio::time::timeout(LONG, fut)
            .await
            .expect("deferred never ran");
        // The owner left a partial file: the deferred item met the policy.
        assert!(matches!(end, TransferState::Skipped(_)), "{how}: {end:?}");
        engine.shutdown().await;
    }
}

/// Many items for one destination do not hold back the site's other items, and leave
/// exactly one writer.
#[tokio::test(flavor = "multi_thread")]
async fn many_deferred_items_do_not_block_other_items() {
    let local = MemFs::new();
    for i in 0..20 {
        local.add_file(&format!("/same/{i}"), &bytes(i as u8, 50_000), mtime());
    }
    for i in 0..5 {
        local.add_file(&format!("/other/{i}"), &bytes(100, 50_000), mtime());
    }
    let mem = MemFs::new();
    mem.add_dir("/srv");
    let (engine, mut rx) = engine_with(
        Arc::new(local),
        Arc::new(slow(&mem)),
        4,
        config(ConflictPolicy::Skip),
    )
    .await;
    let mut same = Vec::new();
    for i in 0..20 {
        same.push(
            engine
                .enqueue(spec(
                    Direction::Upload,
                    &format!("/same/{i}"),
                    "/srv/target",
                ))
                .await
                .unwrap(),
        );
    }
    let mut others = Vec::new();
    for i in 0..5 {
        others.push(
            engine
                .enqueue(spec(
                    Direction::Upload,
                    &format!("/other/{i}"),
                    &format!("/srv/o{i}"),
                ))
                .await
                .unwrap(),
        );
    }
    let all: Vec<_> = same.iter().chain(&others).copied().collect();
    let states = settle(&mut rx, &all).await;
    for id in &others {
        assert_eq!(states[id], TransferState::Done, "{id:?}");
    }
    let done = same
        .iter()
        .filter(|id| states[*id] == TransferState::Done)
        .count();
    assert_eq!(done, 1, "one writer for the shared target: {states:?}");
    assert!(
        same.iter()
            .all(|id| matches!(states[id], TransferState::Done | TransferState::Skipped(_))),
        "{states:?}"
    );
    engine.shutdown().await;
}
