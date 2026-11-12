//! Engine throughput baselines: `MemFs` to `MemFs` with 4 workers.

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

fn runtime() -> Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(WORKERS)
        .enable_all()
        .build()
        .expect("tokio runtime")
}

/// One timed run: fresh store and destination, enqueue `src` and wait for Done.
async fn run_once(source: &MemFs, src: &str, dst: &str, dst_parent: &str) -> Duration {
    let dest = MemFs::new();
    dest.add_dir(dst_parent);
    let site = SiteId("bench".to_string());

    let start = Instant::now();
    let store = QueueStore::open_in_memory().expect("in-memory store");
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
    start.elapsed()
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
                    total += run_once(&big, "/src/big.bin", "/dst/big.bin", "/dst").await;
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
    group.bench_function("upload_10k_small_files", |b| {
        b.iter_custom(|iters| {
            rt.block_on(async {
                let mut total = Duration::ZERO;
                for _ in 0..iters {
                    total += run_once(&small, "/src/tree", "/dst/tree", "/dst").await;
                }
                total
            })
        });
    });
    group.finish();
}

criterion_group!(benches, bench_engine);
criterion_main!(benches);
