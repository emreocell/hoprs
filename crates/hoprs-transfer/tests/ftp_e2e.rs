//! The transfer engine with `LocalFs` (a `TempDir`) on one side and `FtpFs` (explicit FTPS,
//! trusting the test CA) on the `ftp` container of `docker/compose.yml` on the other, 4
//! workers per site.
//!
//! Every test works in its own remote directory, removed (best effort) when the test ends.

use std::collections::HashSet;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, ready};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hoprs_core::{ConnectionState, EntryKind, ErrorKind, RemoteEntry, RemotePath, Result, SiteId};
use hoprs_remote::testing::{ftp_config, ftp_listings, ftps_fixture, kill_ftp_sessions};
use hoprs_remote::{
    BoxAsyncRead, BoxAsyncWrite, Connection, FtpFs, FtpPrompter, FtpSecurity, LocalFs, RemoteFs,
    ScriptedFtpPrompter,
};
use hoprs_ssh::testing::docker;
use hoprs_transfer::{
    ConflictPolicy, Direction, EngineConfig, Event, EventKind, QueueStore, SiteEndpoints,
    TransferEngine, TransferId, TransferSpec, TransferState, spawn_site_watcher,
};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::sync::Notify;
use tokio::sync::broadcast::Receiver;
use tokio::sync::broadcast::error::RecvError;

const MIB: usize = 1 << 20;
const KIB: usize = 1 << 10;
const WORKERS: usize = 4;

fn p(s: &str) -> RemotePath {
    RemotePath::new(s).unwrap()
}

fn site() -> SiteId {
    SiteId("ftp".to_string())
}

fn mtime() -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(1_700_000_000)
}

/// Deterministic pseudo-random bytes (xorshift64).
fn data(len: usize, seed: u64) -> Vec<u8> {
    let mut x = seed | 1;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect()
}

/// A name no other test uses: `<nanos>-<counter>`.
fn unique() -> String {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    format!("{nanos}-{n}")
}

/// Removes a directory in the `ftp` container when dropped. Never panics.
struct Cleanup(String);

impl Drop for Cleanup {
    fn drop(&mut self) {
        let dir = self.0.clone();
        if std::panic::catch_unwind(|| docker(&["exec", "-T", "ftp", "rm", "-rf", &dir])).is_err() {
            eprintln!("could not remove {dir}");
        }
    }
}

/// An explicit FTPS connection as frank (the test CA is trusted, so nothing is prompted) and
/// a fresh remote work directory.
struct Remote {
    fs: FtpFs,
    work: RemotePath,
    _cleanup: Cleanup,
}

impl Remote {
    fn at(&self, rel: &str) -> RemotePath {
        p(&format!("{}/{rel}", self.work))
    }

    async fn read(&self, path: &RemotePath) -> Vec<u8> {
        let mut out = Vec::new();
        self.fs
            .open_read(path, 0)
            .await
            .unwrap()
            .read_to_end(&mut out)
            .await
            .unwrap();
        out
    }

    async fn write(&self, path: &RemotePath, bytes: &[u8]) {
        let mut w = self.fs.open_write(path, 0).await.unwrap();
        w.write_all(bytes).await.unwrap();
        w.shutdown().await.unwrap();
    }

    /// Runs `cmd` as frank inside the work directory.
    fn sh(&self, cmd: &str) {
        let script = format!("cd {} && {cmd}", self.work);
        docker(&["exec", "-T", "ftp", "su", "frank", "-c", &script]);
    }
}

/// Connects and creates `<parent>/t-<unique>` as the work directory.
async fn connect_in(parent: &str) -> Remote {
    let mut cfg = ftp_config(FtpSecurity::ExplicitTls);
    cfg.extra_roots = vec![ftps_fixture("ca.pem")];
    cfg.transfer_connections = WORKERS;
    let prompter = Arc::new(ScriptedFtpPrompter::new());
    let fs = FtpFs::connect(cfg, prompter as Arc<dyn FtpPrompter>)
        .await
        .unwrap();
    let work = format!("{parent}/t-{}", unique());
    let cleanup = Cleanup(work.clone());
    fs.mkdir(&p(&work)).await.unwrap();
    Remote {
        fs,
        work: p(&work),
        _cleanup: cleanup,
    }
}

/// Starts an engine (in-memory queue) with site `"ftp"`: `LocalFs` over `local_root` and
/// `remote`.
async fn start_engine_with(
    config: EngineConfig,
    local_root: &Path,
    remote: Arc<dyn RemoteFs>,
) -> (TransferEngine, Receiver<Event>) {
    let engine = TransferEngine::start(QueueStore::open_in_memory().unwrap(), config).unwrap();
    let rx = engine.subscribe();
    engine
        .register_site(
            site(),
            SiteEndpoints {
                local: Arc::new(LocalFs::new(local_root)),
                remote,
                workers: WORKERS,
            },
        )
        .await;
    (engine, rx)
}

async fn start_engine(
    local_root: &Path,
    remote: Arc<dyn RemoteFs>,
) -> (TransferEngine, Receiver<Event>) {
    start_engine_with(EngineConfig::default(), local_root, remote).await
}

fn transfer(direction: Direction, local: &str, remote: &RemotePath) -> TransferSpec {
    TransferSpec {
        site: site(),
        direction,
        local: p(local),
        remote: remote.clone(),
    }
}

/// Receives events until `id` reaches a terminal state; returns every event received and
/// that state. Panics after `timeout`.
async fn run_until_terminal(
    rx: &mut Receiver<Event>,
    id: TransferId,
    timeout: Duration,
) -> (Vec<Event>, TransferState) {
    let mut seen = Vec::new();
    let wait = async {
        loop {
            match rx.recv().await {
                Ok(ev) => {
                    let terminal = match &ev.kind {
                        EventKind::StateChanged { id: i, state }
                            if *i == id && state.is_terminal() =>
                        {
                            Some(state.clone())
                        }
                        _ => None,
                    };
                    seen.push(ev);
                    if let Some(state) = terminal {
                        return state;
                    }
                }
                Err(RecvError::Lagged(n)) => panic!("lagged by {n} events"),
                Err(RecvError::Closed) => panic!("event channel closed"),
            }
        }
    };
    match tokio::time::timeout(timeout, wait).await {
        Ok(state) => (seen, state),
        Err(_) => panic!("{id:?} not finished after {timeout:?}; events: {seen:?}"),
    }
}

fn site_paused_count(events: &[Event]) -> usize {
    events
        .iter()
        .filter(|e| matches!(&e.kind, EventKind::SitePaused { .. }))
        .count()
}

fn failed(events: &[Event]) -> Vec<&Event> {
    events
        .iter()
        .filter(|e| {
            matches!(
                &e.kind,
                EventKind::StateChanged {
                    state: TransferState::Failed(_),
                    ..
                }
            )
        })
        .collect()
}

/// Holds every stream that has passed `after` bytes until `release` is called, so that a
/// test can act while a copy is known to be unfinished.
struct Gate {
    after: u64,
    released: AtomicBool,
    notify: Notify,
    /// A stream is waiting at the gate.
    holding: AtomicBool,
    held: Notify,
}

impl Gate {
    fn new(after: u64) -> Arc<Self> {
        Arc::new(Gate {
            after,
            released: AtomicBool::new(false),
            notify: Notify::new(),
            holding: AtomicBool::new(false),
            held: Notify::new(),
        })
    }

    /// Returns once a stream is waiting at the gate. Panics after 30 s.
    async fn held(&self) {
        let wait = async {
            loop {
                let notified = self.held.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                if self.holding.load(Ordering::SeqCst) {
                    return;
                }
                notified.await;
            }
        };
        tokio::time::timeout(Duration::from_secs(30), wait)
            .await
            .expect("no stream reached the gate");
    }

    /// Lets every held and future stream through.
    fn release(&self) {
        self.released.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    fn holds(&self, passed: u64) -> bool {
        passed >= self.after && !self.released.load(Ordering::SeqCst)
    }

    async fn wait(self: Arc<Self>) {
        loop {
            let notified = self.notify.notified();
            tokio::pin!(notified);
            // Registered before the check, so a `release` in between is not missed.
            notified.as_mut().enable();
            if self.released.load(Ordering::SeqCst) {
                return;
            }
            notified.await;
        }
    }
}

type Waiting = Option<Pin<Box<dyn Future<Output = ()> + Send>>>;

/// Polls `waiting` (created from `gate`) while `gate` holds a stream that passed `passed`
/// bytes.
fn poll_gate(
    gate: &Arc<Gate>,
    waiting: &mut Waiting,
    passed: u64,
    cx: &mut Context<'_>,
) -> Poll<()> {
    if gate.holds(passed) {
        if !gate.holding.swap(true, Ordering::SeqCst) {
            gate.held.notify_waiters();
        }
        let gate = Arc::clone(gate);
        let w = waiting.get_or_insert_with(|| Box::pin(gate.wait()));
        ready!(w.as_mut().poll(cx));
        *waiting = None;
    }
    Poll::Ready(())
}

/// A write stream that stops at its `Gate`.
struct GatedWriter {
    inner: BoxAsyncWrite,
    gate: Arc<Gate>,
    written: u64,
    waiting: Waiting,
}

impl AsyncWrite for GatedWriter {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let this = self.get_mut();
        ready!(poll_gate(&this.gate, &mut this.waiting, this.written, cx));
        let n = ready!(Pin::new(&mut this.inner).poll_write(cx, buf))?;
        this.written += n as u64;
        Poll::Ready(Ok(n))
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

/// A read stream that stops at its `Gate`.
struct GatedReader {
    inner: BoxAsyncRead,
    gate: Arc<Gate>,
    read: u64,
    waiting: Waiting,
}

impl AsyncRead for GatedReader {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let this = self.get_mut();
        ready!(poll_gate(&this.gate, &mut this.waiting, this.read, cx));
        let before = buf.filled().len();
        ready!(Pin::new(&mut this.inner).poll_read(cx, buf))?;
        this.read += (buf.filled().len() - before) as u64;
        Poll::Ready(Ok(()))
    }
}

/// `RemoteFs` that forwards to an `FtpFs`, records the offset of every `open_read` and
/// `open_write`, and holds the streams of the gated direction at their gate.
struct RecordingFs {
    inner: FtpFs,
    read_offsets: Arc<Mutex<Vec<u64>>>,
    write_offsets: Arc<Mutex<Vec<u64>>>,
    read_gate: Option<Arc<Gate>>,
    write_gate: Option<Arc<Gate>>,
}

impl RecordingFs {
    fn new(inner: FtpFs) -> Self {
        RecordingFs {
            inner,
            read_offsets: Arc::default(),
            write_offsets: Arc::default(),
            read_gate: None,
            write_gate: None,
        }
    }
}

#[async_trait::async_trait]
impl RemoteFs for RecordingFs {
    async fn list(&self, path: &RemotePath) -> Result<Vec<RemoteEntry>> {
        self.inner.list(path).await
    }
    async fn stat(&self, path: &RemotePath) -> Result<RemoteEntry> {
        self.inner.stat(path).await
    }
    async fn open_read(&self, path: &RemotePath, offset: u64) -> Result<BoxAsyncRead> {
        self.read_offsets.lock().unwrap().push(offset);
        let inner = self.inner.open_read(path, offset).await?;
        Ok(match &self.read_gate {
            Some(gate) => Box::new(GatedReader {
                inner,
                gate: Arc::clone(gate),
                read: 0,
                waiting: None,
            }),
            None => inner,
        })
    }
    async fn open_write(&self, path: &RemotePath, offset: u64) -> Result<BoxAsyncWrite> {
        self.write_offsets.lock().unwrap().push(offset);
        let inner = self.inner.open_write(path, offset).await?;
        Ok(match &self.write_gate {
            Some(gate) => Box::new(GatedWriter {
                inner,
                gate: Arc::clone(gate),
                written: 0,
                waiting: None,
            }),
            None => inner,
        })
    }
    async fn mkdir(&self, path: &RemotePath) -> Result<()> {
        self.inner.mkdir(path).await
    }
    async fn rename(&self, from: &RemotePath, to: &RemotePath) -> Result<()> {
        self.inner.rename(from, to).await
    }
    async fn remove_file(&self, path: &RemotePath) -> Result<()> {
        self.inner.remove_file(path).await
    }
    async fn remove_dir(&self, path: &RemotePath) -> Result<()> {
        self.inner.remove_dir(path).await
    }
    async fn chmod(&self, path: &RemotePath, mode: u32) -> Result<()> {
        self.inner.chmod(path, mode).await
    }
    async fn set_modified(&self, path: &RemotePath, mtime: SystemTime) -> Result<()> {
        self.inner.set_modified(path, mtime).await
    }
}

/// Files of the Plan 1 tree, relative to its root: (path, contents). `img/` is an empty
/// directory next to them.
fn site_files() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("index.html", b"<html></html>".to_vec()),
        (".env", b"SECRET=1".to_vec()),
        ("relatório final.pdf", data(70_000, 1)),
        ("css/app.css", b"body { margin: 0 }".to_vec()),
    ]
}

fn write_local(root: &Path, rel: &str, bytes: &[u8]) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, bytes).unwrap();
    let file = std::fs::File::options().write(true).open(&path).unwrap();
    file.set_modified(mtime()).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d --build --wait"]
async fn uploads_and_downloads_tree() {
    let remote = connect_in("/home/frank").await;
    let local = TempDir::new().unwrap();
    for (rel, bytes) in site_files() {
        write_local(local.path(), &format!("site/{rel}"), &bytes);
    }
    std::fs::create_dir_all(local.path().join("site/img")).unwrap();
    let (engine, mut rx) = start_engine(local.path(), Arc::new(remote.fs.clone())).await;

    let up = engine
        .enqueue(transfer(Direction::Upload, "/site", &remote.at("site")))
        .await
        .unwrap();
    let (events, state) = run_until_terminal(&mut rx, up, Duration::from_secs(60)).await;
    assert_eq!(state, TransferState::Done, "{:?}", failed(&events));
    for (rel, bytes) in site_files() {
        let got = remote.read(&remote.at(&format!("site/{rel}"))).await;
        assert!(got == bytes, "{rel}: {} bytes", got.len());
    }
    let img = remote.fs.stat(&remote.at("site/img")).await.unwrap();
    assert_eq!(img.kind, EntryKind::Dir);
    assert!(
        remote
            .fs
            .list(&remote.at("site/img"))
            .await
            .unwrap()
            .is_empty()
    );

    std::fs::create_dir(local.path().join("down")).unwrap();
    let down = engine
        .enqueue(transfer(
            Direction::Download,
            "/down/site",
            &remote.at("site"),
        ))
        .await
        .unwrap();
    let (events, state) = run_until_terminal(&mut rx, down, Duration::from_secs(60)).await;
    assert_eq!(state, TransferState::Done, "{:?}", failed(&events));
    for (rel, bytes) in site_files() {
        let got = std::fs::read(local.path().join("down/site").join(rel)).unwrap();
        assert!(got == bytes, "{rel}: {} bytes", got.len());
    }
    let img = local.path().join("down/site/img");
    assert!(img.is_dir());
    assert_eq!(std::fs::read_dir(img).unwrap().count(), 0);
    let mut names: Vec<String> = std::fs::read_dir(local.path().join("down/site"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(
        names,
        [".env", "css", "img", "index.html", "relatório final.pdf"]
    );
}

/// Spec §4.3, Review Focus 3: a dropped connection pauses the site (it does not fail the
/// item); once the session reconnects the upload continues where it stopped (`APPE`).
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d --build --wait"]
async fn network_drop_pauses_and_resumes() {
    let remote = connect_in("/home/frank").await;
    let local = TempDir::new().unwrap();
    let bytes = data(64 * MIB, 64);
    write_local(local.path(), "big.bin", &bytes);
    let source_hash = Sha256::digest(&bytes);
    drop(bytes);

    // The upload cannot finish before the connections are killed: it stops after 8 MiB
    // until then.
    let gate = Gate::new(8 * MIB as u64);
    let mut recording = RecordingFs::new(remote.fs.clone());
    recording.write_gate = Some(Arc::clone(&gate));
    let write_offsets = Arc::clone(&recording.write_offsets);
    let (engine, mut rx) = start_engine(local.path(), Arc::new(recording)).await;
    let watcher = spawn_site_watcher(engine.clone(), site(), Arc::new(remote.fs.clone()));
    assert_eq!(Connection::generation(&remote.fs), 1);

    let target = remote.at("big.bin");
    let id = engine
        .enqueue(transfer(Direction::Upload, "/big.bin", &target))
        .await
        .unwrap();
    // Killed only once the copy is held at 8 MiB, so the destination has bytes to continue.
    gate.held().await;
    kill_ftp_sessions();
    // The held stream now writes on the dead connection and fails.
    gate.release();

    let (events, state) = run_until_terminal(&mut rx, id, Duration::from_secs(120)).await;
    watcher.abort();
    assert_eq!(state, TransferState::Done, "{events:?}");
    assert_eq!(site_paused_count(&events), 1, "{events:?}");
    assert!(failed(&events).is_empty(), "{:?}", failed(&events));
    assert_eq!(Connection::generation(&remote.fs), 2);
    let offsets = write_offsets.lock().unwrap().clone();
    assert!(
        offsets.first() == Some(&0) && offsets.iter().any(|&o| o > 0),
        "the upload continued from its partial destination: {offsets:?}"
    );

    let remote_hash = Sha256::digest(remote.read(&target).await);
    assert_eq!(remote_hash, source_hash);
}

/// The download twin of `network_drop_pauses_and_resumes`: it continues with `REST`.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d --build --wait"]
async fn download_drop_resumes() {
    let remote = connect_in("/home/frank").await;
    let local = TempDir::new().unwrap();
    let source = remote.at("big.bin");
    let bytes = data(64 * MIB, 65);
    remote.write(&source, &bytes).await;
    let source_hash = Sha256::digest(&bytes);
    drop(bytes);

    // The download cannot finish before the connections are killed: it stops after 8 MiB
    // until then.
    let gate = Gate::new(8 * MIB as u64);
    let mut recording = RecordingFs::new(remote.fs.clone());
    recording.read_gate = Some(Arc::clone(&gate));
    let read_offsets = Arc::clone(&recording.read_offsets);
    let (engine, mut rx) = start_engine(local.path(), Arc::new(recording)).await;
    let watcher = spawn_site_watcher(engine.clone(), site(), Arc::new(remote.fs.clone()));
    assert_eq!(Connection::generation(&remote.fs), 1);

    let id = engine
        .enqueue(transfer(Direction::Download, "/big.bin", &source))
        .await
        .unwrap();
    // Killed only once the copy is held at 8 MiB, so the destination has bytes to continue.
    gate.held().await;
    kill_ftp_sessions();
    // The held stream now reads from the dead connection and fails.
    gate.release();

    let (events, state) = run_until_terminal(&mut rx, id, Duration::from_secs(120)).await;
    watcher.abort();
    assert_eq!(state, TransferState::Done, "{events:?}");
    assert_eq!(site_paused_count(&events), 1, "{events:?}");
    assert!(failed(&events).is_empty(), "{:?}", failed(&events));
    assert_eq!(Connection::generation(&remote.fs), 2);
    let offsets = read_offsets.lock().unwrap().clone();
    assert!(
        offsets.first() == Some(&0) && offsets.iter().any(|&o| o > 0),
        "the download continued from its partial destination: {offsets:?}"
    );

    let local_hash = Sha256::digest(std::fs::read(local.path().join("big.bin")).unwrap());
    assert_eq!(local_hash, source_hash);
}

/// Review Focus 5: a full disk on the server fails the item that does not fit (`Io`),
/// without pausing the site; the site's other items still run.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d --build --wait"]
async fn disk_full_fails_item_not_site() {
    // `/home/frank/small` is a 1 MiB tmpfs: two of these files fit, the third does not.
    let remote = connect_in("/home/frank/small").await;
    let local = TempDir::new().unwrap();
    let files: Vec<Vec<u8>> = (0..3).map(|i| data(400 * KIB, 100 + i)).collect();
    for (i, bytes) in files.iter().enumerate() {
        write_local(local.path(), &format!("f{i}.bin"), bytes);
    }
    let (engine, mut rx) = start_engine(local.path(), Arc::new(remote.fs.clone())).await;

    // One at a time, so which file does not fit is known.
    let mut events = Vec::new();
    let mut states = Vec::new();
    for i in 0..3 {
        let id = engine
            .enqueue(transfer(
                Direction::Upload,
                &format!("/f{i}.bin"),
                &remote.at(&format!("f{i}.bin")),
            ))
            .await
            .unwrap();
        let (seen, state) = run_until_terminal(&mut rx, id, Duration::from_secs(60)).await;
        events.extend(seen);
        states.push(state);
    }
    assert_eq!(states[0], TransferState::Done);
    assert_eq!(states[1], TransferState::Done);
    match &states[2] {
        TransferState::Failed(e) => assert_eq!(e.kind, ErrorKind::Io, "{e:?}"),
        other => panic!("the third file ended {other:?}"),
    }
    for (i, bytes) in files.iter().take(2).enumerate() {
        assert!(remote.read(&remote.at(&format!("f{i}.bin"))).await == *bytes);
    }
    assert_eq!(site_paused_count(&events), 0, "{events:?}");
    assert_eq!(
        *Connection::state(&remote.fs).borrow(),
        ConnectionState::Connected
    );
}

/// Ruling R9 (spec §9, many small files): the engine's `stat` of a file it has just copied,
/// or is about to overwrite, does not list the directory, even a 2k-entry one; since ruling
/// R11 neither does the check that a new file's destination is missing.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d --build --wait"]
async fn copies_into_big_dir_do_not_list_per_file() {
    const FILES: u64 = 20;
    let remote = connect_in("/home/frank").await;
    remote.sh("seq -f 'e%g' 2000 | xargs touch");
    let local = TempDir::new().unwrap();
    for i in 0..FILES {
        write_local(local.path(), &format!("u{i}"), &data(1000, 500 + i));
    }
    let config = EngineConfig {
        default_policy: ConflictPolicy::Overwrite,
        ..EngineConfig::default()
    };
    let (engine, mut rx) =
        start_engine_with(config, local.path(), Arc::new(remote.fs.clone())).await;

    // Uploads every file and returns how many listings that took.
    let upload_all = async |rx: &mut Receiver<Event>| {
        let before = ftp_listings(&remote.fs);
        let mut ids = Vec::new();
        for i in 0..FILES {
            let spec = transfer(
                Direction::Upload,
                &format!("/u{i}"),
                &remote.at(&format!("u{i}")),
            );
            ids.push(engine.enqueue(spec).await.unwrap());
        }
        let mut pending: HashSet<TransferId> = ids.into_iter().collect();
        let all_done = async {
            while !pending.is_empty() {
                match rx.recv().await.unwrap().kind {
                    EventKind::StateChanged { id, state } if state.is_terminal() => {
                        assert_eq!(state, TransferState::Done, "{id:?}");
                        pending.remove(&id);
                    }
                    _ => {}
                }
            }
        };
        tokio::time::timeout(Duration::from_secs(60), all_done)
            .await
            .expect("uploads not finished");
        ftp_listings(&remote.fs) - before
    };
    // New files: none either (`SIZE` `550` + an empty `STAT` reply say it is missing).
    assert_eq!(upload_all(&mut rx).await, 0);
    // Existing files: none.
    assert_eq!(upload_all(&mut rx).await, 0);
    for i in 0..FILES {
        assert!(remote.read(&remote.at(&format!("u{i}"))).await == data(1000, 500 + i));
    }
}

/// Ruling R10: `stat` never follows a symlink, so the engine refuses to upload onto a remote
/// symlink (it would write through it) and leaves the link's target untouched.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d --build --wait"]
async fn upload_onto_symlink_fails() {
    let remote = connect_in("/home/frank").await;
    remote.sh("printf original > real && ln -s real link");
    let local = TempDir::new().unwrap();
    write_local(local.path(), "f", b"replacement");
    let config = EngineConfig {
        default_policy: ConflictPolicy::Overwrite,
        ..EngineConfig::default()
    };
    let (engine, mut rx) =
        start_engine_with(config, local.path(), Arc::new(remote.fs.clone())).await;
    let id = engine
        .enqueue(transfer(Direction::Upload, "/f", &remote.at("link")))
        .await
        .unwrap();
    let (_, state) = run_until_terminal(&mut rx, id, Duration::from_secs(60)).await;
    match &state {
        TransferState::Failed(e) => {
            assert_eq!(e.message, "Destination is not a regular file", "{e:?}")
        }
        other => panic!("the upload ended {other:?}"),
    }
    assert_eq!(remote.read(&remote.at("real")).await, b"original");
}

/// Ruling R10: a single-file download of a remote symlink is skipped, as on every backend.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d --build --wait"]
async fn download_of_symlink_is_skipped() {
    let remote = connect_in("/home/frank").await;
    remote.sh("printf data > real && ln -s real link");
    let local = TempDir::new().unwrap();
    let (engine, mut rx) = start_engine(local.path(), Arc::new(remote.fs.clone())).await;
    let id = engine
        .enqueue(transfer(Direction::Download, "/link", &remote.at("link")))
        .await
        .unwrap();
    let (_, state) = run_until_terminal(&mut rx, id, Duration::from_secs(60)).await;
    assert!(matches!(state, TransferState::Skipped(_)), "{state:?}");
    assert!(!local.path().join("link").exists());
}
