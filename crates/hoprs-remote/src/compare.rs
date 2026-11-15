//! Recursive comparison of two directory trees on any pair of [`RemoteFs`] backends.
//!
//! The walk goes folder by folder from the two roots, pairing children by name (case-sensitive).
//! A folder on one side only is reported once and never listed. Cancellation is dropping the
//! future: nothing more is listed after that.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use futures_util::stream::FuturesUnordered;
use hoprs_core::{EntryKind, Error, RemoteEntry, RemotePath, Result};
use tokio::sync::{Semaphore, mpsc, watch};

use crate::RemoteFs;

/// A path relative to the compared roots: `/`-separated, no leading slash.
pub type RelPath = String;

/// Which side of the comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompareSide {
    Local,
    Remote,
}

/// How an entry differs between the two sides.
#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    OnlyLocal,
    OnlyRemote,
    LocalNewer,
    RemoteNewer,
    SizeDiffers,
    /// A file on one side, a folder (or symlink) on the other.
    KindDiffers,
    /// The folder could not be listed on `side`; its contents were not compared.
    Unreadable {
        side: CompareSide,
        error: Error,
    },
}

/// One entry that is not the same on both sides.
#[derive(Debug, Clone, PartialEq)]
pub struct Difference {
    pub path: RelPath,
    /// The entry's kind (the local one when both sides have the entry).
    pub kind: EntryKind,
    pub status: Status,
    pub local: Option<RemoteEntry>,
    pub remote: Option<RemoteEntry>,
    /// Entries inside a one-sided folder, when known. A one-sided folder is never listed, so
    /// the walk leaves this `None`.
    pub entries_inside: Option<u64>,
}

/// What a run reports while it walks.
#[derive(Debug, Clone, PartialEq)]
// `Found` is the common message; boxing it would only add an allocation per difference.
#[allow(clippy::large_enum_variant)]
pub enum Progress {
    /// Entries seen so far per side (ignored names excluded); sent at least every 200 entries
    /// and once at the end.
    Seen {
        local: usize,
        remote: usize,
    },
    Found(Difference),
    /// A side reached [`CompareOptions::limit`]; the walk waits for `resume` to turn `true`.
    LimitReached {
        local: usize,
        remote: usize,
    },
}

/// The result of a finished run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    /// Entries (files and symlinks) that are the same on both sides.
    pub identical: u64,
    /// Number of [`Progress::Found`] sent.
    pub differences: usize,
    /// The run stopped early (the progress receiver or the resume sender went away).
    pub partial: bool,
    /// The run ended because the `stop` signal turned `true` (always with `partial`): no new
    /// listing started after it, listings in flight finished and were discarded.
    pub stopped: bool,
}

/// The two ways to steer a running [`compare`].
#[derive(Debug, Clone)]
pub struct CompareSignals {
    /// Turns `true` to continue after [`Progress::LimitReached`] (and to lift the limit).
    pub resume: watch::Receiver<bool>,
    /// Turns `true` to stop cooperatively: no new listing starts, the one in flight
    /// finishes, and the call returns `Ok` with [`Summary::stopped`] set.
    pub stop: watch::Receiver<bool>,
}

#[derive(Debug, Clone, Default)]
pub struct CompareOptions {
    /// Names to skip on both sides at any depth (`*` matches any run of characters).
    pub ignore: Vec<String>,
    /// Entries per side after which the walk pauses; `None` for no limit.
    pub limit: Option<usize>,
    /// Concurrent listings on the local (`a`) side; 0 counts as 1.
    pub concurrency_a: usize,
    /// Concurrent listings on the remote (`b`) side; 0 counts as 1.
    pub concurrency_b: usize,
    /// Files the app transferred this session: relative path -> (local size, remote size) after
    /// the transfer. A file whose sizes still match is identical even when the mtimes differ
    /// (servers that cannot set mtimes).
    pub known_synced: HashMap<RelPath, (u64, u64)>,
}

/// Whether `name` matches one of `patterns` (`*` matches any run of characters; the rest is
/// literal, case-sensitive).
pub fn ignored(name: &str, patterns: &[String]) -> bool {
    patterns.iter().any(|p| wildcard(p, name))
}

fn wildcard(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    let (mut pi, mut ni) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while ni < n.len() {
        if pi < p.len() && p[pi] == '*' {
            star = Some((pi, ni));
            pi += 1;
        } else if pi < p.len() && p[pi] == n[ni] {
            pi += 1;
            ni += 1;
        } else if let Some((sp, sn)) = star {
            pi = sp + 1;
            ni = sn + 1;
            star = Some((sp, sn + 1));
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

struct Pair {
    rel: RelPath,
    local: RemotePath,
    remote: RemotePath,
}

struct Listed {
    pair: Pair,
    local: Result<Vec<RemoteEntry>>,
    remote: Result<Vec<RemoteEntry>>,
}

async fn list_one(
    fs: &dyn RemoteFs,
    sem: &Semaphore,
    path: &RemotePath,
) -> Result<Vec<RemoteEntry>> {
    let _permit = sem.acquire().await;
    fs.list(path).await
}

async fn list_pair(
    local: &Arc<dyn RemoteFs>,
    local_sem: &Semaphore,
    remote: &Arc<dyn RemoteFs>,
    remote_sem: &Semaphore,
    pair: Pair,
) -> Listed {
    let (l, r) = tokio::join!(
        list_one(local.as_ref(), local_sem, &pair.local),
        list_one(remote.as_ref(), remote_sem, &pair.remote),
    );
    Listed {
        pair,
        local: l,
        remote: r,
    }
}

fn join_rel(parent: &str, name: &str) -> RelPath {
    if parent.is_empty() {
        name.to_owned()
    } else {
        format!("{parent}/{name}")
    }
}

/// Walks both trees and reports every difference through `progress`.
///
/// Listing errors on a root fail the call; on a subfolder they become [`Status::Unreadable`]
/// differences and the walk goes on. When a side reaches `options.limit` the walk sends
/// [`Progress::LimitReached`] and waits until `resume` turns `true`, then continues without a
/// limit (a `resume` that is `true` from the start means no limit).
///
/// `stop` is the cooperative cancel: once it turns `true` no new listing starts, listings in
/// flight finish (dropping a listing midway can force an FTP session to reconnect) and their
/// results are discarded, and the call returns `Ok` with [`Summary::stopped`] set. A dropped
/// `stop` sender never stops the run. Dropping the returned future also cancels the run, but
/// abruptly: use it only when the session is going away.
pub async fn compare(
    local: Arc<dyn RemoteFs>,
    local_root: RemotePath,
    remote: Arc<dyn RemoteFs>,
    remote_root: RemotePath,
    options: CompareOptions,
    progress: mpsc::UnboundedSender<Progress>,
    signals: CompareSignals,
) -> Result<Summary> {
    let CompareSignals {
        mut resume,
        mut stop,
    } = signals;
    let local_sem = Semaphore::new(options.concurrency_a.max(1));
    let remote_sem = Semaphore::new(options.concurrency_b.max(1));
    let width = options.concurrency_a.max(options.concurrency_b).max(1);
    let mut limit = if *resume.borrow() {
        None
    } else {
        options.limit
    };

    let mut summary = Summary {
        identical: 0,
        differences: 0,
        partial: false,
        stopped: false,
    };
    let (mut seen_l, mut seen_r) = (0usize, 0usize);
    let mut last_seen_sent = 0usize;
    let mut pending: VecDeque<Pair> = VecDeque::new();
    pending.push_back(Pair {
        rel: String::new(),
        local: local_root,
        remote: remote_root,
    });
    let mut inflight = FuturesUnordered::new();
    let mut paused = false;

    // Sends a progress message; `false` when nobody listens any more.
    let send = |p: Progress| progress.send(p).is_ok();

    loop {
        if *stop.borrow() {
            return stopped(summary, &mut inflight).await;
        }
        while !paused && inflight.len() < width {
            let Some(pair) = pending.pop_front() else {
                break;
            };
            inflight.push(list_pair(&local, &local_sem, &remote, &remote_sem, pair));
        }
        let Some(listed) = inflight.next().await else {
            if paused && !pending.is_empty() {
                if !send(Progress::LimitReached {
                    local: seen_l,
                    remote: seen_r,
                }) {
                    summary.partial = true;
                    return Ok(summary);
                }
                let stop_signal = async {
                    if stop.wait_for(|v| *v).await.is_err() {
                        std::future::pending::<()>().await;
                    }
                };
                // `Some(resumed)` or `None` for a stop; the guard is not held across an await.
                let outcome = tokio::select! {
                    resumed = resume.wait_for(|v| *v) => Some(resumed.is_ok()),
                    () = stop_signal => None,
                };
                match outcome {
                    Some(true) => {}
                    Some(false) => {
                        summary.partial = true;
                        return Ok(summary);
                    }
                    None => return stopped(summary, &mut inflight).await,
                }
                limit = None;
                paused = false;
                continue;
            }
            break;
        };

        if *stop.borrow() {
            // Finished after the stop: discarded.
            return stopped(summary, &mut inflight).await;
        }
        let Listed {
            pair,
            local: l,
            remote: r,
        } = listed;
        let is_root = pair.rel.is_empty();
        let (l, r) = match (l, r) {
            (Err(e), _) | (_, Err(e)) if is_root => return Err(e),
            (Err(error), _) => {
                summary.differences += 1;
                if !send(Progress::Found(unreadable(
                    &pair,
                    CompareSide::Local,
                    error,
                ))) {
                    summary.partial = true;
                    return Ok(summary);
                }
                continue;
            }
            (Ok(_), Err(error)) => {
                summary.differences += 1;
                if !send(Progress::Found(unreadable(
                    &pair,
                    CompareSide::Remote,
                    error,
                ))) {
                    summary.partial = true;
                    return Ok(summary);
                }
                continue;
            }
            (Ok(l), Ok(r)) => (l, r),
        };

        let keep = |e: &RemoteEntry| !ignored(&e.name, &options.ignore);
        let lmap: BTreeMap<String, RemoteEntry> = l
            .into_iter()
            .filter(keep)
            .map(|e| (e.name.clone(), e))
            .collect();
        let mut rmap: BTreeMap<String, RemoteEntry> = r
            .into_iter()
            .filter(keep)
            .map(|e| (e.name.clone(), e))
            .collect();
        seen_l += lmap.len();
        seen_r += rmap.len();

        let mut found: Vec<Difference> = Vec::new();
        for (name, le) in lmap {
            let rel = join_rel(&pair.rel, &name);
            let Some(re) = rmap.remove(&name) else {
                found.push(Difference {
                    path: rel,
                    kind: le.kind,
                    status: Status::OnlyLocal,
                    local: Some(le),
                    remote: None,
                    entries_inside: None,
                });
                continue;
            };
            match (le.kind, re.kind) {
                (EntryKind::Dir, EntryKind::Dir) => pending.push_back(Pair {
                    local: pair.local.join(&name),
                    remote: pair.remote.join(&name),
                    rel,
                }),
                (EntryKind::Symlink, EntryKind::Symlink) => summary.identical += 1,
                (EntryKind::File, EntryKind::File) => {
                    match file_status(&le, &re, &rel, &options, local.as_ref(), remote.as_ref()) {
                        None => summary.identical += 1,
                        Some(status) => found.push(Difference {
                            path: rel,
                            kind: EntryKind::File,
                            status,
                            local: Some(le),
                            remote: Some(re),
                            entries_inside: None,
                        }),
                    }
                }
                _ => found.push(Difference {
                    path: rel,
                    kind: le.kind,
                    status: Status::KindDiffers,
                    local: Some(le),
                    remote: Some(re),
                    entries_inside: None,
                }),
            }
        }
        for (name, re) in rmap {
            found.push(Difference {
                path: join_rel(&pair.rel, &name),
                kind: re.kind,
                status: Status::OnlyRemote,
                local: None,
                remote: Some(re),
                entries_inside: None,
            });
        }
        for d in found {
            summary.differences += 1;
            if !send(Progress::Found(d)) {
                summary.partial = true;
                return Ok(summary);
            }
        }

        if seen_l + seen_r - last_seen_sent >= 200 {
            last_seen_sent = seen_l + seen_r;
            if !send(Progress::Seen {
                local: seen_l,
                remote: seen_r,
            }) {
                summary.partial = true;
                return Ok(summary);
            }
        }
        if limit.is_some_and(|n| seen_l >= n || seen_r >= n) {
            paused = true;
        }
    }

    let _ = send(Progress::Seen {
        local: seen_l,
        remote: seen_r,
    });
    Ok(summary)
}

/// Ends a stopped run: lets the listings in flight finish (never drops one midway) and
/// discards what they return.
async fn stopped<F: std::future::Future>(
    mut summary: Summary,
    inflight: &mut FuturesUnordered<F>,
) -> Result<Summary> {
    while inflight.next().await.is_some() {}
    summary.partial = true;
    summary.stopped = true;
    Ok(summary)
}

fn unreadable(pair: &Pair, side: CompareSide, error: Error) -> Difference {
    Difference {
        path: pair.rel.clone(),
        kind: EntryKind::Dir,
        status: Status::Unreadable { side, error },
        local: None,
        remote: None,
        entries_inside: None,
    }
}

/// `None` when the two files are the same.
fn file_status(
    l: &RemoteEntry,
    r: &RemoteEntry,
    rel: &str,
    options: &CompareOptions,
    local: &dyn RemoteFs,
    remote: &dyn RemoteFs,
) -> Option<Status> {
    if options.known_synced.get(rel) == Some(&(l.size, r.size)) {
        return None;
    }
    if let (Some(lm), Some(rm)) = (l.modified, r.modified) {
        let tolerance = Duration::from_secs(2)
            .max(local.mtime_precision())
            .max(remote.mtime_precision());
        match lm.duration_since(rm) {
            Ok(d) if d > tolerance => return Some(Status::LocalNewer),
            Err(e) if e.duration() > tolerance => return Some(Status::RemoteNewer),
            _ => {}
        }
    }
    (l.size != r.size).then_some(Status::SizeDiffers)
}

#[cfg(all(test, feature = "testing"))]
mod tests {
    use super::*;
    use crate::{BoxAsyncRead, BoxAsyncWrite, LocalFs, MemFs};
    use async_trait::async_trait;
    use hoprs_core::ErrorKind;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn t(secs: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(1_000_000 + secs)
    }

    fn p(s: &str) -> RemotePath {
        RemotePath::new(s).unwrap()
    }

    /// Wraps a `MemFs`, counting and recording listings and optionally slowing them down.
    struct Probe {
        inner: MemFs,
        precision: Duration,
        delay: Duration,
        listed: Mutex<Vec<String>>,
        active: AtomicUsize,
        overlapped: AtomicUsize,
        fail: Vec<String>,
    }

    impl Probe {
        fn new(inner: MemFs) -> Probe {
            Probe {
                inner,
                precision: Duration::from_secs(1),
                delay: Duration::ZERO,
                listed: Mutex::new(Vec::new()),
                active: AtomicUsize::new(0),
                overlapped: AtomicUsize::new(0),
                fail: Vec::new(),
            }
        }
        fn listings(&self) -> Vec<String> {
            self.listed.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl RemoteFs for Probe {
        async fn list(&self, path: &RemotePath) -> Result<Vec<RemoteEntry>> {
            if self.active.fetch_add(1, Ordering::SeqCst) > 0 {
                self.overlapped.fetch_add(1, Ordering::SeqCst);
            }
            self.listed.lock().unwrap().push(path.to_string());
            if !self.delay.is_zero() {
                tokio::time::sleep(self.delay).await;
            }
            self.active.fetch_sub(1, Ordering::SeqCst);
            if self.fail.iter().any(|f| f == path.as_str()) {
                return Err(Error::new(ErrorKind::Permission, "denied"));
            }
            self.inner.list(path).await
        }
        async fn stat(&self, path: &RemotePath) -> Result<RemoteEntry> {
            self.inner.stat(path).await
        }
        async fn open_read(&self, path: &RemotePath, offset: u64) -> Result<BoxAsyncRead> {
            self.inner.open_read(path, offset).await
        }
        async fn open_write(&self, path: &RemotePath, offset: u64) -> Result<BoxAsyncWrite> {
            self.inner.open_write(path, offset).await
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
        fn mtime_precision(&self) -> Duration {
            self.precision
        }
    }

    fn opts() -> CompareOptions {
        CompareOptions {
            concurrency_a: 4,
            concurrency_b: 4,
            ..Default::default()
        }
    }

    struct Run {
        found: Vec<Difference>,
        summary: Summary,
        events: Vec<Progress>,
    }

    async fn run(
        a: Arc<dyn RemoteFs>,
        b: Arc<dyn RemoteFs>,
        options: CompareOptions,
    ) -> Result<Run> {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (_resume_tx, resume) = watch::channel(false);
        let (_stop_tx, stop) = watch::channel(false);
        let summary = compare(
            a,
            p("/"),
            b,
            p("/"),
            options,
            tx,
            CompareSignals { resume, stop },
        )
        .await?;
        let mut events = Vec::new();
        while let Ok(e) = rx.try_recv() {
            events.push(e);
        }
        let mut found: Vec<Difference> = events
            .iter()
            .filter_map(|e| match e {
                Progress::Found(d) => Some(d.clone()),
                _ => None,
            })
            .collect();
        found.sort_by(|x, y| x.path.cmp(&y.path));
        Ok(Run {
            found,
            summary,
            events,
        })
    }

    fn statuses(r: &Run) -> Vec<(&str, &Status)> {
        r.found
            .iter()
            .map(|d| (d.path.as_str(), &d.status))
            .collect()
    }

    #[tokio::test]
    async fn only_local_and_only_remote() {
        let (a, b) = (MemFs::new(), MemFs::new());
        a.add_file("/same", b"x", t(0));
        b.add_file("/same", b"x", t(0));
        a.add_file("/l", b"x", t(0));
        b.add_file("/r", b"x", t(0));
        let r = run(Arc::new(a), Arc::new(b), opts()).await.unwrap();
        assert_eq!(
            statuses(&r),
            vec![("l", &Status::OnlyLocal), ("r", &Status::OnlyRemote)]
        );
        assert_eq!(r.summary.identical, 1);
        assert_eq!(r.summary.differences, 2);
        assert!(!r.summary.partial);
    }

    #[tokio::test]
    async fn size_differs() {
        let (a, b) = (MemFs::new(), MemFs::new());
        a.add_file("/d/f", b"abc", t(0));
        b.add_file("/d/f", b"abcd", t(0));
        let r = run(Arc::new(a), Arc::new(b), opts()).await.unwrap();
        assert_eq!(statuses(&r), vec![("d/f", &Status::SizeDiffers)]);
    }

    #[tokio::test]
    async fn local_newer_and_remote_newer() {
        let (a, b) = (MemFs::new(), MemFs::new());
        a.add_file("/ln", b"x", t(10));
        b.add_file("/ln", b"x", t(0));
        a.add_file("/rn", b"x", t(0));
        b.add_file("/rn", b"x", t(10));
        let r = run(Arc::new(a), Arc::new(b), opts()).await.unwrap();
        assert_eq!(
            statuses(&r),
            vec![("ln", &Status::LocalNewer), ("rn", &Status::RemoteNewer)]
        );
    }

    #[tokio::test]
    async fn within_tolerance_is_identical() {
        let (a, b) = (MemFs::new(), MemFs::new());
        a.add_file("/f", b"x", t(0) + Duration::from_millis(1500));
        b.add_file("/f", b"x", t(0));
        let r = run(Arc::new(a), Arc::new(b), opts()).await.unwrap();
        assert!(r.found.is_empty());
        assert_eq!(r.summary.identical, 1);
    }

    #[tokio::test]
    async fn minute_precision_widens_tolerance() {
        let (a, b) = (MemFs::new(), MemFs::new());
        a.add_file("/f", b"x", t(45));
        b.add_file("/f", b"x", t(0));
        let mut remote = Probe::new(b);
        remote.precision = Duration::from_secs(60);
        let r = run(Arc::new(a), Arc::new(remote), opts()).await.unwrap();
        assert!(r.found.is_empty());
    }

    #[tokio::test]
    async fn missing_mtime_compares_size_only() {
        // `MemFs` always has times, so a double that drops them.
        struct NoTimes(MemFs);
        #[async_trait]
        impl RemoteFs for NoTimes {
            async fn list(&self, path: &RemotePath) -> Result<Vec<RemoteEntry>> {
                let mut v = self.0.list(path).await?;
                v.iter_mut().for_each(|e| e.modified = None);
                Ok(v)
            }
            async fn stat(&self, path: &RemotePath) -> Result<RemoteEntry> {
                self.0.stat(path).await
            }
            async fn open_read(&self, path: &RemotePath, o: u64) -> Result<BoxAsyncRead> {
                self.0.open_read(path, o).await
            }
            async fn open_write(&self, path: &RemotePath, o: u64) -> Result<BoxAsyncWrite> {
                self.0.open_write(path, o).await
            }
            async fn mkdir(&self, path: &RemotePath) -> Result<()> {
                self.0.mkdir(path).await
            }
            async fn rename(&self, f: &RemotePath, to: &RemotePath) -> Result<()> {
                self.0.rename(f, to).await
            }
            async fn remove_file(&self, path: &RemotePath) -> Result<()> {
                self.0.remove_file(path).await
            }
            async fn remove_dir(&self, path: &RemotePath) -> Result<()> {
                self.0.remove_dir(path).await
            }
            async fn chmod(&self, path: &RemotePath, m: u32) -> Result<()> {
                self.0.chmod(path, m).await
            }
        }
        let (a, b) = (MemFs::new(), MemFs::new());
        a.add_file("/same", b"x", t(0));
        b.add_file("/same", b"x", t(500));
        a.add_file("/size", b"x", t(0));
        b.add_file("/size", b"xx", t(0));
        let r = run(Arc::new(NoTimes(a)), Arc::new(b), opts())
            .await
            .unwrap();
        assert_eq!(statuses(&r), vec![("size", &Status::SizeDiffers)]);
    }

    #[tokio::test]
    async fn one_sided_folder_reported_once_not_walked() {
        let (a, b) = (MemFs::new(), MemFs::new());
        a.add_file("/big/a", b"x", t(0));
        a.add_file("/big/sub/b", b"x", t(0));
        b.add_dir("/other");
        let local = Arc::new(Probe::new(a));
        let r = run(local.clone(), Arc::new(b), opts()).await.unwrap();
        assert_eq!(
            statuses(&r),
            vec![("big", &Status::OnlyLocal), ("other", &Status::OnlyRemote)]
        );
        assert_eq!(r.found[0].kind, EntryKind::Dir);
        assert_eq!(local.listings(), vec!["/".to_string()]);
    }

    #[tokio::test]
    async fn file_vs_folder_is_kind_differs() {
        let (a, b) = (MemFs::new(), MemFs::new());
        a.add_file("/x", b"x", t(0));
        b.add_file("/x/inner", b"x", t(0));
        let r = run(Arc::new(a), Arc::new(b), opts()).await.unwrap();
        assert_eq!(statuses(&r), vec![("x", &Status::KindDiffers)]);
        assert_eq!(r.found[0].kind, EntryKind::File);
    }

    #[tokio::test]
    async fn symlink_by_name_only() {
        let (a, b) = (MemFs::new(), MemFs::new());
        a.add_symlink("/both", "/one");
        b.add_symlink("/both", "/two");
        a.add_symlink("/only", "/one");
        let r = run(Arc::new(a), Arc::new(b), opts()).await.unwrap();
        assert_eq!(statuses(&r), vec![("only", &Status::OnlyLocal)]);
        assert_eq!(r.found[0].kind, EntryKind::Symlink);
        assert_eq!(r.summary.identical, 1);
    }

    #[tokio::test]
    async fn ignored_at_depth_and_wildcard() {
        let (a, b) = (MemFs::new(), MemFs::new());
        a.add_file("/src/node_modules/x/y", b"x", t(0));
        a.add_file("/src/app.log", b"x", t(0));
        b.add_file("/src/other.log", b"x", t(0));
        a.add_file("/src/keep", b"x", t(0));
        b.add_dir("/src/node_modules");
        let o = CompareOptions {
            ignore: vec!["node_modules".into(), "*.log".into()],
            ..opts()
        };
        let local = Arc::new(Probe::new(a));
        let r = run(local.clone(), Arc::new(b), o).await.unwrap();
        assert_eq!(statuses(&r), vec![("src/keep", &Status::OnlyLocal)]);
        assert!(!local.listings().iter().any(|l| l.contains("node_modules")));
    }

    #[test]
    fn ignored_patterns() {
        let pats = |p: &[&str]| p.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(ignored("x.log", &pats(&["*.log"])));
        assert!(!ignored("x.logs", &pats(&["*.log"])));
        assert!(ignored("axxb", &pats(&["a*b"])));
        assert!(ignored("ab", &pats(&["a*b"])));
        assert!(!ignored("abc", &pats(&["a*b"])));
        assert!(ignored(".git", &pats(&[".git"])));
        assert!(!ignored("a.git", &pats(&[".git"])));
        assert!(!ignored("anything", &[]));
        assert!(ignored("anything", &pats(&["*"])));
    }

    #[tokio::test]
    async fn limit_pauses_then_continues() {
        let (a, b) = (MemFs::new(), MemFs::new());
        for s in [&a, &b] {
            s.add_file("/d1/f", b"x", t(0));
            s.add_file("/d2/f", b"x", t(0));
            s.add_file("/top", b"x", t(0));
        }
        a.add_file("/d2/extra", b"x", t(0));
        let (la, lb) = (Arc::new(Probe::new(a)), Arc::new(Probe::new(b)));
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (resume_tx, resume) = watch::channel(false);
        let (_stop_tx, stop) = watch::channel(false);
        let o = CompareOptions {
            limit: Some(3),
            concurrency_a: 1,
            concurrency_b: 1,
            ..Default::default()
        };
        let (a2, b2) = (la.clone(), lb.clone());
        let task = tokio::spawn(async move {
            compare(
                a2,
                p("/"),
                b2,
                p("/"),
                o,
                tx,
                CompareSignals { resume, stop },
            )
            .await
        });
        loop {
            if let Progress::LimitReached { local, remote } = rx.recv().await.unwrap() {
                assert_eq!((local, remote), (3, 3));
                break;
            }
        }
        for _ in 0..20 {
            tokio::task::yield_now().await;
        }
        assert_eq!(la.listings(), vec!["/".to_string()]);
        assert!(!task.is_finished());
        resume_tx.send(true).unwrap();
        let summary = task.await.unwrap().unwrap();
        assert!(!summary.partial);
        assert_eq!(summary.differences, 1);
        assert_eq!(la.listings().len(), 3);
    }

    #[tokio::test(start_paused = true)]
    async fn dropping_the_future_stops_listing() {
        let (a, b) = (MemFs::new(), MemFs::new());
        for s in [&a, &b] {
            s.add_file("/a/b/c/d/e/f", b"x", t(0));
        }
        let mut pa = Probe::new(a);
        pa.delay = Duration::from_secs(1);
        let pa = Arc::new(pa);
        let (tx, _rx) = mpsc::unbounded_channel();
        let (_rt, resume) = watch::channel(false);
        let (_st, stop) = watch::channel(false);
        {
            let fut = compare(
                pa.clone(),
                p("/"),
                Arc::new(b),
                p("/"),
                opts(),
                tx,
                CompareSignals { resume, stop },
            );
            let _ = tokio::time::timeout(Duration::from_millis(2500), fut).await;
        }
        let seen = pa.listings().len();
        assert!((2..6).contains(&seen), "{seen}");
        tokio::time::sleep(Duration::from_secs(30)).await;
        assert_eq!(pa.listings().len(), seen);
    }

    #[tokio::test(start_paused = true)]
    async fn stop_lets_the_inflight_listing_finish_and_starts_no_more() {
        let (a, b) = (MemFs::new(), MemFs::new());
        for s in [&a, &b] {
            s.add_file("/d1/f", b"x", t(0));
            s.add_file("/d2/f", b"x", t(0));
            s.add_file("/d3/f", b"x", t(0));
        }
        a.add_file("/d1/extra", b"x", t(0));
        let mut pa = Probe::new(a);
        pa.delay = Duration::from_secs(1);
        let (pa, pb) = (Arc::new(pa), Arc::new(Probe::new(b)));
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (_resume_tx, resume) = watch::channel(false);
        let (stop_tx, stop) = watch::channel(false);
        let o = CompareOptions {
            concurrency_a: 1,
            concurrency_b: 1,
            ..Default::default()
        };
        let (a2, b2) = (pa.clone(), pb.clone());
        let task = tokio::spawn(async move {
            compare(
                a2,
                p("/"),
                b2,
                p("/"),
                o,
                tx,
                CompareSignals { resume, stop },
            )
            .await
        });
        // The root listing is in flight (it takes 1 s).
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert_eq!(pa.listings(), vec!["/".to_string()]);
        stop_tx.send(true).unwrap();
        let summary = task.await.unwrap().unwrap();
        assert!(summary.stopped && summary.partial);
        // The in-flight listing completed (not dropped) and nothing started after it.
        assert_eq!(pa.listings(), vec!["/".to_string()]);
        assert_eq!(pa.active.load(Ordering::SeqCst), 0);
        // Its result was discarded.
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn stop_while_paused_returns() {
        let (a, b) = (MemFs::new(), MemFs::new());
        for s in [&a, &b] {
            s.add_file("/d1/f", b"x", t(0));
            s.add_file("/top", b"x", t(0));
        }
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (_resume_tx, resume) = watch::channel(false);
        let (stop_tx, stop) = watch::channel(false);
        let o = CompareOptions {
            limit: Some(2),
            concurrency_a: 1,
            concurrency_b: 1,
            ..Default::default()
        };
        let task = tokio::spawn(async move {
            compare(
                Arc::new(a),
                p("/"),
                Arc::new(b),
                p("/"),
                o,
                tx,
                CompareSignals { resume, stop },
            )
            .await
        });
        loop {
            if let Progress::LimitReached { .. } = rx.recv().await.unwrap() {
                break;
            }
        }
        stop_tx.send(true).unwrap();
        let summary = task.await.unwrap().unwrap();
        assert!(summary.stopped);
    }

    #[tokio::test]
    async fn unreadable_subfolder_reported_and_walk_continues() {
        let (a, b) = (MemFs::new(), MemFs::new());
        for s in [&a, &b] {
            s.add_file("/bad/f", b"x", t(0));
            s.add_file("/good/f", b"x", t(0));
        }
        a.add_file("/good/only", b"x", t(0));
        let mut pa = Probe::new(a);
        pa.fail = vec!["/bad".into()];
        let r = run(Arc::new(pa), Arc::new(b), opts()).await.unwrap();
        assert_eq!(r.found.len(), 2);
        assert_eq!(r.found[0].path, "bad");
        match &r.found[0].status {
            Status::Unreadable { side, error } => {
                assert_eq!(*side, CompareSide::Local);
                assert_eq!(error.kind, ErrorKind::Permission);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(r.found[1].path, "good/only");
    }

    #[tokio::test]
    async fn root_error_fails_the_call() {
        let (a, b) = (MemFs::new(), MemFs::new());
        let mut pa = Probe::new(a);
        pa.fail = vec!["/".into()];
        let e = run(Arc::new(pa), Arc::new(b), opts()).await.err().unwrap();
        assert_eq!(e.kind, ErrorKind::Permission);
    }

    #[tokio::test(start_paused = true)]
    async fn one_listing_at_a_time_with_concurrency_1() {
        let (a, b) = (MemFs::new(), MemFs::new());
        for s in [&a, &b] {
            for d in ["x", "y", "z"] {
                s.add_file(&format!("/{d}/f"), b"x", t(0));
            }
        }
        let (mut pa, mut pb) = (Probe::new(a), Probe::new(b));
        pa.delay = Duration::from_millis(10);
        pb.delay = Duration::from_millis(10);
        let (pa, pb) = (Arc::new(pa), Arc::new(pb));
        let o = CompareOptions {
            concurrency_a: 1,
            concurrency_b: 1,
            ..Default::default()
        };
        let r = run(pa.clone(), pb.clone(), o).await.unwrap();
        assert!(r.found.is_empty());
        assert_eq!(pa.overlapped.load(Ordering::SeqCst), 0);
        assert_eq!(pb.overlapped.load(Ordering::SeqCst), 0);
        assert_eq!(pa.listings().len(), 4);
    }

    #[tokio::test]
    async fn known_synced_mtime_only_counts_identical() {
        let (a, b) = (MemFs::new(), MemFs::new());
        a.add_file("/same", b"abc", t(100));
        b.add_file("/same", b"abc", t(0));
        a.add_file("/grew", b"abcd", t(100));
        b.add_file("/grew", b"abc", t(0));
        let mut o = opts();
        o.known_synced.insert("same".into(), (3, 3));
        o.known_synced.insert("grew".into(), (3, 3));
        let r = run(Arc::new(a), Arc::new(b), o).await.unwrap();
        assert_eq!(statuses(&r), vec![("grew", &Status::LocalNewer)]);
        assert_eq!(r.summary.identical, 1);
    }

    #[tokio::test]
    async fn seen_counts_are_sent() {
        let (a, b) = (MemFs::new(), MemFs::new());
        a.add_file("/f", b"x", t(0));
        b.add_file("/f", b"x", t(0));
        let r = run(Arc::new(a), Arc::new(b), opts()).await.unwrap();
        assert_eq!(
            r.events.last(),
            Some(&Progress::Seen {
                local: 1,
                remote: 1
            })
        );
    }

    #[tokio::test]
    async fn works_on_a_local_directory() {
        let (da, db) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        std::fs::create_dir(da.path().join("sub")).unwrap();
        std::fs::write(da.path().join("sub").join("f"), b"abc").unwrap();
        std::fs::create_dir(db.path().join("sub")).unwrap();
        std::fs::write(db.path().join("sub").join("f"), b"abcd").unwrap();
        std::fs::write(db.path().join("only"), b"x").unwrap();
        let r = run(
            Arc::new(LocalFs::new(da.path())),
            Arc::new(LocalFs::new(db.path())),
            opts(),
        )
        .await
        .unwrap();
        let s = statuses(&r);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0], ("only", &Status::OnlyRemote));
        assert_eq!(s[1].0, "sub/f");
    }
}
