//! Tests of remote editing, over `MemFs` and a temporary directory.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::UNIX_EPOCH;

use async_trait::async_trait;
use hoprs_core::{ErrorKind, RemoteEntry};
use hoprs_remote::{BoxAsyncRead, BoxAsyncWrite, MemFs};
use tokio::time::Instant;

use super::*;

/// `MemFs` that counts writes and can lose the network.
struct CountingFs {
    inner: MemFs,
    writes: AtomicUsize,
    offline: AtomicBool,
}

impl CountingFs {
    fn check(&self) -> Result<()> {
        if self.offline.load(Ordering::SeqCst) {
            Err(Error::new(ErrorKind::Network, "Connection lost"))
        } else {
            Ok(())
        }
    }
}

#[async_trait]
impl RemoteFs for CountingFs {
    async fn list(&self, path: &RemotePath) -> Result<Vec<RemoteEntry>> {
        self.check()?;
        self.inner.list(path).await
    }
    async fn stat(&self, path: &RemotePath) -> Result<RemoteEntry> {
        self.check()?;
        self.inner.stat(path).await
    }
    async fn open_read(&self, path: &RemotePath, offset: u64) -> Result<BoxAsyncRead> {
        self.check()?;
        self.inner.open_read(path, offset).await
    }
    async fn open_write(&self, path: &RemotePath, offset: u64) -> Result<BoxAsyncWrite> {
        self.check()?;
        self.writes.fetch_add(1, Ordering::SeqCst);
        self.inner.open_write(path, offset).await
    }
    async fn mkdir(&self, path: &RemotePath) -> Result<()> {
        self.check()?;
        self.inner.mkdir(path).await
    }
    async fn rename(&self, from: &RemotePath, to: &RemotePath) -> Result<()> {
        self.check()?;
        self.inner.rename(from, to).await
    }
    async fn remove_file(&self, path: &RemotePath) -> Result<()> {
        self.check()?;
        self.inner.remove_file(path).await
    }
    async fn remove_dir(&self, path: &RemotePath) -> Result<()> {
        self.check()?;
        self.inner.remove_dir(path).await
    }
    async fn chmod(&self, path: &RemotePath, mode: u32) -> Result<()> {
        self.check()?;
        self.inner.chmod(path, mode).await
    }
}

type Launched = Arc<Mutex<Vec<(String, PathBuf)>>>;

struct Harness {
    _dir: tempfile::TempDir,
    root: PathBuf,
    edits: EditManager,
    mem: MemFs,
    fs: Arc<CountingFs>,
    state: watch::Sender<ConnectionState>,
    launched: Launched,
    events: broadcast::Receiver<EditEvent>,
}

const SESSION: SessionId = SessionId(7);
const FILE: &str = "/home/alice/notes.txt";

fn rp(path: &str) -> RemotePath {
    RemotePath::new(path).unwrap()
}

fn harness(watch_files: bool) -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("edit");
    let launched: Launched = Arc::default();
    let record = launched.clone();
    let launcher: Launcher = Arc::new(move |command: &str, path: &Path| {
        record
            .lock()
            .unwrap()
            .push((command.to_owned(), path.to_owned()));
        Ok(())
    });
    let edits = EditManager::with_options(root.clone(), watch_files, launcher);
    let mem = MemFs::new();
    mem.add_file(
        FILE,
        b"version 1\n",
        UNIX_EPOCH + Duration::from_secs(1_000),
    );
    let fs = Arc::new(CountingFs {
        inner: mem.clone(),
        writes: AtomicUsize::new(0),
        offline: AtomicBool::new(false),
    });
    let events = edits.subscribe();
    Harness {
        _dir: dir,
        root,
        edits,
        mem,
        fs,
        state: watch::channel(ConnectionState::Connected).0,
        launched,
        events,
    }
}

impl Harness {
    async fn start(&self, path: &str) -> Result<EditId> {
        self.edits
            .start(
                SESSION,
                self.fs.clone(),
                self.state.subscribe(),
                UploadMode::TempRename,
                rp(path),
                "my-editor --wait",
            )
            .await
    }

    fn writes(&self) -> usize {
        self.fs.writes.load(Ordering::SeqCst)
    }

    /// The editor saves `data` (a plain write) and the watcher reports it.
    fn save(&self, id: EditId, data: &[u8]) {
        let local = self.edits.local_path(id).unwrap();
        std::fs::write(&local, data).unwrap();
        self.edits.file_event(SESSION, local);
    }

    async fn next_event(&mut self) -> EditEvent {
        tokio::time::timeout(Duration::from_secs(30), self.events.recv())
            .await
            .expect("an edit event")
            .expect("the event channel is open")
    }
}

/// Lets the worker take what was sent to it, without moving the paused clock.
async fn settle() {
    for _ in 0..50 {
        tokio::task::yield_now().await;
    }
}

#[cfg(unix)]
#[tokio::test]
async fn download_goes_to_private_dir_with_0700_0600() {
    use std::os::unix::fs::PermissionsExt;

    let h = harness(false);
    let id = h.start(FILE).await.unwrap();
    let local = h.edits.local_path(id).unwrap();

    let session_dir = h.root.join(SESSION.0.to_string());
    assert_eq!(
        local,
        std::fs::canonicalize(&session_dir)
            .unwrap()
            .join("home")
            .join("alice")
            .join("notes.txt")
    );
    assert_eq!(std::fs::read(&local).unwrap(), b"version 1\n");
    let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&h.root), 0o700);
    assert_eq!(mode(&session_dir), 0o700);
    assert_eq!(mode(&session_dir.join("home")), 0o700);
    assert_eq!(mode(&session_dir.join("home/alice")), 0o700);
    assert_eq!(mode(&local), 0o600);

    // The editor got the command and the path, once.
    let launched = h.launched.lock().unwrap().clone();
    assert_eq!(launched, vec![("my-editor --wait".to_owned(), local)]);
    // Nothing went back up.
    assert_eq!(h.writes(), 0);
}

#[test]
fn path_traversal_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let session = dir.path().join("7");
    for bad in [
        "/../etc/passwd",
        "/a/../../etc/passwd",
        "/a/./b",
        "/a//b",
        "relative/path",
        "/",
        "",
        "/a/..",
        "/a\\..\\..\\b",
        "/nul\0byte",
    ] {
        let err = local_path_for(&session, bad).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Permission, "{bad:?} must be refused");
    }
    assert_eq!(
        local_path_for(&session, "/srv/www/index.html").unwrap(),
        session.join("srv").join("www").join("index.html")
    );
    // A name that merely contains dots is fine.
    assert_eq!(
        local_path_for(&session, "/a/..b/c..").unwrap(),
        session.join("a").join("..b").join("c..")
    );
}

#[tokio::test(start_paused = true)]
async fn save_uploads_after_500ms_debounce() {
    let mut h = harness(false);
    let id = h.start(FILE).await.unwrap();

    // Three saves 200 ms apart: one upload, 500 ms after the last.
    h.save(id, b"draft a\n");
    settle().await;
    tokio::time::advance(Duration::from_millis(200)).await;
    h.save(id, b"draft b\n");
    settle().await;
    tokio::time::advance(Duration::from_millis(200)).await;
    h.save(id, b"final\n");
    let last = Instant::now();
    settle().await;
    tokio::time::advance(DEBOUNCE - Duration::from_millis(1)).await;
    settle().await;
    assert!(Instant::now() - last < DEBOUNCE);
    assert_eq!(h.writes(), 0, "no upload before 500 ms of quiet");
    assert_eq!(h.mem.read_file(FILE).unwrap(), b"version 1\n");

    let event = h.next_event().await;
    assert!(
        matches!(event, EditEvent::Uploaded { id: got, .. } if got == id),
        "{event:?}"
    );
    assert_eq!(h.writes(), 1, "the three saves made one upload");
    assert_eq!(h.mem.read_file(FILE).unwrap(), b"final\n");

    // A watcher event without a change of content uploads nothing.
    let local = h.edits.local_path(id).unwrap();
    h.edits.file_event(SESSION, local);
    tokio::time::advance(DEBOUNCE * 2).await;
    settle().await;
    // The worker handles its files in order: had that event uploaded, this save would
    // be the third write.
    h.save(id, b"later\n");
    tokio::time::advance(DEBOUNCE).await;
    assert!(matches!(h.next_event().await, EditEvent::Uploaded { .. }));
    assert_eq!(h.writes(), 2);
    // No temporary file left beside the original.
    let names: Vec<String> = h
        .mem
        .list(&rp("/home/alice"))
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(names, vec!["notes.txt".to_owned()]);
}

/// The real watcher (`notify`), real files, real time: an editor that saves through a
/// temporary file renamed over the original, then one that removes and recreates it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn atomic_rename_saves_are_seen() {
    let mut h = harness(true);
    let id = h.start(FILE).await.unwrap();
    let local = h.edits.local_path(id).unwrap();
    let dir = local.parent().unwrap().to_owned();

    // vim/gedit style: write a temporary file, rename it over the original.
    let temp = dir.join(".notes.txt.swp");
    std::fs::write(&temp, b"renamed over\n").unwrap();
    std::fs::rename(&temp, &local).unwrap();
    loop {
        if let EditEvent::Uploaded { id: got, .. } = h.next_event().await {
            assert_eq!(got, id);
            break;
        }
    }
    assert_eq!(h.mem.read_file(FILE).unwrap(), b"renamed over\n");

    // Remove, then create anew.
    std::fs::remove_file(&local).unwrap();
    std::fs::write(&local, b"recreated\n").unwrap();
    loop {
        if let EditEvent::Uploaded { id: got, .. } = h.next_event().await {
            assert_eq!(got, id);
            break;
        }
    }
    assert_eq!(h.mem.read_file(FILE).unwrap(), b"recreated\n");
}

/// Review Focus 2: the server's copy changed after the download; the save is not
/// uploaded, the UI is told, and nothing is overwritten until the user chooses.
#[tokio::test(start_paused = true)]
async fn remote_change_blocks_upload_and_offers_choices() {
    let mut h = harness(false);
    let id = h.start(FILE).await.unwrap();

    // Someone else changes the file on the server (same size, newer mtime).
    h.mem.add_file(
        FILE,
        b"version X\n",
        UNIX_EPOCH + Duration::from_secs(2_000),
    );
    h.save(id, b"my change\n");
    tokio::time::advance(DEBOUNCE).await;
    assert_eq!(h.next_event().await, EditEvent::RemoteChanged { id });
    assert_eq!(h.writes(), 0);
    assert_eq!(h.mem.read_file(FILE).unwrap(), b"version X\n");
    let pending = h.edits.pending(SESSION).await;
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].state, EditState::Conflict);

    // Saving again still does not overwrite it.
    h.save(id, b"my change, again\n");
    tokio::time::advance(DEBOUNCE).await;
    assert_eq!(h.next_event().await, EditEvent::RemoteChanged { id });
    assert_eq!(h.mem.read_file(FILE).unwrap(), b"version X\n");

    // A change of size only is caught too.
    let other = harness(false);
    let id2 = other.start(FILE).await.unwrap();
    other.mem.add_file(
        FILE,
        b"longer v1\n!",
        UNIX_EPOCH + Duration::from_secs(1_000),
    );
    let mut events = other.edits.subscribe();
    other.save(id2, b"mine\n");
    tokio::time::advance(DEBOUNCE).await;
    assert_eq!(
        events.recv().await.unwrap(),
        EditEvent::RemoteChanged { id: id2 }
    );

    // "Overwrite": the user's version goes up.
    h.edits.resolve(id, EditChoice::Overwrite).await.unwrap();
    assert!(matches!(h.next_event().await, EditEvent::Uploaded { .. }));
    assert_eq!(h.mem.read_file(FILE).unwrap(), b"my change, again\n");
    assert!(h.edits.pending(SESSION).await.is_empty());

    // And later saves upload normally against the new stamp.
    h.save(id, b"after overwrite\n");
    tokio::time::advance(DEBOUNCE).await;
    assert!(matches!(h.next_event().await, EditEvent::Uploaded { .. }));
    assert_eq!(h.mem.read_file(FILE).unwrap(), b"after overwrite\n");
}

#[tokio::test(start_paused = true)]
async fn keep_remote_preserves_local_copy() {
    let mut h = harness(false);
    let id = h.start(FILE).await.unwrap();
    let local = h.edits.local_path(id).unwrap();
    h.mem.add_file(
        FILE,
        b"their version\n",
        UNIX_EPOCH + Duration::from_secs(2_000),
    );
    h.save(id, b"my version\n");
    tokio::time::advance(DEBOUNCE).await;
    assert_eq!(h.next_event().await, EditEvent::RemoteChanged { id });

    h.edits.resolve(id, EditChoice::KeepRemote).await.unwrap();
    assert_eq!(std::fs::read(&local).unwrap(), b"their version\n");
    let copies: Vec<PathBuf> = std::fs::read_dir(local.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("notes.txt.local-")
        })
        .collect();
    assert_eq!(copies.len(), 1, "one preserved local copy");
    assert_eq!(std::fs::read(&copies[0]).unwrap(), b"my version\n");
    assert_eq!(h.mem.read_file(FILE).unwrap(), b"their version\n");
    assert_eq!(h.writes(), 0);
    assert!(h.edits.pending(SESSION).await.is_empty());

    // The next save goes up against the stamp of what was kept.
    h.save(id, b"merged\n");
    tokio::time::advance(DEBOUNCE).await;
    assert!(matches!(h.next_event().await, EditEvent::Uploaded { .. }));
    assert_eq!(h.mem.read_file(FILE).unwrap(), b"merged\n");
}

#[tokio::test(start_paused = true)]
async fn save_copy_uploads_beside() {
    let mut h = harness(false);
    let id = h.start(FILE).await.unwrap();
    let local = h.edits.local_path(id).unwrap();
    h.mem.add_file(
        FILE,
        b"their version\n",
        UNIX_EPOCH + Duration::from_secs(2_000),
    );
    h.save(id, b"my version\n");
    tokio::time::advance(DEBOUNCE).await;
    assert_eq!(h.next_event().await, EditEvent::RemoteChanged { id });

    h.edits.resolve(id, EditChoice::SaveCopy).await.unwrap();
    let entries = h.mem.list(&rp("/home/alice")).await.unwrap();
    let copy = entries
        .iter()
        .find(|e| e.name.starts_with("notes.txt.hoprs-"))
        .expect("a copy beside the original");
    assert_eq!(entries.len(), 2, "{entries:?}");
    assert_eq!(
        h.mem.read_file(copy.path.as_str()).unwrap(),
        b"my version\n"
    );
    // The original is untouched, and the local file now follows it.
    assert_eq!(h.mem.read_file(FILE).unwrap(), b"their version\n");
    assert_eq!(std::fs::read(&local).unwrap(), b"their version\n");
    assert!(h.edits.pending(SESSION).await.is_empty());
}

/// Review Focus 3: a save while the session is down is held, then uploaded by itself
/// once the session is connected again.
#[tokio::test(start_paused = true)]
async fn pending_edit_uploads_after_reconnect() {
    let mut h = harness(false);
    let id = h.start(FILE).await.unwrap();

    // The connection drops (idle timeout); the transport is reconnecting.
    h.fs.offline.store(true, Ordering::SeqCst);
    h.state
        .send_replace(ConnectionState::Reconnecting { attempt: 1 });
    h.save(id, b"saved while down\n");
    tokio::time::advance(DEBOUNCE).await;
    assert_eq!(h.next_event().await, EditEvent::Pending { id });
    assert_eq!(h.mem.read_file(FILE).unwrap(), b"version 1\n");
    let pending = h.edits.pending(SESSION).await;
    assert_eq!(pending[0].state, EditState::Pending);

    // Gave up, then the user (or the monitor) reconnects: still pending meanwhile.
    h.state
        .send_replace(ConnectionState::Disconnected(Error::new(
            ErrorKind::Network,
            "Connection lost",
        )));
    settle().await;
    assert_eq!(h.writes(), 0);

    h.fs.offline.store(false, Ordering::SeqCst);
    h.state.send_replace(ConnectionState::Connected);
    assert!(matches!(
        h.next_event().await,
        EditEvent::Uploaded { id: got, .. } if got == id
    ));
    assert_eq!(h.mem.read_file(FILE).unwrap(), b"saved while down\n");
    assert!(h.edits.pending(SESSION).await.is_empty());

    // The state said connected but the link was dead: the failed upload is held too.
    h.fs.offline.store(true, Ordering::SeqCst);
    h.save(id, b"second\n");
    tokio::time::advance(DEBOUNCE).await;
    assert_eq!(h.next_event().await, EditEvent::Pending { id });
    h.state
        .send_replace(ConnectionState::Reconnecting { attempt: 1 });
    settle().await;
    h.fs.offline.store(false, Ordering::SeqCst);
    h.state.send_replace(ConnectionState::Connected);
    assert!(matches!(h.next_event().await, EditEvent::Uploaded { .. }));
    assert_eq!(h.mem.read_file(FILE).unwrap(), b"second\n");
}

#[tokio::test(start_paused = true)]
async fn finish_keeps_files_with_pending_changes() {
    let mut h = harness(false);
    let id = h.start(FILE).await.unwrap();
    let local = h.edits.local_path(id).unwrap();
    let session_dir = local.parent().unwrap().parent().unwrap().parent().unwrap();
    assert_eq!(session_dir.file_name().unwrap(), "7");

    // Saved, but not uploaded yet (the watcher has not even said so).
    std::fs::write(&local, b"unsent\n").unwrap();
    let kept = h.edits.finish(SESSION, false).await.unwrap();
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].id, id);
    assert_eq!(kept[0].state, EditState::Modified);
    assert_eq!(kept[0].remote, rp(FILE));
    assert!(local.exists(), "pending changes keep the file");

    // Discarding removes them.
    let kept = h.edits.finish(SESSION, true).await.unwrap();
    assert!(kept.is_empty());
    assert!(!session_dir.exists());
    assert_eq!(h.next_event().await, EditEvent::Closed { id });
    assert!(h.edits.local_path(id).is_none());

    // Without pending changes the files go without asking.
    let id = h.start(FILE).await.unwrap();
    let local = h.edits.local_path(id).unwrap();
    assert!(h.edits.finish(SESSION, false).await.unwrap().is_empty());
    assert!(!local.exists());
    assert_eq!(h.next_event().await, EditEvent::Closed { id });
}

#[test]
fn editor_command_is_split_without_shell() {
    let path = Path::new("/run/user/1000/hoprs/edit/7/my file; rm -rf ~.txt");
    let argv = |command: &str| -> Vec<String> {
        editor_argv(command, path)
            .unwrap()
            .into_iter()
            .map(|a| a.into_string().unwrap())
            .collect()
    };
    let last = path.to_str().unwrap().to_owned();
    assert_eq!(argv("xdg-open"), vec!["xdg-open".to_owned(), last.clone()]);
    assert_eq!(
        argv("code --wait --new-window"),
        vec!["code", "--wait", "--new-window", last.as_str()]
    );
    assert_eq!(
        argv("'/opt/My Editor/bin/edit' -n \"two words\""),
        vec!["/opt/My Editor/bin/edit", "-n", "two words", last.as_str()]
    );
    // Shell syntax stays literal: no expansion, no command substitution, no pipes.
    assert_eq!(
        argv("vim $HOME $(id) | tee"),
        vec!["vim", "$HOME", "$(id)", "|", "tee", last.as_str()]
    );
    for bad in ["", "   ", "'unterminated"] {
        assert!(editor_argv(bad, path).is_err(), "{bad:?}");
    }
}

/// The real launcher: the path reaches the program as one argument, shell characters
/// and all, and nothing in it runs.
#[cfg(unix)]
#[test]
fn launcher_passes_the_path_as_one_argument() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("argv");
    let pwned = dir.path().join("pwned");
    let path = dir
        .path()
        .join(format!("x; touch {}", pwned.to_str().unwrap()));
    // `sh -c SCRIPT NAME ARG`: the script writes its first argument ($1) to `out`.
    let command = format!(
        "sh -c 'printf %s \"$1\" > \"$0\"' {}",
        shlex::try_quote(out.to_str().unwrap()).unwrap()
    );
    launch_editor(&command, &path).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !out.exists() || std::fs::read(&out).unwrap().is_empty() {
        assert!(std::time::Instant::now() < deadline, "the editor never ran");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        std::fs::read_to_string(&out).unwrap(),
        path.to_str().unwrap()
    );
    assert!(!pwned.exists());
}
