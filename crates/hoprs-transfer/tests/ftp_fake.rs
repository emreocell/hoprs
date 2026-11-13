//! The engine with `FtpFs` on a fake in-memory FTP server (no Docker) whose transfer
//! connections die as soon as they go idle: reusing one without a check
//! (`POOL_FRESH`) must cost a retry on a new connection, never a paused site.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, SystemTime};

use hoprs_core::{RemotePath, SiteId};
use hoprs_remote::testing::fake_transfer_server;
use hoprs_remote::{Connection, FtpConfig, FtpFs, FtpSecurity, MemFs, ScriptedFtpPrompter};
use hoprs_transfer::{
    Direction, EngineConfig, EventKind, QueueStore, SiteEndpoints, TransferEngine, TransferSpec,
    TransferState,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dead_pooled_connection_is_replaced_without_pausing_the_site() {
    let server = fake_transfer_server().await;
    let mut cfg = FtpConfig::new("127.0.0.1", server.port, "frank", FtpSecurity::Plain);
    cfg.keepalive_interval = Duration::from_secs(60);
    cfg.command_timeout = Duration::from_secs(5);
    let prompter = Arc::new(ScriptedFtpPrompter::new().with_password(Some("pw")));
    let fs = FtpFs::connect(cfg, prompter).await.unwrap();

    let local = MemFs::new();
    let files: Vec<(String, Vec<u8>)> = (0..4)
        .map(|i| (format!("f{i}.bin"), vec![i as u8; 1000 + i]))
        .collect();
    for (name, bytes) in &files {
        local.add_file(&format!("/w/{name}"), bytes, SystemTime::UNIX_EPOCH);
    }
    let engine = TransferEngine::start(
        QueueStore::open_in_memory().unwrap(),
        EngineConfig::default(),
    )
    .unwrap();
    let mut rx = engine.subscribe();
    let site = SiteId("fake".to_owned());
    engine
        .register_site(
            site.clone(),
            SiteEndpoints {
                local: Arc::new(local),
                remote: Arc::new(fs.clone()),
                workers: 1,
            },
        )
        .await;
    // One after the other: each reuses the connection the previous one left in the pool.
    for (name, _) in &files {
        let id = engine
            .enqueue(TransferSpec {
                site: site.clone(),
                direction: Direction::Upload,
                local: RemotePath::new(&format!("/w/{name}")).unwrap(),
                remote: RemotePath::new(&format!("/{name}")).unwrap(),
            })
            .await
            .unwrap();
        let state = tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                match rx.recv().await.unwrap().kind {
                    EventKind::SitePaused { error, .. } => panic!("site paused: {error:?}"),
                    EventKind::StateChanged { id: i, state } if i == id && state.is_terminal() => {
                        return state;
                    }
                    _ => {}
                }
            }
        })
        .await
        .expect("the upload finishes");
        assert_eq!(state, TransferState::Done, "{name}");
    }
    let stored = server.files.lock().unwrap().clone();
    for (name, bytes) in &files {
        assert_eq!(stored.get(&format!("/{name}")), Some(bytes), "{name}");
    }
    // The pooled connections did die idle (and were replaced), the session never did.
    assert!(server.closed.load(Ordering::SeqCst) >= files.len());
    assert_eq!(Connection::generation(&fs), 1);
    engine.shutdown().await;
    fs.disconnect().await;
}
