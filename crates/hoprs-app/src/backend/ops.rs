//! File actions shared by the real and the fake backend: the recursive delete.
//!
//! A folder is deleted depth first: list it, remove its files and links, go into its
//! folders, then `remove_dir` it once it is empty (an empty folder is just `remove_dir`).
//! Links are never followed: a link to a folder is removed as a link.

use hoprs_core::{EntryKind, RemotePath, Result};
use hoprs_remote::RemoteFs;
use tokio::sync::mpsc;

/// One thing to delete, as the pane listed it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeleteTarget {
    pub path: RemotePath,
    pub kind: EntryKind,
}

/// Deletes `targets` in order, folders with everything in them. After each item removed
/// (file, link or folder) the running count goes to `progress`. Stops at the first error,
/// leaving the rest in place. Returns how many items were removed.
pub async fn delete_all(
    fs: &dyn RemoteFs,
    targets: Vec<DeleteTarget>,
    progress: &mpsc::UnboundedSender<u64>,
) -> Result<u64> {
    let mut removed = 0;
    let step = |removed: &mut u64| {
        *removed += 1;
        // The UI may be gone; the delete still finishes.
        let _ = progress.send(*removed);
    };
    for target in targets {
        if target.kind != EntryKind::Dir {
            fs.remove_file(&target.path).await?;
            step(&mut removed);
            continue;
        }
        // (folder, its content is already removed)
        let mut stack = vec![(target.path, false)];
        while let Some((dir, emptied)) = stack.pop() {
            if emptied {
                fs.remove_dir(&dir).await?;
                step(&mut removed);
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
                        step(&mut removed);
                    }
                }
            }
        }
    }
    Ok(removed)
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
        assert_eq!(counts, (1..=8).collect::<Vec<_>>(), "one report per item");
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
}
