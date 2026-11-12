//! Engine throughput baselines: `MemFs` to `MemFs` with 4 workers, with
//! the queue in memory and (for many small files, where the queue writes
//! dominate) in a file on disk.

use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use hoprs_core::{RemotePath, SiteId};
use hoprs_remote::MemFs;
use hoprs_transfer::{
    Direction, EngineConfig, Event, QueueStore, SiteEndpoints, TransferEngine, TransferSpec,
    TransferState,
};
use tokio::runtime::Runtime;
use tokio::sync::broadcast::error::RecvError;

const WORKERS: usize = 4;
const BIG_FILE: usize = 256 * 1024 * 1024;
const SMALL_FILES: usize = 10_000;
const SMALL_FILE_LEN: usize = 4 * 1024;
/// Upper bound for one run, so that a hung engine fails the bench run.
const RUN_TIMEOUT: Duration = Duration::from_secs(120);

/// Where a run keeps its queue database.
#[derive(Clone, Copy)]
enum Queue {
    Memory,
    /// A fresh file under Cargo's target tmp dir, which (unlike `/tmp` on
    /// many systems) lives on a real disk, so that `fsync` costs are real.
    Disk,
}

/// Opens the queue for one run. The returned guard keeps a `Disk` queue's
/// directory alive until the run ends.
fn open_queue(queue: Queue) -> (QueueStore, Option<tempfile::TempDir>) {
    match queue {
        Queue::Memory => (QueueStore::open_in_memory().expect("in-memory store"), None),
        Queue::Disk => {
            let dir = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).expect("bench tmp dir");
            let store = QueueStore::open(&dir.path().join("queue.db")).expect("on-disk store");
            (store, Some(dir))
        }
    }
}

fn runtime() -> Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(WORKERS)
        .enable_all()
        .build()
        .expect("tokio runtime")
}

/// One timed run: fresh store and destination, enqueue `src` and wait for Done.
async fn run_once(
    source: &MemFs,
    src: &str,
    dst: &str,
    dst_parent: &str,
    queue: Queue,
) -> Duration {
    let dest = MemFs::new();
    dest.add_dir(dst_parent);
    let site = SiteId("bench".to_string());

    let start = Instant::now();
    let (store, _dir) = open_queue(queue);
    let engine = TransferEngine::start(store, EngineConfig::default());
    let mut rx = engine.subscribe();
    engine
        .register_site(
            site.clone(),
            SiteEndpoints {
                local: Arc::new(source.clone()),
                remote: Arc::new(dest),
                workers: WORKERS,
            },
        )
        .await;
    let id = engine
        .enqueue(TransferSpec {
            site,
            direction: Direction::Upload,
            local: RemotePath::new(src).expect("path"),
            remote: RemotePath::new(dst).expect("path"),
        })
        .await
        .expect("enqueue");
    let done = async {
        loop {
            match rx.recv().await {
                Ok(Event::StateChanged { id: i, state }) if i == id && state.is_terminal() => {
                    assert_eq!(state, TransferState::Done);
                    break;
                }
                Ok(_) | Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => panic!("event channel closed"),
            }
        }
    };
    tokio::time::timeout(RUN_TIMEOUT, done)
        .await
        .expect("bench run timed out");
    let elapsed = start.elapsed();
    engine.shutdown().await;
    elapsed
}

fn bench_engine(c: &mut Criterion) {
    let rt = runtime();
    let mtime = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);

    let mut group = c.benchmark_group("engine");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(10));

    let big = MemFs::new();
    big.add_file("/src/big.bin", &vec![0xA5; BIG_FILE], mtime);
    group.throughput(Throughput::Bytes(BIG_FILE as u64));
    group.bench_function("upload_256mib_single_file", |b| {
        b.iter_custom(|iters| {
            rt.block_on(async {
                let mut total = Duration::ZERO;
                for _ in 0..iters {
                    total +=
                        run_once(&big, "/src/big.bin", "/dst/big.bin", "/dst", Queue::Memory).await;
                }
                total
            })
        });
    });
    drop(big);

    let small = MemFs::new();
    let payload = vec![0x5A; SMALL_FILE_LEN];
    small.add_dir("/src/tree");
    for i in 0..SMALL_FILES {
        small.add_file(&format!("/src/tree/f{i:05}.bin"), &payload, mtime);
    }
    group.throughput(Throughput::Elements(SMALL_FILES as u64));
    for (name, queue) in [
        ("upload_10k_small_files", Queue::Memory),
        ("upload_10k_small_files_on_disk_queue", Queue::Disk),
    ] {
        group.bench_function(name, |b| {
            b.iter_custom(|iters| {
                rt.block_on(async {
                    let mut total = Duration::ZERO;
                    for _ in 0..iters {
                        total += run_once(&small, "/src/tree", "/dst/tree", "/dst", queue).await;
                    }
                    total
                })
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_engine);
criterion_main!(benches);
