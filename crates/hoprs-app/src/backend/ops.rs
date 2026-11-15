//! File actions shared by the real and the fake backend: the recursive delete and the
//! rename that never replaces.
//!
//! A folder is deleted depth first: list it, remove its files and links, go into its
//! folders, then `remove_dir` it once it is empty (an empty folder is just `remove_dir`).
//! A link the listing reports as a link is removed as a link, never followed. Some FTP
//! servers report a link to a folder as a folder, though: there the walk goes into it
//! (and deletes what the link points at), since nothing in the listing tells them apart.

use std::time::{Duration, Instant};

use hoprs_core::{EntryKind, Error, ErrorKind, RemotePath, Result};
use hoprs_remote::RemoteFs;
use tokio::sync::mpsc;

/// One thing to delete, as the pane listed it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeleteTarget {
    pub path: RemotePath,
    pub kind: EntryKind,
}

/// Progress goes out at most once per this many items…
pub const PROGRESS_EVERY: u64 = 64;
/// …or once per this long, whichever comes first.
pub const PROGRESS_INTERVAL: Duration = Duration::from_millis(50);

/// The running count, throttled: a report every [`PROGRESS_EVERY`] items or
/// [`PROGRESS_INTERVAL`], and always the last count (on success or failure).
struct Progress<'a> {
    sender: &'a mpsc::UnboundedSender<u64>,
    removed: u64,
    sent: u64,
    last: Instant,
}

impl Progress<'_> {
    fn step(&mut self) {
        self.removed += 1;
        if self.removed - self.sent >= PROGRESS_EVERY || self.last.elapsed() >= PROGRESS_INTERVAL {
            self.send();
        }
    }

    fn send(&mut self) {
        if self.removed != self.sent {
            // The UI may be gone; the delete still finishes.
            let _ = self.sender.send(self.removed);
            self.sent = self.removed;
            self.last = Instant::now();
        }
    }
}

/// Deletes `targets` in order, folders with everything in them. The running count of
/// removed items (files, links, folders) goes to `progress`, throttled, the final count
/// always last. Stops at the first error, leaving the rest in place. Returns how many
/// items were removed.
pub async fn delete_all(
    fs: &dyn RemoteFs,
    targets: Vec<DeleteTarget>,
    progress: &mpsc::UnboundedSender<u64>,
) -> Result<u64> {
    let mut progress = Progress {
        sender: progress,
        removed: 0,
        sent: 0,
        last: Instant::now(),
    };
    let result = delete_targets(fs, targets, &mut progress).await;
    progress.send();
    result.map(|()| progress.removed)
}

async fn delete_targets(
    fs: &dyn RemoteFs,
    targets: Vec<DeleteTarget>,
    progress: &mut Progress<'_>,
) -> Result<()> {
    for target in targets {
        if target.kind != EntryKind::Dir {
            fs.remove_file(&target.path).await?;
            progress.step();
            continue;
        }
        // (folder, its content is already removed)
        let mut stack = vec![(target.path, false)];
        while let Some((dir, emptied)) = stack.pop() {
            if emptied {
                fs.remove_dir(&dir).await?;
                progress.step();
                continue;
            }
            stack.push((dir.clone(), true));
            for entry in fs.list(&dir).await? {
                if matches!(entry.name.as_str(), "" | "." | "..") || entry.name.contains('/') {
                    continue;
                }
                let path = dir.join(&entry.name);
                match entry.kind {
                    EntryKind::Dir => stack.push((path, false)),
                    EntryKind::File | EntryKind::Symlink => {
                        fs.remove_file(&path).await?;
                        progress.step();
                    }
                }
            }
        }
    }
    Ok(())
}

/// Renames `from` to `to` unless something is already at `to`: local `rename`, SFTP
/// posix-rename and FTP `RNTO` would all replace it silently, and the pane's listing may
/// be stale.
///
/// A change of case only (`a.txt` → `A.txt`) can't use `stat(to)`: on a case-insensitive
/// disk it finds `from` itself. Instead the parent is listed and only an entry named
/// exactly `to` counts: a case-sensitive server with both `a.txt` and `A.txt` refuses,
/// while a case-insensitive disk lists only the source's own casing and lets it through.
pub async fn rename_no_replace(
    fs: &dyn RemoteFs,
    from: &RemotePath,
    to: &RemotePath,
) -> Result<()> {
    let exists = if from == to {
        false
    } else if from.as_str().to_lowercase() == to.as_str().to_lowercase() {
        let name = to.file_name().unwrap_or_default();
        let parent = to.parent().unwrap_or_else(RemotePath::root);
        fs.list(&parent).await?.iter().any(|e| e.name == name)
    } else {
        match fs.stat(to).await {
            Ok(_) => true,
            Err(e) if e.kind == ErrorKind::NotFound => false,
            Err(e) => return Err(e),
        }
    };
    if exists {
        return Err(
            Error::coded(ErrorKind::Conflict, "fs-already-exists", "Already exists")
                .with_detail(to.as_str().to_owned()),
        );
    }
    fs.rename(from, to).await
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use hoprs_core::ErrorKind;
    use hoprs_remote::MemFs;

    use super::*;

    fn p(s: &str) -> RemotePath {
        RemotePath::new(s).unwrap()
    }

    #[tokio::test]
    async fn delete_all_walks_depth_first_without_following_links() {
        let fs = MemFs::new();
        let t = SystemTime::UNIX_EPOCH;
        fs.add_file("/srv/tree/a.txt", b"a", t);
        fs.add_file("/srv/tree/sub/b.txt", b"b", t);
        fs.add_dir("/srv/tree/sub/empty");
        fs.add_file("/keep/k.txt", b"k", t);
        // A link to a folder outside: removed as a link, its target untouched.
        fs.add_symlink("/srv/tree/link", "/keep");
        fs.add_dir("/srv/empty");
        fs.add_file("/srv/f.txt", b"f", t);

        let (tx, mut rx) = mpsc::unbounded_channel();
        let removed = delete_all(
            &fs,
            vec![
                DeleteTarget {
                    path: p("/srv/tree"),
                    kind: EntryKind::Dir,
                },
                DeleteTarget {
                    path: p("/srv/empty"),
                    kind: EntryKind::Dir,
                },
                DeleteTarget {
                    path: p("/srv/f.txt"),
                    kind: EntryKind::File,
                },
            ],
            &tx,
        )
        .await
        .unwrap();
        // tree: a.txt, link, b.txt, empty/, sub/, tree/ = 6; empty/ = 1; f.txt = 1.
        assert_eq!(removed, 8);
        drop(tx);
        let mut counts = Vec::new();
        while let Some(n) = rx.recv().await {
            counts.push(n);
        }
        assert_eq!(counts.last(), Some(&8), "the final count always goes out");
        assert!(counts.windows(2).all(|w| w[0] < w[1]), "{counts:?}");
        assert!(fs.list(&p("/srv")).await.unwrap().is_empty());
        assert_eq!(fs.read_file("/keep/k.txt").as_deref(), Some(&b"k"[..]));

        // The first error stops the walk and comes back as is.
        fs.add_file("/locked/x/y.txt", b"y", t);
        fs.deny_writes_under("/locked/x");
        let (tx, _rx) = mpsc::unbounded_channel();
        let err = delete_all(
            &fs,
            vec![DeleteTarget {
                path: p("/locked/x"),
                kind: EntryKind::Dir,
            }],
            &tx,
        )
        .await
        .unwrap_err();
        assert_eq!(err.kind, ErrorKind::Permission);
        assert!(fs.read_file("/locked/x/y.txt").is_some());
    }

    /// Progress is throttled to one report per 64 items (or 50 ms), the last always sent,
    /// also when the walk fails half way.
    #[tokio::test]
    async fn delete_progress_is_throttled() {
        let fs = MemFs::new();
        let t = SystemTime::UNIX_EPOCH;
        for i in 0..200 {
            fs.add_file(&format!("/big/f{i:03}"), b"", t);
        }
        let (tx, mut rx) = mpsc::unbounded_channel();
        let target = DeleteTarget {
            path: p("/big"),
            kind: EntryKind::Dir,
        };
        assert_eq!(delete_all(&fs, vec![target], &tx).await.unwrap(), 201);
        drop(tx);
        let mut counts = Vec::new();
        while let Some(n) = rx.recv().await {
            counts.push(n);
        }
        // 64, 128, 192 at least (more only if the 50 ms tick fires), then 201.
        assert!(counts.len() >= 4 && counts.len() < 201, "{counts:?}");
        assert_eq!(counts.last(), Some(&201));
        assert!(counts.windows(2).all(|w| w[1] - w[0] <= 64), "{counts:?}");

        // A failure half way still reports how far it got.
        for i in 0..3 {
            fs.add_file(&format!("/part/a{i}"), b"", t);
        }
        fs.add_file("/part/z/locked", b"", t);
        fs.deny_writes_under("/part/z");
        let (tx, mut rx) = mpsc::unbounded_channel();
        let target = DeleteTarget {
            path: p("/part"),
            kind: EntryKind::Dir,
        };
        assert!(delete_all(&fs, vec![target], &tx).await.is_err());
        drop(tx);
        let mut last = None;
        while let Some(n) = rx.recv().await {
            last = Some(n);
        }
        assert_eq!(last, Some(3), "a0..a2 went before z/locked failed");
    }

    #[tokio::test]
    async fn rename_never_replaces() {
        let fs = MemFs::new();
        let t = SystemTime::UNIX_EPOCH;
        fs.add_file("/a.txt", b"a", t);
        fs.add_file("/b.txt", b"b", t);
        let err = rename_no_replace(&fs, &p("/a.txt"), &p("/b.txt"))
            .await
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::Conflict);
        assert_eq!(fs.read_file("/b.txt").as_deref(), Some(&b"b"[..]));
        rename_no_replace(&fs, &p("/a.txt"), &p("/c.txt"))
            .await
            .unwrap();
        assert_eq!(fs.read_file("/c.txt").as_deref(), Some(&b"a"[..]));
        // A change of case only with no exact-case sibling goes through (MemFs is case
        // sensitive; a case-insensitive disk lists only the source's casing, same result).
        rename_no_replace(&fs, &p("/c.txt"), &p("/C.txt"))
            .await
            .unwrap();
        assert!(fs.read_file("/C.txt").is_some());
        assert!(fs.read_file("/c.txt").is_none());

        // A case-sensitive server with both casings: the exact-case sibling is kept.
        fs.add_file("/x.txt", b"lower", t);
        fs.add_file("/X.txt", b"upper", t);
        let err = rename_no_replace(&fs, &p("/x.txt"), &p("/X.txt"))
            .await
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::Conflict);
        assert_eq!(fs.read_file("/X.txt").as_deref(), Some(&b"upper"[..]));
        assert_eq!(fs.read_file("/x.txt").as_deref(), Some(&b"lower"[..]));
    }
}
