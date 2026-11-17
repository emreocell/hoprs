//! The update download (ruling 7): `SHA256SUMS` first (a missing entry downloads nothing),
//! then the asset into a part file, verified before it is renamed to `<name>`. The final
//! name never holds unverified bytes; a file already there is reused when it matches and
//! replaced only by a download that does.
//!
//! The part file is `<name>.<pid>.<n>.part`, created with `create_new` (never an existing
//! path, never through a symlink), so two downloads never share one. It is written,
//! hashed and synced through the one handle that created it. An existing `<name>` is
//! reused only when it is a regular file (not a symlink) that this user owns (Unix: its
//! uid; Windows has no cheap equivalent and the Downloads folder is per-user there, so
//! the check is skipped) and that verifies. A `<name>` that is not a regular file (a
//! directory, a symlink) is refused, never replaced.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use hoprs_core::{Error, ErrorKind};
use sha2::{Digest, Sha256};

use super::Asset;
use super::sums::{expected_for, verify};

/// How a download reaches the network (the real one is [`super::http::Client`]).
pub trait Fetch {
    /// A small text file (`SHA256SUMS`).
    fn text(&self, url: &str) -> Result<String, Error>;
    /// Writes the body of `url` into `out`, calling `progress` with the bytes written so
    /// far and the expected total, if known. Returns the bytes written.
    fn file(
        &self,
        url: &str,
        out: &mut dyn Write,
        progress: &mut dyn FnMut(u64, Option<u64>),
    ) -> Result<u64, Error>;
}

/// What to download: the installer and the release's `SHA256SUMS`.
#[derive(Clone, Debug)]
pub struct DownloadJob {
    pub asset: Asset,
    pub sums_url: String,
}

/// What a download reports, ending with exactly one `Done`.
#[derive(Clone, Debug)]
pub enum DownloadEvent {
    Progress { done: u64, total: Option<u64> },
    Verifying,
    Done(Result<PathBuf, Error>),
}

/// The least time between two `Progress` events.
const PROGRESS_EVERY: Duration = Duration::from_millis(100);

/// Runs the download of `job` into `dir` (see the module docs). Blocking.
pub fn run(fetch: &dyn Fetch, job: &DownloadJob, dir: &Path, send: &mut dyn FnMut(DownloadEvent)) {
    let result = steps(fetch, job, dir, send);
    send(DownloadEvent::Done(result));
}

/// Where downloads go: the user's Downloads folder when there is one; otherwise a private
/// per-user directory, `<cache>/hoprs/updates` (made `0700` on Unix), or, without a cache
/// directory, a new private temporary directory. Never the shared temporary directory
/// itself, where another user could plant or swap a file. (The choice is
/// `choose_download_dir`, which tests drive with their own directories.)
pub fn download_dir() -> Result<PathBuf, Error> {
    choose_download_dir(
        dirs::download_dir(),
        dirs::cache_dir(),
        &std::env::temp_dir(),
    )
}

/// [`download_dir`] from its inputs: the Downloads folder, the cache directory, and the
/// directory a new private temporary directory goes in as the last resort.
fn choose_download_dir(
    downloads: Option<PathBuf>,
    cache: Option<PathBuf>,
    temp: &Path,
) -> Result<PathBuf, Error> {
    if let Some(dir) = downloads.filter(|dir| dir.is_dir()) {
        return Ok(dir);
    }
    if let Some(dir) = cache.map(|dir| dir.join("hoprs").join("updates"))
        && private_dir(&dir).is_ok()
    {
        return Ok(dir);
    }
    tempfile::Builder::new()
        .prefix("hoprs-update-")
        .tempdir_in(temp)
        .map(tempfile::TempDir::keep)
        .map_err(|e| failed(format!("no directory to download into: {e}")))
}

/// Makes `dir` (and its parents) if needed; it must be a real directory of this user,
/// private on Unix.
fn private_dir(dir: &Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(dir)?;
    let meta = std::fs::symlink_metadata(dir)?;
    if !meta.is_dir() || !owned(&meta) {
        return Err(std::io::Error::other("not a private directory"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Whether this user owns the file (always true on Windows, see the module docs).
fn owned(meta: &std::fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // SAFETY: geteuid has no preconditions and cannot fail.
        meta.uid() == unsafe { libc::geteuid() }
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        true
    }
}

pub(super) fn failed(detail: impl Into<String>) -> Error {
    Error::coded(
        ErrorKind::Network,
        "update-download-failed",
        "The update could not be downloaded",
    )
    .with_detail(detail)
}

/// A plain file name: the asset name becomes a path in the user's Downloads folder.
fn plain_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\', ':', '\0'])
        && !name.starts_with('.')
}

/// What is at the final path now.
enum Existing {
    None,
    /// A regular file this user owns.
    Ours,
    /// A regular file of someone else: never reused, but replaced by a verified download.
    Foreign,
}

fn existing(target: &Path) -> Result<Existing, Error> {
    match std::fs::symlink_metadata(target) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Existing::None),
        Err(e) => Err(failed(e.to_string())),
        Ok(meta) if !meta.is_file() => Err(failed(format!(
            "{} exists and is not a regular file",
            target.display()
        ))),
        Ok(meta) if owned(&meta) => Ok(Existing::Ours),
        Ok(_) => Ok(Existing::Foreign),
    }
}

/// How old a part file must be (last written) before a new download removes it.
const STALE_PART: Duration = Duration::from_secs(60 * 60);

/// Whether `file` is a part file of `name` ([`create_part`]'s `<name>.<pid>.<digits>.part`).
fn is_part_of(file: &str, name: &str) -> bool {
    let Some(middle) = file
        .strip_prefix(name)
        .and_then(|rest| rest.strip_prefix('.'))
        .and_then(|rest| rest.strip_suffix(".part"))
    else {
        return false;
    };
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    matches!(middle.split_once('.'), Some((pid, n)) if digits(pid) && digits(n))
}

/// Removes the part files of `name` in `dir` that earlier downloads left (a crash, a
/// power cut): regular files this user owns, not written for [`STALE_PART`]. A download
/// in flight writes its part file all the time (and one of the same asset in this process
/// is refused while it runs), so its file is never this old. Best effort: errors are
/// ignored.
fn remove_stale_parts(dir: &Path, name: &str, now: std::time::SystemTime) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let file = entry.file_name();
        if !file.to_str().is_some_and(|file| is_part_of(file, name)) {
            continue;
        }
        let path = entry.path();
        // Not followed: a symlink is no regular file.
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        let old = meta
            .modified()
            .ok()
            .and_then(|at| now.duration_since(at).ok())
            .is_some_and(|age| age >= STALE_PART);
        if meta.is_file() && owned(&meta) && old {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// A new part file next to `target`, created exclusively.
fn create_part(dir: &Path, name: &str) -> Result<(PathBuf, File), Error> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let pid = std::process::id();
    let mut last = None;
    for _ in 0..16 {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let path = dir.join(format!("{name}.{pid}.{n}{nanos:09}.part"));
        match OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => return Ok((path, file)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => last = Some(e),
            Err(e) => return Err(failed(e.to_string())),
        }
    }
    Err(failed(format!(
        "no free part file name: {}",
        last.map(|e| e.to_string()).unwrap_or_default()
    )))
}

fn steps(
    fetch: &dyn Fetch,
    job: &DownloadJob,
    dir: &Path,
    send: &mut dyn FnMut(DownloadEvent),
) -> Result<PathBuf, Error> {
    let name = &job.asset.name;
    if !plain_name(name) {
        return Err(super::bad(format!(
            "asset name `{name}` is not a file name"
        )));
    }
    remove_stale_parts(dir, name, std::time::SystemTime::now());
    let sums = fetch.text(&job.sums_url)?;
    let expected = expected_for(&sums, name)?;
    let target = dir.join(name);
    if matches!(existing(&target)?, Existing::Ours) && verify(&target, &expected).is_ok() {
        return Ok(target);
    }
    let (part, mut file) = create_part(dir, name)?;
    let result = fetch_and_verify(fetch, job, &mut file, &expected, send).and_then(|()| {
        drop(file);
        // Checked again just before: something may have appeared meanwhile.
        existing(&target)?;
        std::fs::rename(&part, &target).map_err(|e| failed(e.to_string()))
    });
    match result {
        Ok(()) => Ok(target),
        Err(err) => {
            let _ = std::fs::remove_file(&part);
            Err(err)
        }
    }
}

fn fetch_and_verify(
    fetch: &dyn Fetch,
    job: &DownloadJob,
    file: &mut File,
    expected: &[u8; 32],
    send: &mut dyn FnMut(DownloadEvent),
) -> Result<(), Error> {
    let mut last: Option<Instant> = None;
    let mut progress = |done: u64, total: Option<u64>| {
        if last.is_none_or(|at| at.elapsed() >= PROGRESS_EVERY) {
            last = Some(Instant::now());
            send(DownloadEvent::Progress { done, total });
        }
    };
    let written = fetch.file(&job.asset.url, file, &mut progress)?;
    // GitHub reports the exact size; a different count is a broken download, whatever
    // the checksum would say.
    let size = job.asset.size;
    if size != 0 && written != size {
        return Err(failed(format!(
            "got {written} bytes, the release lists {size}"
        )));
    }
    send(DownloadEvent::Verifying);
    let io = |e: std::io::Error| failed(e.to_string());
    file.sync_all().map_err(io)?;
    // Hashed through the handle that wrote it, not by reopening the path.
    file.rewind().map_err(io)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buffer).map_err(io)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    if <[u8; 32]>::from(hasher.finalize()) == *expected {
        Ok(())
    } else {
        Err(Error::coded(
            ErrorKind::Protocol,
            "update-checksum-mismatch",
            "The download does not match its checksum",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;

    const NAME: &str = "hoprs_9.9.9-1_amd64.deb";
    const ASSET_URL: &str = "https://github.com/x/dl/hoprs_9.9.9-1_amd64.deb";
    const SUMS_URL: &str = "https://github.com/x/dl/SHA256SUMS";

    fn hex(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    /// Serves URLs from a map; `cut` stops a file body after that many bytes with an error.
    /// With `watch`, `file` records that directory's entries while it writes.
    #[derive(Default)]
    struct TestFetch {
        bodies: HashMap<String, Vec<u8>>,
        cut: Option<usize>,
        files: Cell<u32>,
        watch: Option<PathBuf>,
        seen: RefCell<Vec<String>>,
    }

    impl TestFetch {
        fn new(asset: &[u8], sums_for: &[u8]) -> TestFetch {
            let mut bodies = HashMap::new();
            bodies.insert(ASSET_URL.to_owned(), asset.to_vec());
            bodies.insert(
                SUMS_URL.to_owned(),
                format!("{}  other.msi\n{}  {NAME}\n", hex(b"x"), hex(sums_for)).into_bytes(),
            );
            TestFetch {
                bodies,
                ..TestFetch::default()
            }
        }
    }

    impl Fetch for TestFetch {
        fn text(&self, url: &str) -> Result<String, Error> {
            let body = self.bodies.get(url).ok_or_else(|| failed("404"))?;
            Ok(String::from_utf8(body.clone()).unwrap())
        }

        fn file(
            &self,
            url: &str,
            out: &mut dyn Write,
            progress: &mut dyn FnMut(u64, Option<u64>),
        ) -> Result<u64, Error> {
            self.files.set(self.files.get() + 1);
            if let Some(dir) = &self.watch {
                *self.seen.borrow_mut() = names(dir);
            }
            let body = self.bodies.get(url).ok_or_else(|| failed("404"))?;
            let total = Some(body.len() as u64);
            let take = self.cut.unwrap_or(body.len()).min(body.len());
            let mut done = 0u64;
            for chunk in body[..take].chunks(3) {
                out.write_all(chunk).unwrap();
                done += chunk.len() as u64;
                progress(done, total);
            }
            if self.cut.is_some() {
                return Err(failed("connection reset"));
            }
            Ok(done)
        }
    }

    fn job(size: u64) -> DownloadJob {
        DownloadJob {
            asset: Asset {
                name: NAME.to_owned(),
                url: ASSET_URL.to_owned(),
                size,
            },
            sums_url: SUMS_URL.to_owned(),
        }
    }

    fn collect(fetch: &TestFetch, job: &DownloadJob, dir: &Path) -> Vec<DownloadEvent> {
        let mut events = Vec::new();
        run(fetch, job, dir, &mut |event| events.push(event));
        events
    }

    /// The single `Done` at the end.
    fn done(events: &[DownloadEvent]) -> Result<PathBuf, Error> {
        let dones = events
            .iter()
            .filter(|e| matches!(e, DownloadEvent::Done(_)))
            .count();
        assert_eq!(dones, 1, "{events:?}");
        match events.last() {
            Some(DownloadEvent::Done(result)) => result.clone(),
            other => panic!("last event {other:?}"),
        }
    }

    /// The entry names of `dir`, sorted.
    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    /// No part file is left in `dir`.
    fn no_parts(dir: &Path) -> bool {
        !names(dir).iter().any(|n| n.ends_with(".part"))
    }

    const BYTES: &[u8] = b"the new installer bytes";

    #[test]
    fn verified_download_is_renamed() {
        let dir = tempfile::tempdir().unwrap();
        let mut fetch = TestFetch::new(BYTES, BYTES);
        fetch.watch = Some(dir.path().to_owned());
        let events = collect(&fetch, &job(BYTES.len() as u64), dir.path());
        assert!(
            matches!(
                events[0],
                DownloadEvent::Progress {
                    total: Some(23),
                    ..
                }
            ),
            "{events:?}"
        );
        let verifying = events
            .iter()
            .position(|e| matches!(e, DownloadEvent::Verifying))
            .unwrap();
        assert_eq!(verifying, events.len() - 2);
        assert!(
            events[..verifying]
                .iter()
                .all(|e| matches!(e, DownloadEvent::Progress { .. }))
        );
        assert_eq!(done(&events).unwrap(), dir.path().join(NAME));
        assert_eq!(std::fs::read(dir.path().join(NAME)).unwrap(), BYTES);
        assert!(no_parts(dir.path()));
        // While downloading, only a unique part file existed, never the final name.
        let seen = fetch.seen.borrow();
        assert_eq!(seen.len(), 1, "{seen:?}");
        assert!(
            seen[0].starts_with(&format!("{NAME}.")) && seen[0].ends_with(".part"),
            "{seen:?}"
        );
        assert_ne!(seen[0], format!("{NAME}.part"));
    }

    #[test]
    fn checksum_mismatch_deletes_part() {
        let dir = tempfile::tempdir().unwrap();
        let fetch = TestFetch::new(BYTES, b"something else");
        let events = collect(&fetch, &job(BYTES.len() as u64), dir.path());
        assert_eq!(
            done(&events).unwrap_err().code,
            Some("update-checksum-mismatch")
        );
        assert!(!dir.path().join(NAME).exists());
        assert!(no_parts(dir.path()));
    }

    #[test]
    fn truncated_download_leaves_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut fetch = TestFetch::new(BYTES, BYTES);
        fetch.cut = Some(10);
        let events = collect(&fetch, &job(BYTES.len() as u64), dir.path());
        assert_eq!(
            done(&events).unwrap_err().code,
            Some("update-download-failed")
        );
        assert!(!events.iter().any(|e| matches!(e, DownloadEvent::Verifying)));
        assert!(!dir.path().join(NAME).exists());
        assert!(no_parts(dir.path()));
    }

    #[test]
    fn short_body_against_the_listed_size_fails() {
        // A server that ends the body early without an error (no Content-Length): the
        // size GitHub listed catches it before the checksum does.
        let dir = tempfile::tempdir().unwrap();
        let fetch = TestFetch::new(&BYTES[..10], BYTES);
        let events = collect(&fetch, &job(BYTES.len() as u64), dir.path());
        assert_eq!(
            done(&events).unwrap_err().code,
            Some("update-download-failed")
        );
        assert!(!dir.path().join(NAME).exists());
        assert!(no_parts(dir.path()));
    }

    #[test]
    fn existing_verified_file_is_reused() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(NAME), BYTES).unwrap();
        let fetch = TestFetch::new(b"not fetched", BYTES);
        let events = collect(&fetch, &job(BYTES.len() as u64), dir.path());
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(done(&events).unwrap(), dir.path().join(NAME));
        assert_eq!(fetch.files.get(), 0);
        assert_eq!(std::fs::read(dir.path().join(NAME)).unwrap(), BYTES);
    }

    #[test]
    fn existing_bad_file_kept_until_new_one_verifies() {
        let dir = tempfile::tempdir().unwrap();
        let old = b"an older, unrelated file";
        std::fs::write(dir.path().join(NAME), old).unwrap();
        // The new download does not match: the old file stays as it was.
        let fetch = TestFetch::new(b"corrupted bytes here!!!", BYTES);
        let events = collect(&fetch, &job(BYTES.len() as u64), dir.path());
        assert_eq!(
            done(&events).unwrap_err().code,
            Some("update-checksum-mismatch")
        );
        assert_eq!(std::fs::read(dir.path().join(NAME)).unwrap(), old);
        assert!(no_parts(dir.path()));
        // A good download replaces it.
        let fetch = TestFetch::new(BYTES, BYTES);
        let events = collect(&fetch, &job(BYTES.len() as u64), dir.path());
        assert_eq!(done(&events).unwrap(), dir.path().join(NAME));
        assert_eq!(std::fs::read(dir.path().join(NAME)).unwrap(), BYTES);
        assert!(no_parts(dir.path()));
    }

    #[test]
    fn missing_sums_entry_downloads_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut fetch = TestFetch::new(BYTES, BYTES);
        fetch.bodies.insert(
            SUMS_URL.to_owned(),
            format!("{}  other.msi\n", hex(BYTES)).into_bytes(),
        );
        let events = collect(&fetch, &job(BYTES.len() as u64), dir.path());
        assert_eq!(
            done(&events).unwrap_err().code,
            Some("update-checksum-missing")
        );
        assert_eq!(fetch.files.get(), 0);
        assert!(!dir.path().join(NAME).exists());
        assert!(no_parts(dir.path()));
    }

    #[test]
    fn unwritable_directory_fails_cleanly() {
        // The download folder vanished (or cannot be written): an error, no panic.
        let dir = tempfile::tempdir().unwrap();
        let gone = dir.path().join("missing");
        let fetch = TestFetch::new(BYTES, BYTES);
        let mut events = Vec::new();
        run(&fetch, &job(BYTES.len() as u64), &gone, &mut |e| {
            events.push(e)
        });
        assert_eq!(
            done(&events).unwrap_err().code,
            Some("update-download-failed")
        );
        assert_eq!(fetch.files.get(), 0);
        assert!(!gone.exists());
    }

    /// (a) Part files are created exclusively under fresh names: an old `<name>.part` (or
    /// anything planted under that name) is neither followed nor touched, and two
    /// downloads never share one.
    #[test]
    fn part_files_are_exclusive_and_unique() {
        let dir = tempfile::tempdir().unwrap();
        let planted = dir.path().join(format!("{NAME}.part"));
        std::fs::write(&planted, b"planted").unwrap();
        let (a, _fa) = create_part(dir.path(), NAME).unwrap();
        let (b, _fb) = create_part(dir.path(), NAME).unwrap();
        assert_ne!(a, b);
        assert_ne!(a, planted);
        let fetch = TestFetch::new(BYTES, BYTES);
        let events = collect(&fetch, &job(BYTES.len() as u64), dir.path());
        assert_eq!(done(&events).unwrap(), dir.path().join(NAME));
        assert_eq!(std::fs::read(&planted).unwrap(), b"planted");
    }

    /// (b) The fallback directory is a private one of this user.
    #[test]
    fn private_dir_is_private() {
        let dir = tempfile::tempdir().unwrap();
        let updates = dir.path().join("hoprs").join("updates");
        private_dir(&updates).unwrap();
        assert!(updates.is_dir());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&updates).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700);
            // A symlink in its place is refused.
            let link = dir.path().join("link");
            std::os::unix::fs::symlink(dir.path(), &link).unwrap();
            assert!(private_dir(&link).is_err());
        }
        // Again: it already exists, fine.
        private_dir(&updates).unwrap();
    }

    /// The Downloads folder, else `<cache>/hoprs/updates`, else a new directory inside the
    /// temporary one (never the temporary directory itself). Only tempdirs are touched.
    #[test]
    fn download_dir_choice() {
        let root = tempfile::tempdir().unwrap();
        let downloads = root.path().join("Downloads");
        let cache = root.path().join("cache");
        let temp = root.path().join("tmp");
        std::fs::create_dir(&downloads).unwrap();
        std::fs::create_dir(&temp).unwrap();

        let chosen =
            choose_download_dir(Some(downloads.clone()), Some(cache.clone()), &temp).unwrap();
        assert_eq!(chosen, downloads);
        assert!(!cache.exists());

        // No Downloads folder (or one that is not there): the private cache directory.
        let missing = root.path().join("missing");
        let chosen = choose_download_dir(Some(missing.clone()), Some(cache.clone()), &temp);
        assert_eq!(chosen.unwrap(), cache.join("hoprs").join("updates"));
        assert!(!missing.exists());

        // Neither: a new directory inside `temp`, not `temp` itself.
        let chosen = choose_download_dir(None, None, &temp).unwrap();
        assert_ne!(chosen, temp);
        assert_eq!(chosen.parent(), Some(temp.as_path()));
        assert!(chosen.is_dir());
    }

    /// (c, e) A directory at the final name is refused, not replaced or reused.
    #[test]
    fn directory_at_final_name_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(NAME)).unwrap();
        let fetch = TestFetch::new(BYTES, BYTES);
        let events = collect(&fetch, &job(BYTES.len() as u64), dir.path());
        assert_eq!(
            done(&events).unwrap_err().code,
            Some("update-download-failed")
        );
        assert_eq!(fetch.files.get(), 0);
        assert!(dir.path().join(NAME).is_dir());
        assert!(no_parts(dir.path()));
    }

    /// (c, e) A symlink at the final name, even to a matching file, is neither reused nor
    /// replaced.
    #[cfg(unix)]
    #[test]
    fn symlink_at_final_name_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let real = elsewhere.path().join("real.deb");
        std::fs::write(&real, BYTES).unwrap();
        std::os::unix::fs::symlink(&real, dir.path().join(NAME)).unwrap();
        let fetch = TestFetch::new(BYTES, BYTES);
        let events = collect(&fetch, &job(BYTES.len() as u64), dir.path());
        assert_eq!(
            done(&events).unwrap_err().code,
            Some("update-download-failed")
        );
        assert_eq!(fetch.files.get(), 0);
        assert!(
            std::fs::symlink_metadata(dir.path().join(NAME))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(std::fs::read(&real).unwrap(), BYTES);
    }

    /// (c) Ownership: a file this user made counts as ours.
    #[test]
    fn own_files_are_owned() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f");
        std::fs::write(&file, b"x").unwrap();
        assert!(owned(&std::fs::symlink_metadata(&file).unwrap()));
        assert!(matches!(existing(&file).unwrap(), Existing::Ours));
        assert!(matches!(
            existing(&dir.path().join("none")).unwrap(),
            Existing::None
        ));
        // Someone else's file (the root directory belongs to root) is not ours.
        #[cfg(unix)]
        // SAFETY: geteuid has no preconditions and cannot fail.
        if unsafe { libc::geteuid() } != 0 {
            assert!(!owned(&std::fs::symlink_metadata("/").unwrap()));
        }
    }

    #[test]
    fn asset_names_must_be_plain_file_names() {
        let dir = tempfile::tempdir().unwrap();
        for name in [
            "../evil.deb",
            "a/b.deb",
            "a\\b.msi",
            "",
            ".hidden",
            "C:x.msi",
        ] {
            let fetch = TestFetch::new(BYTES, BYTES);
            let mut job = job(BYTES.len() as u64);
            job.asset.name = name.to_owned();
            let events = collect(&fetch, &job, dir.path());
            assert_eq!(
                done(&events).unwrap_err().code,
                Some("update-bad-response"),
                "{name}"
            );
            assert_eq!(fetch.files.get(), 0);
        }
    }

    #[test]
    fn part_file_names() {
        assert!(is_part_of(&format!("{NAME}.1234.5678.part"), NAME));
        for other in [
            format!("{NAME}.part"),
            format!("{NAME}.1234.part"),
            format!("{NAME}.x.5678.part"),
            format!("{NAME}.1234.56a.part"),
            format!("{NAME}.1234.5678.part.bak"),
            format!("{NAME}.1234.5678"),
            "other.deb.1234.5678.part".to_owned(),
            format!("{NAME}x.1234.5678.part"),
            NAME.to_owned(),
        ] {
            assert!(!is_part_of(&other, NAME), "{other}");
        }
    }

    /// A new download removes this asset's part files older than an hour; a fresh one
    /// (another download writing it), other files and other assets' parts stay.
    #[test]
    fn stale_part_files_are_removed() {
        let dir = tempfile::tempdir().unwrap();
        let now = std::time::SystemTime::now();
        let old = now - STALE_PART - Duration::from_secs(60);
        let make = |file: &str, at: std::time::SystemTime| {
            let path = dir.path().join(file);
            std::fs::write(&path, b"x").unwrap();
            File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(at)
                .unwrap();
            path
        };
        let stale = make(&format!("{NAME}.11.22.part"), old);
        let live = make(&format!("{NAME}.33.44.part"), now);
        let planted = make(&format!("{NAME}.part"), old);
        let other = make("other.deb.11.22.part", old);
        let installer = make(NAME, old);
        // A directory with a part file's name is not a regular file: kept.
        let folder = dir.path().join(format!("{NAME}.55.66.part"));
        std::fs::create_dir(&folder).unwrap();
        #[cfg(unix)]
        let link = {
            let target = make("target", old);
            let link = dir.path().join(format!("{NAME}.77.88.part"));
            std::os::unix::fs::symlink(&target, &link).unwrap();
            link
        };

        remove_stale_parts(dir.path(), NAME, now);
        assert!(!stale.exists());
        for kept in [&live, &planted, &other, &installer, &folder] {
            assert!(kept.exists(), "{}", kept.display());
        }
        #[cfg(unix)]
        assert!(std::fs::symlink_metadata(&link).is_ok());

        // Through a real download too.
        let stale = make(&format!("{NAME}.11.23.part"), old);
        let fetch = TestFetch::new(BYTES, BYTES);
        let events = collect(&fetch, &job(BYTES.len() as u64), dir.path());
        assert_eq!(done(&events).unwrap(), dir.path().join(NAME));
        assert!(!stale.exists());
        assert!(live.exists());
    }
}
