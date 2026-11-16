//! `compare` between a local temp tree and a tree planted in the containers of
//! `docker/compose.yml`, over SFTP and over FTP (vsftpd has no `MLSD`, so FTP lists with `LIST`
//! and minute precision).

use std::collections::BTreeMap;
use std::fs;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hoprs_core::RemotePath;
use hoprs_remote::testing::ftp_config;
use hoprs_remote::{
    CompareOptions, CompareSignals, FtpFs, FtpPrompter, FtpSecurity, LocalFs, Progress, RemoteFs,
    ScriptedFtpPrompter, SftpConfig, SftpFs, Status, compare,
};
use hoprs_ssh::testing::{docker, fixture, test_config};
use hoprs_ssh::{KeySource, Prompter, ScriptedPrompter, SshSession};
use tokio::sync::{mpsc, watch};

fn p(s: &str) -> RemotePath {
    RemotePath::new(s).unwrap()
}

fn unique() -> String {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{nanos}-{}", COUNTER.fetch_add(1, Ordering::SeqCst))
}

/// Removes a directory in a compose service's container when dropped. Never panics.
struct Cleanup {
    service: &'static str,
    path: String,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        let (service, path) = (self.service, self.path.clone());
        if std::panic::catch_unwind(|| docker(&["exec", "-T", service, "rm", "-rf", &path]))
            .is_err()
        {
            eprintln!("could not remove {path} in {service}");
        }
    }
}

/// A modification time a day ago, on a minute boundary, so `LIST` shows it exactly.
///
/// Independent of the host's time zone: vsftpd lists in GMT and the test's expected times are
/// absolute (`SystemTime`), and a day back stays inside the 6-month window in which `LIST`
/// shows a time of day rather than only a date.
fn base_time() -> SystemTime {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        - 86_400;
    UNIX_EPOCH + Duration::from_secs(secs - secs % 60)
}

fn write_local(root: &std::path::Path, rel: &str, bytes: &[u8], mtime: SystemTime) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, bytes).unwrap();
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(mtime)
        .unwrap();
}

/// Plants the same tree on the server (by shell, as `user` inside `work`) and locally:
///
/// - `same.txt`, `sub/deep.txt`: identical on both sides;
/// - `only-local.txt`, `sub/only-local-too.txt`: local only;
/// - `only-remote.txt`: remote only; `remote-dir/` (with a file): remote folder only;
/// - `size.txt`: 5 bytes locally, 8 remotely.
fn plant(service: &str, user: &str, work: &str, local: &std::path::Path) -> SystemTime {
    let t = base_time();
    let epoch = t.duration_since(UNIX_EPOCH).unwrap().as_secs();
    let script = format!(
        "cd {work} && mkdir -p sub remote-dir && printf same > same.txt && printf deep > sub/deep.txt \
         && printf remote > only-remote.txt && printf eightbyt > size.txt && printf x > remote-dir/f \
         && touch -d @{epoch} same.txt sub/deep.txt size.txt only-remote.txt"
    );
    docker(&["exec", "-T", service, "su", user, "-c", &script]);
    write_local(local, "same.txt", b"same", t);
    write_local(local, "sub/deep.txt", b"deep", t);
    write_local(local, "size.txt", b"five!", t);
    write_local(local, "only-local.txt", b"local", t);
    write_local(local, "sub/only-local-too.txt", b"local", t);
    t
}

async fn run(
    local_dir: &std::path::Path,
    remote: Arc<dyn RemoteFs>,
    remote_root: RemotePath,
) -> (BTreeMap<String, Status>, u64) {
    let local: Arc<dyn RemoteFs> = Arc::new(LocalFs::new(local_dir));
    let (tx, mut rx) = mpsc::unbounded_channel();
    let (_resume, resume_rx) = watch::channel(true);
    let (_stop, stop_rx) = watch::channel(false);
    let summary = compare(
        local,
        RemotePath::root(),
        remote,
        remote_root,
        CompareOptions {
            concurrency_a: 1,
            concurrency_b: 1,
            ..CompareOptions::default()
        },
        tx,
        CompareSignals {
            resume: resume_rx,
            stop: stop_rx,
        },
    )
    .await
    .unwrap();
    let mut found = BTreeMap::new();
    while let Ok(msg) = rx.try_recv() {
        if let Progress::Found(d) = msg {
            found.insert(d.path, d.status);
        }
    }
    assert!(!summary.partial);
    (found, summary.identical)
}

fn expected() -> BTreeMap<String, Status> {
    BTreeMap::from([
        ("only-local.txt".to_owned(), Status::OnlyLocal),
        ("only-remote.txt".to_owned(), Status::OnlyRemote),
        ("remote-dir".to_owned(), Status::OnlyRemote),
        ("size.txt".to_owned(), Status::SizeDiffers),
        ("sub/only-local-too.txt".to_owned(), Status::OnlyLocal),
    ])
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d --build --wait"]
async fn compare_over_sftp() {
    let known_hosts = tempfile::tempdir().unwrap();
    let mut cfg = test_config("alice", known_hosts.path());
    cfg.auth.key = KeySource::File {
        path: fixture("id_ed25519"),
        passphrase: None,
    };
    let prompter = Arc::new(ScriptedPrompter::new().accept_host(true));
    let session = SshSession::connect(cfg, prompter as Arc<dyn Prompter>)
        .await
        .unwrap();
    let sftp = SftpFs::new(session, SftpConfig::default()).await.unwrap();
    let work = format!("/home/alice/cmp-{}", unique());
    let _cleanup = Cleanup {
        service: "sshd",
        path: work.clone(),
    };
    sftp.mkdir(&p(&work)).await.unwrap();
    let local = tempfile::tempdir().unwrap();
    let t = plant("sshd", "alice", &work, local.path());
    // One second apart is inside the 2 s tolerance: still identical.
    write_local(
        local.path(),
        "same.txt",
        b"same",
        t + Duration::from_secs(1),
    );

    let (found, identical) = run(local.path(), Arc::new(sftp), p(&work)).await;
    assert_eq!(found, expected());
    assert_eq!(identical, 2);
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d --build --wait"]
async fn compare_over_ftp_list() {
    let prompter = Arc::new(ScriptedFtpPrompter::new());
    let ftp = FtpFs::connect(
        ftp_config(FtpSecurity::Plain),
        prompter as Arc<dyn FtpPrompter>,
    )
    .await
    .unwrap();
    let work = format!("/home/frank/cmp-{}", unique());
    let _cleanup = Cleanup {
        service: "ftp",
        path: work.clone(),
    };
    ftp.mkdir(&p(&work)).await.unwrap();
    let local = tempfile::tempdir().unwrap();
    let t = plant("ftp", "frank", &work, local.path());
    // 30 s apart is inside the minute `LIST` can resolve: identical over FTP.
    write_local(
        local.path(),
        "same.txt",
        b"same",
        t + Duration::from_secs(30),
    );

    let ftp = Arc::new(ftp);
    let (found, identical) = run(local.path(), ftp.clone(), p(&work)).await;
    assert_eq!(found, expected());
    assert_eq!(identical, 2);
    // vsftpd 3.0.3 has no MLST, so the listings came from `LIST`: minute precision.
    assert_eq!(ftp.mtime_precision(), Duration::from_secs(60));
}
