//! Crash-safe file replacement: write a sibling temporary file, flush it to
//! disk, then rename it over the destination.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use hoprs_core::{Error, ErrorKind, Result};

/// Atomically replaces `path` with `bytes`.
///
/// The parent directory is created if missing (`0o700` on Unix) and the file
/// ends up `0o600` on Unix. On error the temporary file is removed and the
/// destination is left untouched.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let file_name = path
        .file_name()
        .ok_or_else(|| Error::new(ErrorKind::Io, "Invalid profile path"))?;
    let parent = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    create_private_dir(parent)?;

    let (tmp_path, mut file) = create_temp(parent, file_name)?;
    let written = file
        .write_all(bytes)
        .and_then(|()| file.sync_all())
        .and_then(|()| {
            drop(file);
            fs::rename(&tmp_path, path)
        });
    if let Err(err) = written {
        let _ = fs::remove_file(&tmp_path);
        return Err(err.into());
    }
    sync_dir(parent);
    Ok(())
}

fn create_private_dir(dir: &Path) -> Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir)?;
    Ok(())
}

/// Creates a new, uniquely named temporary file next to the destination.
fn create_temp(dir: &Path, file_name: &std::ffi::OsStr) -> Result<(PathBuf, File)> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut attempts = 0;
    loop {
        let mut name = std::ffi::OsString::from(".");
        name.push(file_name);
        name.push(format!(
            ".{}.{}.tmp",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let tmp_path = dir.join(name);
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&tmp_path) {
            Ok(file) => return Ok((tmp_path, file)),
            // A stale temporary from a crashed process with the same pid.
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists && attempts < 16 => {
                attempts += 1;
            }
            Err(err) => return Err(err.into()),
        }
    }
}

/// Makes the rename durable. Best effort: the data itself is already synced.
#[cfg(unix)]
fn sync_dir(dir: &Path) {
    if let Ok(d) = File::open(dir) {
        let _ = d.sync_all();
    }
}

#[cfg(not(unix))]
fn sync_dir(_dir: &Path) {}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn writes_and_replaces() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("a.toml");
        write_atomic(&path, b"one").unwrap();
        write_atomic(&path, b"two").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"two");
        let names: Vec<_> = fs::read_dir(tmp.path()).unwrap().collect();
        assert_eq!(names.len(), 1, "no temporary left behind");
    }

    #[test]
    fn atomic_write_never_leaves_partial_file() {
        let tmp = tempfile::tempdir().unwrap();
        let original = tmp.path().join("sites.toml");
        fs::write(&original, b"version = 1\n").unwrap();
        // The destination is a non-empty directory, so the rename must fail.
        let target = tmp.path().join("target");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("keep"), b"keep").unwrap();

        assert!(write_atomic(&target, b"new contents").is_err());

        assert_eq!(fs::read(&original).unwrap(), b"version = 1\n");
        assert!(target.is_dir());
        assert_eq!(fs::read(target.join("keep")).unwrap(), b"keep");
        let mut names: Vec<String> = fs::read_dir(tmp.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, ["sites.toml", "target"], "no temporary left behind");
    }
}
