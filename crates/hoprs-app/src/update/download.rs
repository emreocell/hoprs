//! The update download (ruling 7): `SHA256SUMS` first (a missing entry downloads nothing),
//! then the asset into `<name>.part`, verified before it is renamed to `<name>`. The final
//! name never holds unverified bytes; a file already there is reused when it matches and
//! replaced only by a download that does.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use hoprs_core::{Error, ErrorKind};

use super::Asset;
use super::sums::{expected_for, verify};

/// How a download reaches the network (the real one is [`super::http::Client`]).
pub trait Fetch {
    /// A small text file (`SHA256SUMS`).
    fn text(&self, url: &str) -> Result<String, Error>;
    /// Writes the body of `url` into `to` (created or truncated), calling `progress` with
    /// the bytes written so far and the expected total, if known. Returns the bytes written.
    fn file(
        &self,
        url: &str,
        to: &Path,
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

/// Where downloads go: the user's Downloads folder, or the temporary directory without one.
pub fn download_dir() -> PathBuf {
    dirs::download_dir()
        .filter(|dir| dir.is_dir())
        .unwrap_or_else(std::env::temp_dir)
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
    let sums = fetch.text(&job.sums_url)?;
    let expected = expected_for(&sums, name)?;
    let target = dir.join(name);
    if target.is_file() && verify(&target, &expected).is_ok() {
        return Ok(target);
    }
    let part = dir.join(format!("{name}.part"));
    let result = fetch_and_verify(fetch, job, &part, &expected, send)
        .and_then(|()| std::fs::rename(&part, &target).map_err(|e| failed(e.to_string())));
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
    part: &Path,
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
    let written = fetch.file(&job.asset.url, part, &mut progress)?;
    // GitHub reports the exact size; a different count is a broken download, whatever
    // the checksum would say.
    let size = job.asset.size;
    if size != 0 && written != size {
        return Err(failed(format!(
            "got {written} bytes, the release lists {size}"
        )));
    }
    send(DownloadEvent::Verifying);
    verify(part, expected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
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
    #[derive(Default)]
    struct TestFetch {
        bodies: HashMap<String, Vec<u8>>,
        cut: Option<usize>,
        files: Cell<u32>,
        seen_part: RefCell<Vec<PathBuf>>,
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
            to: &Path,
            progress: &mut dyn FnMut(u64, Option<u64>),
        ) -> Result<u64, Error> {
            self.files.set(self.files.get() + 1);
            self.seen_part.borrow_mut().push(to.to_owned());
            let body = self.bodies.get(url).ok_or_else(|| failed("404"))?;
            let total = Some(body.len() as u64);
            let take = self.cut.unwrap_or(body.len()).min(body.len());
            let mut done = 0u64;
            let mut out = Vec::new();
            for chunk in body[..take].chunks(3) {
                out.extend_from_slice(chunk);
                std::fs::write(to, &out).unwrap();
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

    fn part(dir: &Path) -> PathBuf {
        dir.join(format!("{NAME}.part"))
    }

    const BYTES: &[u8] = b"the new installer bytes";

    #[test]
    fn verified_download_is_renamed() {
        let dir = tempfile::tempdir().unwrap();
        let fetch = TestFetch::new(BYTES, BYTES);
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
        assert!(!part(dir.path()).exists());
        // The bytes went to the `.part`, never to the final name.
        assert_eq!(*fetch.seen_part.borrow(), vec![part(dir.path())]);
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
        assert!(!part(dir.path()).exists());
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
        assert!(!part(dir.path()).exists());
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
        assert!(!part(dir.path()).exists());
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
        assert!(!part(dir.path()).exists());
        // A good download replaces it.
        let fetch = TestFetch::new(BYTES, BYTES);
        let events = collect(&fetch, &job(BYTES.len() as u64), dir.path());
        assert_eq!(done(&events).unwrap(), dir.path().join(NAME));
        assert_eq!(std::fs::read(dir.path().join(NAME)).unwrap(), BYTES);
        assert!(!part(dir.path()).exists());
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
        assert!(!part(dir.path()).exists());
    }

    #[test]
    fn unwritable_directory_fails_cleanly() {
        // The Downloads folder vanished (or cannot be written): an error, no panic.
        let dir = tempfile::tempdir().unwrap();
        let gone = dir.path().join("missing");
        let fetch = RealFileFetch(TestFetch::new(BYTES, BYTES));
        let mut events = Vec::new();
        run(&fetch, &job(BYTES.len() as u64), &gone, &mut |e| {
            events.push(e)
        });
        assert_eq!(
            done(&events).unwrap_err().code,
            Some("update-download-failed")
        );
        assert!(!gone.exists());
    }

    /// Opens the target like the real client: a missing directory is an I/O error.
    struct RealFileFetch(TestFetch);

    impl Fetch for RealFileFetch {
        fn text(&self, url: &str) -> Result<String, Error> {
            self.0.text(url)
        }

        fn file(
            &self,
            url: &str,
            to: &Path,
            progress: &mut dyn FnMut(u64, Option<u64>),
        ) -> Result<u64, Error> {
            std::fs::File::create(to).map_err(|e| failed(e.to_string()))?;
            self.0.file(url, to, progress)
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
    fn download_dir_exists() {
        assert!(download_dir().is_dir());
    }
}
