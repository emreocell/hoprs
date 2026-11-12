mod common;

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use common::*;
use hoprs_core::{EntryKind, ErrorKind, RemoteEntry, RemotePath, Result};
use hoprs_remote::{BoxAsyncRead, BoxAsyncWrite, FaultFs, MemFs, Op, RemoteFs};
use hoprs_transfer::{Direction, EngineConfig, Event, TransferItem, TransferState};
use tokio::sync::broadcast::error::RecvError;

const SYMLINK_SKIP: &str = "symlinks are not followed";

/// Files of the `/w/site` tree used by several tests: (relative path, contents).
fn site_files() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("index.html", b"<html></html>".to_vec()),
        (".env", b"SECRET=1".to_vec()),
        ("relatório final.pdf", data(70_000)),
        ("css/app.css", b"body { margin: 0 }".to_vec()),
    ]
}

/// `/w/site/{index.html, .env, "relatório final.pdf", css/app.css, img/}`.
fn add_site(fs: &MemFs, root: &str) {
    for (rel, bytes) in site_files() {
        fs.add_file(&format!("{root}/{rel}"), &bytes, mtime());
    }
    fs.add_dir(&format!("{root}/img"));
}

fn item<'a>(snap: &'a [TransferItem], local: &str) -> &'a TransferItem {
    snap.iter()
        .find(|i| i.spec.local.as_str() == local)
        .unwrap_or_else(|| panic!("no item for {local}"))
}

#[tokio::test(flavor = "multi_thread")]
async fn uploads_tree() {
    let (engine, local, remote, mut rx) = engine_with_memfs(4).await;
    add_site(&local, "/w/site");
    remote.add_dir("/srv");

    let root = engine
        .enqueue(spec(Direction::Upload, "/w/site", "/srv/site"))
        .await
        .unwrap();
    assert_eq!(wait_terminal(&mut rx, root).await, TransferState::Done);

    for (rel, bytes) in site_files() {
        assert_eq!(
            remote.read_file(&format!("/srv/site/{rel}")).as_deref(),
            Some(bytes.as_slice()),
            "{rel}"
        );
    }
    let img = remote.stat(&path("/srv/site/img")).await.unwrap();
    assert_eq!(img.kind, EntryKind::Dir);

    let snap = engine.snapshot().await;
    assert_eq!(snap.len(), 7, "root, 3 files, css, css/app.css, img");
    assert!(snap.iter().all(|i| i.state == TransferState::Done));
    let css = item(&snap, "/w/site/css");
    assert_eq!(css.parent, Some(root));
    assert_eq!(css.is_dir, Some(true));
    assert_eq!(css.spec.remote.as_str(), "/srv/site/css");
    let app = item(&snap, "/w/site/css/app.css");
    assert_eq!(app.parent, Some(css.id));
    assert_eq!(app.spec.remote.as_str(), "/srv/site/css/app.css");
    assert_eq!(item(&snap, "/w/site/img").is_dir, Some(true));
}

#[tokio::test(flavor = "multi_thread")]
async fn downloads_tree() {
    let (engine, local, remote, mut rx) = engine_with_memfs(2).await;
    add_site(&remote, "/srv/site");
    local.add_dir("/w");

    let root = engine
        .enqueue(spec(Direction::Download, "/w/site", "/srv/site"))
        .await
        .unwrap();
    assert_eq!(wait_terminal(&mut rx, root).await, TransferState::Done);
    for (rel, bytes) in site_files() {
        assert_eq!(
            local.read_file(&format!("/w/site/{rel}")).as_deref(),
            Some(bytes.as_slice()),
            "{rel}"
        );
    }
    let img = local.stat(&path("/w/site/img")).await.unwrap();
    assert_eq!(img.kind, EntryKind::Dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn symlink_is_skipped() {
    let (engine, local, remote, mut rx) = engine_with_memfs(4).await;
    local.add_file("/w/site/a.txt", b"a", mtime());
    local.add_symlink("/w/site/link", "/etc/passwd");
    remote.add_dir("/srv");

    let root = engine
        .enqueue(spec(Direction::Upload, "/w/site", "/srv/site"))
        .await
        .unwrap();
    let mut link_added = None;
    let root_state = loop {
        match wait_for(&mut rx, |_| true).await {
            Event::Added(i) if i.spec.local.as_str() == "/w/site/link" => link_added = Some(i),
            Event::StateChanged { id, state } if id == root && state.is_terminal() => break state,
            _ => {}
        }
    };
    assert_eq!(root_state, TransferState::Done);
    let link_added = link_added.expect("Added event for the symlink");
    assert_eq!(link_added.parent, Some(root));
    assert_eq!(
        link_added.state,
        TransferState::Skipped(SYMLINK_SKIP.to_string())
    );

    let snap = engine.snapshot().await;
    assert_eq!(
        item(&snap, "/w/site/link").state,
        TransferState::Skipped(SYMLINK_SKIP.to_string())
    );
    assert_eq!(item(&snap, "/w/site/a.txt").state, TransferState::Done);
    assert!(remote.stat(&path("/srv/site/link")).await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn root_symlink_is_skipped() {
    let (engine, local, remote, mut rx) = engine_with_memfs(4).await;
    local.add_symlink("/w/link", "/w/elsewhere");
    remote.add_dir("/srv");

    let root = engine
        .enqueue(spec(Direction::Upload, "/w/link", "/srv/link"))
        .await
        .unwrap();
    assert_eq!(
        wait_terminal(&mut rx, root).await,
        TransferState::Skipped(SYMLINK_SKIP.to_string())
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn child_failure_fails_parent() {
    let (engine, local, remote, mut rx) = engine_with_memfs(4).await;
    add_site(&local, "/w/site");
    // The css directory already exists remotely; writing into it is denied.
    remote.add_dir("/srv/site/css");
    remote.deny_writes_under("/srv/site/css");

    let root = engine
        .enqueue(spec(Direction::Upload, "/w/site", "/srv/site"))
        .await
        .unwrap();
    match wait_terminal(&mut rx, root).await {
        TransferState::Failed(e) => {
            assert_eq!(e.kind, ErrorKind::Io);
            assert_eq!(e.message, "1 items failed");
        }
        other => panic!("root ended {other:?}"),
    }

    let snap = engine.snapshot().await;
    match &item(&snap, "/w/site/css/app.css").state {
        TransferState::Failed(e) => assert_eq!(e.kind, ErrorKind::Permission),
        other => panic!("app.css ended {other:?}"),
    }
    match &item(&snap, "/w/site/css").state {
        TransferState::Failed(e) => assert_eq!(e.message, "1 items failed"),
        other => panic!("css ended {other:?}"),
    }
    for done in [
        "/w/site/index.html",
        "/w/site/.env",
        "/w/site/relatório final.pdf",
        "/w/site/img",
    ] {
        assert_eq!(item(&snap, done).state, TransferState::Done, "{done}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn list_failure_fails_directory() {
    let local_mem = MemFs::new();
    local_mem.add_file("/w/site/a.txt", b"a", mtime());
    let local = FaultFs::new(local_mem);
    local.fail_next(Op::List, ErrorKind::Permission);
    let remote = MemFs::new();
    remote.add_dir("/srv");
    let (engine, mut rx) = engine_with(
        Arc::new(local),
        Arc::new(remote.clone()),
        4,
        EngineConfig::default(),
    )
    .await;

    let root = engine
        .enqueue(spec(Direction::Upload, "/w/site", "/srv/site"))
        .await
        .unwrap();
    match wait_terminal(&mut rx, root).await {
        TransferState::Failed(e) => assert_eq!(e.kind, ErrorKind::Permission),
        other => panic!("root ended {other:?}"),
    }
    assert_eq!(engine.snapshot().await.len(), 1, "no children inserted");
}

#[tokio::test(flavor = "multi_thread")]
async fn large_tree_is_lazy() {
    const DIRS: usize = 50;
    const FILES_PER_DIR: usize = 100;
    const TOTAL: usize = 1 + DIRS + DIRS * FILES_PER_DIR;

    let (engine, local, remote, mut rx) = engine_with_memfs(4).await;
    for d in 0..DIRS {
        for f in 0..FILES_PER_DIR {
            local.add_file(
                &format!("/w/big/d{d}/f{f}"),
                format!("{d}:{f}").as_bytes(),
                mtime(),
            );
        }
    }
    remote.add_dir("/srv");

    let root = engine
        .enqueue(spec(Direction::Upload, "/w/big", "/srv/big"))
        .await
        .unwrap();

    let mut added = 0usize;
    let mut files = HashSet::new();
    let mut added_at_first_file_done = None;
    let collect = async {
        loop {
            match rx.recv().await {
                Ok(Event::Added(i)) => {
                    added += 1;
                    if i.is_dir == Some(false) {
                        files.insert(i.id);
                    }
                }
                Ok(Event::StateChanged { id, state }) => {
                    if state == TransferState::Done
                        && files.contains(&id)
                        && added_at_first_file_done.is_none()
                    {
                        added_at_first_file_done = Some(added);
                    }
                    if id == root && state.is_terminal() {
                        return state;
                    }
                }
                Ok(_) => {}
                // A lagged receiver would make the ordering check meaningless.
                Err(RecvError::Lagged(n)) => panic!("event receiver lagged by {n}"),
                Err(RecvError::Closed) => panic!("event channel closed"),
            }
        }
    };
    let root_state = tokio::time::timeout(Duration::from_secs(60), collect)
        .await
        .expect("tree transfer timed out");

    assert_eq!(root_state, TransferState::Done);
    assert_eq!(added, TOTAL);
    let at = added_at_first_file_done.expect("a file finished");
    assert!(
        at < TOTAL,
        "first file finished only after all {TOTAL} items were added"
    );

    let snap = engine.snapshot().await;
    assert_eq!(snap.len(), TOTAL);
    assert!(snap.iter().all(|i| i.state == TransferState::Done));
    assert_eq!(
        remote.read_file("/srv/big/d49/f99").as_deref(),
        Some(b"49:99".as_slice())
    );
}

/// `MemFs` whose listings report every entry named `a.txt` twice, like a
/// broken or hostile server.
struct DuplicatingFs(MemFs);

#[async_trait::async_trait]
impl RemoteFs for DuplicatingFs {
    async fn list(&self, p: &RemotePath) -> Result<Vec<RemoteEntry>> {
        let mut out = Vec::new();
        for entry in self.0.list(p).await? {
            if entry.name == "a.txt" {
                out.push(entry.clone());
            }
            out.push(entry);
        }
        Ok(out)
    }
    async fn stat(&self, p: &RemotePath) -> Result<RemoteEntry> {
        self.0.stat(p).await
    }
    async fn open_read(&self, p: &RemotePath, offset: u64) -> Result<BoxAsyncRead> {
        self.0.open_read(p, offset).await
    }
    async fn open_write(&self, p: &RemotePath, offset: u64) -> Result<BoxAsyncWrite> {
        self.0.open_write(p, offset).await
    }
    async fn mkdir(&self, p: &RemotePath) -> Result<()> {
        self.0.mkdir(p).await
    }
    async fn rename(&self, from: &RemotePath, to: &RemotePath) -> Result<()> {
        self.0.rename(from, to).await
    }
    async fn remove_file(&self, p: &RemotePath) -> Result<()> {
        self.0.remove_file(p).await
    }
    async fn remove_dir(&self, p: &RemotePath) -> Result<()> {
        self.0.remove_dir(p).await
    }
    async fn chmod(&self, p: &RemotePath, mode: u32) -> Result<()> {
        self.0.chmod(p, mode).await
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn duplicate_names_in_listing() {
    let local = MemFs::new();
    local.add_file("/w/d/a.txt", &data(50_000), mtime());
    local.add_file("/w/d/b.txt", b"b", mtime());
    let remote = MemFs::new();
    remote.add_dir("/srv");
    let (engine, mut rx) = engine_with(
        Arc::new(DuplicatingFs(local)),
        Arc::new(remote.clone()),
        4,
        EngineConfig::default(),
    )
    .await;

    let root = engine
        .enqueue(spec(Direction::Upload, "/w/d", "/srv/d"))
        .await
        .unwrap();
    match wait_terminal(&mut rx, root).await {
        TransferState::Failed(e) => assert_eq!(e.message, "1 items failed"),
        other => panic!("root ended {other:?}"),
    }

    let snap = engine.snapshot().await;
    let a: Vec<&TransferItem> = snap
        .iter()
        .filter(|i| i.spec.local.as_str() == "/w/d/a.txt")
        .collect();
    assert_eq!(a.len(), 2);
    assert_eq!(a[0].state, TransferState::Done);
    match &a[1].state {
        TransferState::Failed(e) => assert_eq!(
            (e.kind, e.message.as_str()),
            (ErrorKind::Io, "Duplicate name in directory listing")
        ),
        other => panic!("duplicate ended {other:?}"),
    }
    assert_eq!(item(&snap, "/w/d/b.txt").state, TransferState::Done);
    assert_eq!(remote.read_file("/srv/d/a.txt").unwrap(), data(50_000));
}
