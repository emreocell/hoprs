//! `SftpFs` metadata operations against the container in `docker/compose.yml`.
//!
//! Every test works in its own directory, `/home/alice/t-<nanos>-<counter>`, created through
//! `SftpFs` and removed (best effort) when the test ends. Files, FIFOs and symlinks are made
//! inside the container with `su alice -c ...`, since `open_write` is not part of this task.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use hoprs_core::{EntryKind, ErrorKind, RemotePath};
use hoprs_remote::{RemoteFs, SftpConfig, SftpFs};
use hoprs_ssh::testing::{docker, fixture, kill_ssh_sessions, test_config};
use hoprs_ssh::{KeySource, Prompter, ReconnectPolicy, ScriptedPrompter, SessionState, SshSession};
use tempfile::TempDir;
use tokio::sync::watch;

fn p(s: &str) -> RemotePath {
    RemotePath::new(s).unwrap()
}

/// Removes the work directory when dropped. Never panics.
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

struct Ctx {
    fs: SftpFs,
    session: SshSession,
    work: RemotePath,
    _cleanup: Cleanup,
    _known_hosts: TempDir,
}

impl Ctx {
    /// `work` joined with a relative path.
    fn at(&self, rel: &str) -> RemotePath {
        p(&format!("{}/{rel}", self.work))
    }

    /// Runs `cmd` as alice inside the work directory.
    fn sh(&self, cmd: &str) {
        let script = format!("cd {} && {cmd}", self.work);
        docker(&["exec", "-T", "sshd", "su", "alice", "-c", &script]);
    }
}

async fn connect(reconnect: Option<ReconnectPolicy>) -> (SshSession, TempDir) {
    let dir = TempDir::new().unwrap();
    let mut cfg = test_config("alice", dir.path());
    cfg.auth.key = KeySource::File {
        path: fixture("id_ed25519"),
        passphrase: None,
    };
    if let Some(policy) = reconnect {
        cfg.reconnect = policy;
    }
    let prompter = Arc::new(ScriptedPrompter::new().accept_host(true));
    let session = SshSession::connect(cfg, prompter as Arc<dyn Prompter>)
        .await
        .unwrap();
    (session, dir)
}

async fn setup_with(reconnect: Option<ReconnectPolicy>) -> Ctx {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let (session, known_hosts) = connect(reconnect).await;
    let fs = SftpFs::new(session.clone(), SftpConfig::default())
        .await
        .unwrap();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let work = format!("/home/alice/t-{nanos}-{n}");
    let cleanup = Cleanup(work.clone());
    fs.mkdir(&p(&work)).await.unwrap();
    Ctx {
        fs,
        session,
        work: p(&work),
        _cleanup: cleanup,
        _known_hosts: known_hosts,
    }
}

async fn setup() -> Ctx {
    setup_with(None).await
}

fn names(list: &[hoprs_core::RemoteEntry]) -> Vec<String> {
    let mut v: Vec<String> = list.iter().map(|e| e.name.clone()).collect();
    v.sort();
    v
}

async fn wait_state(
    rx: &mut watch::Receiver<SessionState>,
    pred: impl FnMut(&SessionState) -> bool,
    timeout: Duration,
) -> SessionState {
    let found = match tokio::time::timeout(timeout, rx.wait_for(pred)).await {
        Ok(Ok(state)) => Some(state.clone()),
        Ok(Err(_)) => panic!("session state channel closed"),
        Err(_) => None,
    };
    found.unwrap_or_else(|| panic!("timed out after {timeout:?}; state is {:?}", *rx.borrow()))
}

#[test]
fn sftp_fs_is_clone_send_sync() {
    fn check<T: Clone + Send + Sync + 'static>() {}
    check::<SftpFs>();
    assert_eq!(SftpConfig::default().stream_channels, 4);
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn list_returns_direct_children_without_dot_entries() {
    let c = setup().await;
    c.fs.mkdir(&c.at("a")).await.unwrap();
    c.fs.mkdir(&c.at("a/sub")).await.unwrap();
    c.sh("printf hello > a/f.txt && : > a/sub/deep");
    let list = c.fs.list(&c.at("a")).await.unwrap();
    assert_eq!(names(&list), ["f.txt", "sub"]);
    let f = list.iter().find(|e| e.name == "f.txt").unwrap();
    assert_eq!(f.path, c.at("a/f.txt"));
    assert_eq!(f.kind, EntryKind::File);
    assert_eq!(f.size, 5);
    assert!(f.modified.is_some());
    assert_eq!(f.mode.map(|m| m & !0o7777), Some(0));
    let sub = list.iter().find(|e| e.name == "sub").unwrap();
    assert_eq!(sub.kind, EntryKind::Dir);
    assert_eq!(sub.path, c.at("a/sub"));

    let st = c.fs.stat(&c.at("a/f.txt")).await.unwrap();
    assert_eq!(st.name, "f.txt");
    assert_eq!(st.kind, EntryKind::File);
    assert_eq!(st.size, 5);
    let root = c.fs.stat(&p("/")).await.unwrap();
    assert_eq!(root.kind, EntryKind::Dir);
    assert_eq!(root.name, "/");
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn stat_missing_is_not_found() {
    let c = setup().await;
    let missing = c.at("nope");
    let e = c.fs.stat(&missing).await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::NotFound);
    assert!(e.message.contains(missing.as_str()), "{e:?}");
    assert!(
        e.detail.as_deref().unwrap_or("").contains("NoSuchFile"),
        "{e:?}"
    );
    assert_eq!(
        c.fs.list(&missing).await.unwrap_err().kind,
        ErrorKind::NotFound
    );
    assert_eq!(
        c.fs.chmod(&missing, 0o600).await.unwrap_err().kind,
        ErrorKind::NotFound
    );
    assert_eq!(
        c.fs.set_modified(&missing, UNIX_EPOCH)
            .await
            .unwrap_err()
            .kind,
        ErrorKind::NotFound
    );
    assert_eq!(
        c.fs.remove_file(&missing).await.unwrap_err().kind,
        ErrorKind::NotFound
    );
    assert_eq!(
        c.fs.remove_dir(&missing).await.unwrap_err().kind,
        ErrorKind::NotFound
    );
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn list_on_file_is_io() {
    let c = setup().await;
    c.sh(": > f");
    let e = c.fs.list(&c.at("f")).await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::Io, "{e:?}");
    assert!(e.message.contains(c.at("f").as_str()), "{e:?}");
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn mkdir_existing_ok() {
    let c = setup().await;
    c.fs.mkdir(&c.at("d")).await.unwrap();
    c.fs.mkdir(&c.at("d")).await.unwrap();
    assert_eq!(c.fs.stat(&c.at("d")).await.unwrap().kind, EntryKind::Dir);
    c.sh(": > f");
    assert_eq!(
        c.fs.mkdir(&c.at("f")).await.unwrap_err().kind,
        ErrorKind::Io
    );
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn mkdir_missing_parent_not_found() {
    let c = setup().await;
    assert_eq!(
        c.fs.mkdir(&c.at("x/y")).await.unwrap_err().kind,
        ErrorKind::NotFound
    );
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn rename_file_and_dir() {
    let c = setup().await;
    c.sh("printf x > a.txt");
    c.fs.rename(&c.at("a.txt"), &c.at("b.txt")).await.unwrap();
    assert_eq!(
        c.fs.stat(&c.at("a.txt")).await.unwrap_err().kind,
        ErrorKind::NotFound
    );
    assert_eq!(c.fs.stat(&c.at("b.txt")).await.unwrap().size, 1);

    // posix-rename@openssh.com replaces an existing target (plain SFTP rename refuses).
    c.sh("printf abc > c.txt");
    c.fs.rename(&c.at("c.txt"), &c.at("b.txt")).await.unwrap();
    assert_eq!(c.fs.stat(&c.at("b.txt")).await.unwrap().size, 3);
    assert_eq!(
        c.fs.stat(&c.at("c.txt")).await.unwrap_err().kind,
        ErrorKind::NotFound
    );

    c.fs.mkdir(&c.at("d")).await.unwrap();
    c.fs.mkdir(&c.at("d/e")).await.unwrap();
    c.sh("printf y > d/e/f.txt");
    c.fs.rename(&c.at("d"), &c.at("g")).await.unwrap();
    assert_eq!(c.fs.stat(&c.at("g/e/f.txt")).await.unwrap().size, 1);
    assert_eq!(
        c.fs.stat(&c.at("d")).await.unwrap_err().kind,
        ErrorKind::NotFound
    );
    assert_eq!(
        c.fs.rename(&c.at("nope"), &c.at("z"))
            .await
            .unwrap_err()
            .kind,
        ErrorKind::NotFound
    );
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn remove_dir_non_empty_is_io() {
    let c = setup().await;
    c.fs.mkdir(&c.at("d")).await.unwrap();
    c.sh(": > d/f");
    assert_eq!(
        c.fs.remove_dir(&c.at("d")).await.unwrap_err().kind,
        ErrorKind::Io
    );
    assert_eq!(
        c.fs.remove_file(&c.at("d")).await.unwrap_err().kind,
        ErrorKind::Io
    );
    assert_eq!(
        c.fs.remove_dir(&c.at("d/f")).await.unwrap_err().kind,
        ErrorKind::Io
    );
    c.fs.remove_file(&c.at("d/f")).await.unwrap();
    c.fs.remove_dir(&c.at("d")).await.unwrap();
    assert_eq!(
        c.fs.remove_file(&c.at("d/f")).await.unwrap_err().kind,
        ErrorKind::NotFound
    );
    assert_eq!(
        c.fs.stat(&c.at("d")).await.unwrap_err().kind,
        ErrorKind::NotFound
    );
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn chmod_and_set_modified_roundtrip() {
    let c = setup().await;
    c.sh("printf data > m");
    c.fs.chmod(&c.at("m"), 0o640).await.unwrap();
    assert_eq!(c.fs.stat(&c.at("m")).await.unwrap().mode, Some(0o640));
    let t = UNIX_EPOCH + Duration::from_secs(1_000_000);
    c.fs.set_modified(&c.at("m"), t).await.unwrap();
    let st = c.fs.stat(&c.at("m")).await.unwrap();
    assert_eq!(st.modified, Some(t));
    assert_eq!(st.mode, Some(0o640));
    // atime is set to the same value.
    c.sh("test \"$(stat -c %X m)\" = 1000000");
    // Directories too.
    c.fs.mkdir(&c.at("d")).await.unwrap();
    c.fs.set_modified(&c.at("d"), t).await.unwrap();
    assert_eq!(c.fs.stat(&c.at("d")).await.unwrap().modified, Some(t));
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn symlink_listed_not_followed() {
    let c = setup().await;
    c.fs.mkdir(&c.at("target")).await.unwrap();
    c.sh(": > target/inside && ln -s target link && ln -s missing dangling");
    let list = c.fs.list(&c.work).await.unwrap();
    assert_eq!(names(&list), ["dangling", "link", "target"]);
    for name in ["link", "dangling"] {
        let e = list.iter().find(|e| e.name == name).unwrap();
        assert_eq!(e.kind, EntryKind::Symlink, "{name}");
        assert_eq!(e.size, 0, "{name}");
    }
    assert_eq!(
        c.fs.stat(&c.at("link")).await.unwrap().kind,
        EntryKind::Symlink
    );
    assert_eq!(
        c.fs.stat(&c.at("dangling")).await.unwrap().kind,
        EntryKind::Symlink
    );
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn special_files_hidden() {
    let c = setup().await;
    c.sh("mkfifo fifo && : > plain");
    let list = c.fs.list(&c.work).await.unwrap();
    assert_eq!(names(&list), ["plain"]);
    let e = c.fs.stat(&c.at("fifo")).await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::Io, "{e:?}");
    assert!(e.message.contains("Not a regular file"), "{e:?}");
    assert!(e.message.contains(c.at("fifo").as_str()), "{e:?}");
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn lists_10k_entries() {
    let c = setup().await;
    c.fs.mkdir(&c.at("big")).await.unwrap();
    c.sh("cd big && seq -f 'f%g' 1 10000 | xargs touch");
    let list = c.fs.list(&c.at("big")).await.unwrap();
    assert_eq!(list.len(), 10_000);
    let mut n = names(&list);
    n.dedup();
    assert_eq!(n.len(), 10_000);
    assert!(list.iter().all(|e| e.kind == EntryKind::File));
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn permission_denied_maps() {
    let c = setup().await;
    let e = c.fs.stat(&p("/root/x")).await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::Permission, "{e:?}");
    assert!(e.message.contains("/root/x"), "{e:?}");
    assert_eq!(
        c.fs.list(&p("/root")).await.unwrap_err().kind,
        ErrorKind::Permission
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn reopens_channels_after_reconnect() {
    // The first reconnection attempt waits 2 s, which keeps the session in Reconnecting
    // long enough to observe it.
    let c = setup_with(Some(ReconnectPolicy {
        max_attempts: 5,
        base_delay: Duration::from_secs(2),
        max_delay: Duration::from_secs(2),
    }))
    .await;
    c.fs.mkdir(&c.at("d")).await.unwrap();
    assert!(c.fs.list(&c.work).await.unwrap().len() == 1);
    assert_eq!(c.session.generation(), 1);

    let mut rx = c.session.state();
    kill_ssh_sessions();
    wait_state(
        &mut rx,
        |s| matches!(s, SessionState::Reconnecting { .. }),
        Duration::from_secs(10),
    )
    .await;
    let start = Instant::now();
    let e = c.fs.list(&c.work).await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::Network, "{e:?}");
    assert!(start.elapsed() < Duration::from_millis(500), "list waited");
    assert_eq!(
        c.fs.stat(&c.work).await.unwrap_err().kind,
        ErrorKind::Network
    );

    wait_state(
        &mut rx,
        |s| *s == SessionState::Connected,
        Duration::from_secs(20),
    )
    .await;
    assert_eq!(c.session.generation(), 2);
    assert_eq!(names(&c.fs.list(&c.work).await.unwrap()), ["d"]);
    assert_eq!(c.fs.stat(&c.at("d")).await.unwrap().kind, EntryKind::Dir);
}
