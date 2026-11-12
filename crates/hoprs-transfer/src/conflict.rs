use hoprs_core::RemoteEntry;

use crate::model::ConflictPolicy;

/// Outcome of the conflict decision table for one source entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    Write { offset: u64 },
    Skip(String),
    Ask,
}

/// Decides what to do with `src` given the policy and the existing destination entry, if any.
pub fn decide(policy: ConflictPolicy, src: &RemoteEntry, dst: Option<&RemoteEntry>) -> Decision {
    let Some(dst) = dst else {
        return Decision::Write { offset: 0 };
    };
    match policy {
        ConflictPolicy::Ask => Decision::Ask,
        ConflictPolicy::Overwrite => Decision::Write { offset: 0 },
        ConflictPolicy::Skip => Decision::Skip("exists".to_string()),
        ConflictPolicy::OverwriteIfNewer => match (src.modified, dst.modified) {
            (Some(src_mtime), Some(dst_mtime)) if src_mtime > dst_mtime => {
                Decision::Write { offset: 0 }
            }
            (Some(_), Some(_)) => Decision::Skip("not newer".to_string()),
            _ => Decision::Write { offset: 0 },
        },
        ConflictPolicy::Resume => {
            if dst.size < src.size {
                Decision::Write { offset: dst.size }
            } else if dst.size == src.size {
                Decision::Skip("already complete".to_string())
            } else {
                Decision::Write { offset: 0 }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use hoprs_core::{EntryKind, RemoteEntry, RemotePath};

    use super::{Decision, decide};
    use crate::model::ConflictPolicy;

    fn entry(size: u64, modified: Option<SystemTime>) -> RemoteEntry {
        let path = RemotePath::new("/data/file.bin").expect("valid test path");
        RemoteEntry {
            name: "file.bin".to_string(),
            path,
            kind: EntryKind::File,
            size,
            modified,
            mode: None,
        }
    }

    fn at(secs: u64) -> Option<SystemTime> {
        Some(SystemTime::UNIX_EPOCH + Duration::from_secs(secs))
    }

    #[test]
    fn no_destination_writes_from_zero_under_any_policy() {
        let src = entry(300, at(100));
        for policy in [
            ConflictPolicy::Ask,
            ConflictPolicy::Overwrite,
            ConflictPolicy::Skip,
            ConflictPolicy::OverwriteIfNewer,
            ConflictPolicy::Resume,
        ] {
            assert_eq!(
                decide(policy, &src, None),
                Decision::Write { offset: 0 },
                "policy {policy:?}"
            );
        }
    }

    #[test]
    fn ask_policy_with_existing_destination_asks() {
        let src = entry(300, at(100));
        let dst = entry(300, at(100));
        assert_eq!(decide(ConflictPolicy::Ask, &src, Some(&dst)), Decision::Ask);
    }

    #[test]
    fn overwrite_policy_with_existing_destination_writes_from_zero() {
        let src = entry(300, at(100));
        let dst = entry(500, at(200));
        assert_eq!(
            decide(ConflictPolicy::Overwrite, &src, Some(&dst)),
            Decision::Write { offset: 0 }
        );
    }

    #[test]
    fn skip_policy_with_existing_destination_skips_as_exists() {
        let src = entry(300, at(100));
        let dst = entry(300, at(100));
        assert_eq!(
            decide(ConflictPolicy::Skip, &src, Some(&dst)),
            Decision::Skip("exists".to_string())
        );
    }

    #[test]
    fn overwrite_if_newer_writes_when_source_is_newer() {
        let src = entry(300, at(200));
        let dst = entry(300, at(100));
        assert_eq!(
            decide(ConflictPolicy::OverwriteIfNewer, &src, Some(&dst)),
            Decision::Write { offset: 0 }
        );
    }

    #[test]
    fn overwrite_if_newer_skips_when_source_is_not_newer() {
        let dst = entry(300, at(100));
        let equal = entry(300, at(100));
        let older = entry(300, at(50));
        for src in [equal, older] {
            assert_eq!(
                decide(ConflictPolicy::OverwriteIfNewer, &src, Some(&dst)),
                Decision::Skip("not newer".to_string()),
                "src modified {:?}",
                src.modified
            );
        }
    }

    #[test]
    fn overwrite_if_newer_writes_when_a_mtime_is_missing() {
        let dst = entry(300, at(100));
        let src_without_mtime = entry(300, None);
        assert_eq!(
            decide(
                ConflictPolicy::OverwriteIfNewer,
                &src_without_mtime,
                Some(&dst)
            ),
            Decision::Write { offset: 0 }
        );

        let src = entry(300, at(200));
        let dst_without_mtime = entry(300, None);
        assert_eq!(
            decide(
                ConflictPolicy::OverwriteIfNewer,
                &src,
                Some(&dst_without_mtime)
            ),
            Decision::Write { offset: 0 }
        );
    }

    #[test]
    fn resume_with_shorter_destination_continues_from_its_size() {
        let src = entry(300, at(100));
        let dst = entry(100, at(100));
        assert_eq!(
            decide(ConflictPolicy::Resume, &src, Some(&dst)),
            Decision::Write { offset: 100 }
        );
    }

    #[test]
    fn resume_with_equal_size_skips_as_already_complete() {
        let src = entry(300, at(100));
        let dst = entry(300, at(100));
        assert_eq!(
            decide(ConflictPolicy::Resume, &src, Some(&dst)),
            Decision::Skip("already complete".to_string())
        );
    }

    #[test]
    fn resume_with_longer_destination_writes_from_zero() {
        let src = entry(300, at(100));
        let dst = entry(500, at(100));
        assert_eq!(
            decide(ConflictPolicy::Resume, &src, Some(&dst)),
            Decision::Write { offset: 0 }
        );
    }
}
