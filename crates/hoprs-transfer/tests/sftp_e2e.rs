//! The transfer engine with `LocalFs` (a `TempDir`) on one side and `SftpFs` on the
//! container in `docker/compose.yml` on the other, 4 workers per site.
//!
//! Every test works in its own remote directory, removed (best effort) when the test ends.

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, ready};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hoprs_core::{EntryKind, ErrorKind, RemoteEntry, RemotePath, Result, SiteId};
use hoprs_remote::{BoxAsyncRead, BoxAsyncWrite, LocalFs, RemoteFs, SftpConfig, SftpFs};
use hoprs_ssh::testing::{docker, fixture, kill_ssh_sessions, test_config};
use hoprs_ssh::{KeySource, Prompter, ScriptedPrompter, SessionState, SshSession};
use hoprs_transfer::{
    Direction, EngineConfig, Event, EventKind, QueueStore, SiteEndpoints, TransferEngine,
    TransferId, TransferSpec, TransferState, spawn_site_watcher,
};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWrite};
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
    SiteId("sftp".to_string())
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

/// Removes a remote directory when dropped. Never panics.
struct Cleanup(String);

impl Drop for Cleanup {
    fn drop(&mut self) {
        let dir = self.0.clone();
        if std::panic::catch_unwind(|| docker(&["exec", "-T", "sshd", "rm", "-rf", &dir])).is_err()
        {
            eprintln!("could not remove {dir}");
        }
    }
}

/// An SFTP connection as alice (key auth) and a fresh remote work directory.
struct Remote {
    fs: SftpFs,
    session: SshSession,
    work: RemotePath,
    _cleanup: Cleanup,
    _known_hosts: TempDir,
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
}

/// Connects and creates `<parent>/t-<unique>` as the work directory.
async fn connect_in(parent: &str) -> Remote {
    let known_hosts = TempDir::new().unwrap();
    let mut cfg = test_config("alice", known_hosts.path());
    cfg.auth.key = KeySource::File {
        path: fixture("id_ed25519"),
        passphrase: None,
    };
    let prompter = Arc::new(ScriptedPrompter::new().accept_host(true));
    let session = SshSession::connect(cfg, prompter as Arc<dyn Prompter>)
        .await
        .unwrap();
    let fs = SftpFs::new(session.clone(), SftpConfig::default())
        .await
        .unwrap();
    let work = format!("{parent}/t-{}", unique());
    let cleanup = Cleanup(work.clone());
    fs.mkdir(&p(&work)).await.unwrap();
    Remote {
        fs,
        session,
        work: p(&work),
        _cleanup: cleanup,
        _known_hosts: known_hosts,
    }
}

/// Starts an engine (in-memory queue) with site `"sftp"`: `LocalFs` over `local_root`
/// and `remote`.
async fn start_engine(
    local_root: &Path,
    remote: Arc<dyn RemoteFs>,
) -> (TransferEngine, Receiver<Event>) {
    let engine = TransferEngine::start(
        QueueStore::open_in_memory().unwrap(),
        EngineConfig::default(),
    )
    .unwrap();
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

fn transfer(direction: Direction, local: &str, remote: &RemotePath) -> TransferSpec {
    TransferSpec {
        site: site(),
        direction,
        local: p(local),
        remote: remote.clone(),
    }
}

/// Receives events until `id` reaches a terminal state; returns every event
/// received and that state. Panics after `timeout`.
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

/// Holds every write stream that has passed `after` bytes until `release` is called, so
/// that a test can act while a copy is known to be unfinished.
struct WriteGate {
    after: u64,
    released: AtomicBool,
    notify: Notify,
}

impl WriteGate {
    fn new(after: u64) -> Arc<Self> {
        Arc::new(WriteGate {
            after,
            released: AtomicBool::new(false),
            notify: Notify::new(),
        })
    }

    /// Lets every held and future write through.
    fn release(&self) {
        self.released.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
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

/// A write stream that stops at its `WriteGate`.
struct GatedWriter {
    inner: BoxAsyncWrite,
    gate: Arc<WriteGate>,
    written: u64,
    waiting: Option<Pin<Box<dyn Future<Output = ()> + Send>>>,
}

impl AsyncWrite for GatedWriter {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let this = self.get_mut();
        if this.written >= this.gate.after && !this.gate.released.load(Ordering::SeqCst) {
            let gate = Arc::clone(&this.gate);
            let waiting = this.waiting.get_or_insert_with(|| Box::pin(gate.wait()));
            ready!(waiting.as_mut().poll(cx));
            this.waiting = None;
        }
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

/// `RemoteFs` that forwards to an `SftpFs`, records the offset of every `open_write`
/// and holds its write streams at `gate`.
struct RecordingFs {
    inner: SftpFs,
    write_offsets: Arc<Mutex<Vec<u64>>>,
    gate: Arc<WriteGate>,
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
        self.inner.open_read(path, offset).await
    }
    async fn open_write(&self, path: &RemotePath, offset: u64) -> Result<BoxAsyncWrite> {
        self.write_offsets.lock().unwrap().push(offset);
        let inner = self.inner.open_write(path, offset).await?;
        Ok(Box::new(GatedWriter {
            inner,
            gate: Arc::clone(&self.gate),
            written: 0,
            waiting: None,
        }))
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
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn uploads_and_downloads_tree() {
    let remote = connect_in("/home/alice").await;
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

/// Spec §4.3: a dropped connection pauses the site (it does not fail the item); once the
/// session reconnects the copy continues from where it stopped.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn network_drop_pauses_and_resumes() {
    let remote = connect_in("/home/alice").await;
    let local = TempDir::new().unwrap();
    let bytes = data(64 * MIB, 64);
    write_local(local.path(), "big.bin", &bytes);
    let source_hash = Sha256::digest(&bytes);
    drop(bytes);

    let write_offsets = Arc::new(Mutex::new(Vec::new()));
    // The upload cannot finish before the connection is killed: it stops after 8 MiB
    // until then.
    let gate = WriteGate::new(8 * MIB as u64);
    let recording = RecordingFs {
        inner: remote.fs.clone(),
        write_offsets: Arc::clone(&write_offsets),
        gate: Arc::clone(&gate),
    };
    let (engine, mut rx) = start_engine(local.path(), Arc::new(recording)).await;
    let watcher = spawn_site_watcher(engine.clone(), site(), Arc::new(remote.session.clone()));
    assert_eq!(remote.session.generation(), 1);

    let target = remote.at("big.bin");
    let id = engine
        .enqueue(transfer(Direction::Upload, "/big.bin", &target))
        .await
        .unwrap();
    let mut events = Vec::new();
    let first_progress = async {
        loop {
            let ev = rx.recv().await.unwrap();
            let progressed = matches!(&ev.kind, EventKind::Progress(us) if us.iter().any(|u| u.id == id && u.bytes_done > 0));
            events.push(ev);
            if progressed {
                break;
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(30), first_progress)
        .await
        .expect("no progress");
    kill_ssh_sessions();
    // The held stream now writes on the dead connection and fails.
    gate.release();

    let (rest, state) = run_until_terminal(&mut rx, id, Duration::from_secs(120)).await;
    events.extend(rest);
    watcher.abort();
    assert_eq!(state, TransferState::Done, "{events:?}");
    assert_eq!(site_paused_count(&events), 1, "{events:?}");
    assert!(failed(&events).is_empty(), "{:?}", failed(&events));
    assert!(remote.session.generation() >= 2);
    let offsets = write_offsets.lock().unwrap().clone();
    assert!(
        offsets.first() == Some(&0) && offsets.iter().any(|&o| o > 0),
        "the copy continued from its partial destination: {offsets:?}"
    );

    let remote_hash = Sha256::digest(remote.read(&target).await);
    assert_eq!(remote_hash, source_hash);
}

/// Review Focus 4: a full disk on the server fails the item that does not fit (`Io`),
/// without pausing the site; the site's other items still run.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn disk_full_fails_item_not_site() {
    // `/home/alice/small` is a 1 MiB tmpfs: two of these files fit, the third does not.
    let remote = connect_in("/home/alice/small").await;
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
    assert_eq!(*remote.session.state().borrow(), SessionState::Connected);
}
