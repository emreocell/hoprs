use std::time::{Duration, UNIX_EPOCH};

use hoprs_core::{EntryKind, ErrorKind, RemotePath};
use hoprs_remote::{LocalFs, RemoteFs};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn p(s: &str) -> RemotePath {
    RemotePath::new(s).unwrap()
}

fn setup() -> (tempfile::TempDir, LocalFs) {
    let dir = tempfile::tempdir().unwrap();
    let fs = LocalFs::new(dir.path());
    (dir, fs)
}

async fn write(fs: &LocalFs, path: &str, offset: u64, data: &[u8]) {
    let mut w = fs.open_write(&p(path), offset).await.unwrap();
    w.write_all(data).await.unwrap();
    w.shutdown().await.unwrap();
}

async fn read(fs: &LocalFs, path: &str, offset: u64) -> String {
    let mut s = String::new();
    fs.open_read(&p(path), offset)
        .await
        .unwrap()
        .read_to_string(&mut s)
        .await
        .unwrap();
    s
}

#[tokio::test]
async fn list_read_write_and_errors() {
    let (dir, fs) = setup();
    fs.mkdir(&p("/a")).await.unwrap();
    write(&fs, "/a/b.txt", 0, b"hello").await;
    let list = fs.list(&p("/a")).await.unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].name, "b.txt");
    assert_eq!(list[0].path, p("/a/b.txt"));
    assert_eq!(list[0].kind, EntryKind::File);
    assert_eq!(list[0].size, 5);
    assert_eq!(read(&fs, "/a/b.txt", 2).await, "llo");
    write(&fs, "/a/b.txt", 3, b"p!").await;
    assert_eq!(read(&fs, "/a/b.txt", 0).await, "help!");
    assert_eq!(
        fs.stat(&p("/nope")).await.unwrap_err().kind,
        ErrorKind::NotFound
    );
    assert_eq!(
        fs.list(&p("/nope")).await.unwrap_err().kind,
        ErrorKind::NotFound
    );
    assert_eq!(
        fs.list(&p("/a/b.txt")).await.unwrap_err().kind,
        ErrorKind::Io
    );
    assert_eq!(
        fs.open_read(&p("/a"), 0).await.err().unwrap().kind,
        ErrorKind::Io
    );
    assert_eq!(
        fs.open_write(&p("/a"), 0).await.err().unwrap().kind,
        ErrorKind::Io
    );
    assert_eq!(fs.stat(&p("/")).await.unwrap().kind, EntryKind::Dir);
    drop(dir);
}

#[tokio::test]
async fn open_write_offsets() {
    let (_dir, fs) = setup();
    write(&fs, "/new", 0, b"").await;
    assert_eq!(fs.stat(&p("/new")).await.unwrap().size, 0);
    assert_eq!(
        fs.open_write(&p("/new"), 5).await.err().unwrap().kind,
        ErrorKind::Io
    );
    assert_eq!(
        fs.open_write(&p("/missing"), 1).await.err().unwrap().kind,
        ErrorKind::Io
    );
    write(&fs, "/new", 0, b"abcdef").await;
    write(&fs, "/new", 0, b"xy").await;
    assert_eq!(read(&fs, "/new", 0).await, "xy");
}

#[tokio::test]
async fn mkdir_is_idempotent_and_leaf_only() {
    let (_dir, fs) = setup();
    fs.mkdir(&p("/d")).await.unwrap();
    fs.mkdir(&p("/d")).await.unwrap();
    assert_eq!(fs.stat(&p("/d")).await.unwrap().kind, EntryKind::Dir);
    assert_eq!(
        fs.mkdir(&p("/x/y")).await.unwrap_err().kind,
        ErrorKind::NotFound
    );
}

#[tokio::test]
async fn rename_file_and_dir_subtree() {
    let (_dir, fs) = setup();
    write(&fs, "/a.txt", 0, b"x").await;
    fs.rename(&p("/a.txt"), &p("/b.txt")).await.unwrap();
    assert_eq!(
        fs.stat(&p("/a.txt")).await.unwrap_err().kind,
        ErrorKind::NotFound
    );
    assert_eq!(read(&fs, "/b.txt", 0).await, "x");
    fs.mkdir(&p("/d")).await.unwrap();
    fs.mkdir(&p("/d/e")).await.unwrap();
    write(&fs, "/d/e/f.txt", 0, b"y").await;
    fs.rename(&p("/d"), &p("/g")).await.unwrap();
    assert_eq!(read(&fs, "/g/e/f.txt", 0).await, "y");
    assert_eq!(
        fs.stat(&p("/d")).await.unwrap_err().kind,
        ErrorKind::NotFound
    );
}

#[tokio::test]
async fn remove_dir_non_empty_fails() {
    let (_dir, fs) = setup();
    fs.mkdir(&p("/d")).await.unwrap();
    write(&fs, "/d/f", 0, b"").await;
    assert_eq!(
        fs.remove_dir(&p("/d")).await.unwrap_err().kind,
        ErrorKind::Io
    );
    assert_eq!(
        fs.remove_file(&p("/d")).await.unwrap_err().kind,
        ErrorKind::Io
    );
    fs.remove_file(&p("/d/f")).await.unwrap();
    fs.remove_dir(&p("/d")).await.unwrap();
    assert_eq!(
        fs.remove_file(&p("/d/f")).await.unwrap_err().kind,
        ErrorKind::NotFound
    );
}

#[tokio::test]
async fn set_modified_roundtrips() {
    let (_dir, fs) = setup();
    write(&fs, "/m", 0, b"data").await;
    let t = UNIX_EPOCH + Duration::from_secs(1_000_000);
    fs.set_modified(&p("/m"), t).await.unwrap();
    assert_eq!(fs.stat(&p("/m")).await.unwrap().modified, Some(t));
    assert_eq!(
        fs.set_modified(&p("/none"), t).await.unwrap_err().kind,
        ErrorKind::NotFound
    );
}

#[cfg(unix)]
#[tokio::test]
async fn chmod_and_symlinks() {
    let (dir, fs) = setup();
    write(&fs, "/f", 0, b"x").await;
    fs.chmod(&p("/f"), 0o600).await.unwrap();
    assert_eq!(fs.stat(&p("/f")).await.unwrap().mode, Some(0o600));
    std::os::unix::fs::symlink("f", dir.path().join("link")).unwrap();
    assert_eq!(fs.stat(&p("/link")).await.unwrap().kind, EntryKind::Symlink);
    assert!(
        fs.list(&p("/"))
            .await
            .unwrap()
            .iter()
            .any(|e| e.name == "link" && e.kind == EntryKind::Symlink)
    );
    assert_eq!(
        fs.chmod(&p("/none"), 0o600).await.unwrap_err().kind,
        ErrorKind::NotFound
    );
}
