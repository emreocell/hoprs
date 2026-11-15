//! Crash-safe file replacement: write a sibling temporary file, flush it to
//! disk, then rename it over the destination.
//!
//! If the destination is a symbolic link, the file it points to is replaced
//! and the link itself is kept.

use std::ffi::{OsStr, OsString};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use hoprs_core::{Error, ErrorKind, Result};

/// Temporaries older than this are leftovers of a crash and get swept.
const STALE_TEMP_AGE: Duration = Duration::from_secs(60);

/// Upper bound on symlink chains, like the kernel's `ELOOP` limit.
const MAX_SYMLINKS: usize = 40;

/// Atomically replaces `path` with `bytes`.
///
/// The parent directory is created if missing (`0o700` on Unix) and a newly
/// created file is `0o600` on Unix. A symlinked `path` keeps its link: the
/// final target is replaced instead, keeping that file's permissions. On
/// error the temporary file is removed and the destination is left untouched.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let (target, via_link) = resolve_target(path)?;
    let file_name = target
        .file_name()
        .ok_or_else(|| {
            Error::coded(
                ErrorKind::Io,
                "profile-invalid-path",
                "Invalid profile path",
            )
        })?
        .to_owned();
    let parent = match target.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    create_private_dir(parent)?;
    // Only a symlinked file keeps its own permissions; ours stay private.
    let keep_permissions = if via_link {
        fs::metadata(&target).ok().map(|m| m.permissions())
    } else {
        None
    };

    let (tmp_path, mut file) = create_temp(parent, &file_name)?;
    let written = (|| -> io::Result<()> {
        if let Some(permissions) = keep_permissions {
            file.set_permissions(permissions)?;
        }
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp_path, &target)
    })();
    if let Err(err) = written {
        let _ = fs::remove_file(&tmp_path);
        return Err(err.into());
    }
    sync_dir(parent);
    sweep_stale_temps(parent, &file_name);
    Ok(())
}

/// Follows `path` through any chain of symlinks. Returns the final path
/// (which may not exist, for a dangling link) and whether a link was followed.
fn resolve_target(path: &Path) -> Result<(PathBuf, bool)> {
    let mut current = path.to_path_buf();
    for hops in 0..=MAX_SYMLINKS {
        match fs::symlink_metadata(&current) {
            Ok(meta) if meta.file_type().is_symlink() => {
                let link = fs::read_link(&current)?;
                current = if link.is_absolute() {
                    link
                } else {
                    current.parent().unwrap_or(Path::new(".")).join(link)
                };
            }
            Ok(_) => return Ok((current, hops > 0)),
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok((current, hops > 0)),
            Err(err) => return Err(err.into()),
        }
    }
    Err(Error::coded(
        ErrorKind::Io,
        "profile-symlink-loop",
        "Too many levels of symbolic links",
    )
    .with_detail(path.display().to_string()))
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
fn create_temp(dir: &Path, file_name: &OsStr) -> Result<(PathBuf, File)> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut attempts = 0;
    loop {
        let mut name = OsString::from(".");
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

/// Whether `name` looks like `create_temp`'s `.<file>.<pid>.<n>.tmp`.
fn is_temp_of(name: &OsStr, file_name: &OsStr) -> bool {
    let (Some(name), Some(file_name)) = (name.to_str(), file_name.to_str()) else {
        return false;
    };
    let Some(rest) = name
        .strip_prefix('.')
        .and_then(|n| n.strip_prefix(file_name))
        .and_then(|n| n.strip_prefix('.'))
        .and_then(|n| n.strip_suffix(".tmp"))
    else {
        return false;
    };
    let mut parts = rest.split('.');
    let numeric =
        |p: Option<&str>| p.is_some_and(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    numeric(parts.next()) && numeric(parts.next()) && parts.next().is_none()
}

/// Removes temporaries of `file_name` left by earlier crashes. Recent ones
/// are skipped: they may belong to a write in progress in another process.
fn sweep_stale_temps(dir: &Path, file_name: &OsStr) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        if !is_temp_of(&entry.file_name(), file_name) {
            continue;
        }
        let stale = entry
            .metadata()
            .and_then(|m| m.modified())
            .is_ok_and(|t| now.duration_since(t).is_ok_and(|age| age >= STALE_TEMP_AGE));
        if stale {
            let _ = fs::remove_file(entry.path());
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

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn atomic_write_never_leaves_partial_file() {
        let tmp = tempfile::tempdir().unwrap();
        // The destination is a non-empty directory, so the rename must fail.
        let target = tmp.path().join("sites.toml");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("keep"), b"keep").unwrap();

        assert!(write_atomic(&target, b"new contents").is_err());

        assert!(target.is_dir(), "target directory intact");
        assert_eq!(names(&target), ["keep"]);
        assert_eq!(fs::read(target.join("keep")).unwrap(), b"keep");
        assert_eq!(names(tmp.path()), ["sites.toml"], "no temporary left");
    }

    #[test]
    fn stale_temporaries_are_swept() {
        let tmp = tempfile::tempdir().unwrap();
        let old = SystemTime::now() - Duration::from_secs(3600);
        for name in [".sites.toml.1.0.tmp", ".sites.toml.2.5.tmp"] {
            let f = File::create(tmp.path().join(name)).unwrap();
            f.set_modified(old).unwrap();
        }
        // Recent (possibly in-progress) and unrelated files are left alone.
        fs::write(tmp.path().join(".sites.toml.3.0.tmp"), b"").unwrap();
        let unrelated = File::create(tmp.path().join(".other.toml.1.0.tmp")).unwrap();
        unrelated.set_modified(old).unwrap();
        fs::write(tmp.path().join("notes.tmp"), b"").unwrap();

        write_atomic(&tmp.path().join("sites.toml"), b"x").unwrap();
        assert_eq!(
            names(tmp.path()),
            [
                ".other.toml.1.0.tmp",
                ".sites.toml.3.0.tmp",
                "notes.tmp",
                "sites.toml"
            ]
        );
    }

    #[cfg(unix)]
    fn mode(p: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(p).unwrap().permissions().mode() & 0o777
    }

    #[cfg(unix)]
    #[test]
    fn symlink_to_file_in_other_dir_is_kept() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        let tmp = tempfile::tempdir().unwrap();
        let dotfiles = tmp.path().join("dotfiles");
        fs::create_dir(&dotfiles).unwrap();
        let real = dotfiles.join("sites.toml");
        fs::write(&real, b"old").unwrap();
        fs::set_permissions(&real, fs::Permissions::from_mode(0o640)).unwrap();
        let config = tmp.path().join("config");
        fs::create_dir(&config).unwrap();
        let link = config.join("sites.toml");
        symlink(&real, &link).unwrap();

        write_atomic(&link, b"new").unwrap();

        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read_link(&link).unwrap(), real);
        assert_eq!(fs::read(&real).unwrap(), b"new");
        assert_eq!(mode(&real), 0o640, "target permissions kept");
        assert_eq!(names(&config), ["sites.toml"]);
        assert_eq!(names(&dotfiles), ["sites.toml"]);
    }

    #[cfg(unix)]
    #[test]
    fn relative_symlink_chain_is_followed() {
        use std::os::unix::fs::symlink;

        let tmp = tempfile::tempdir().unwrap();
        let dotfiles = tmp.path().join("dotfiles");
        let config = tmp.path().join("config");
        fs::create_dir(&dotfiles).unwrap();
        fs::create_dir(&config).unwrap();
        fs::write(dotfiles.join("real.toml"), b"old").unwrap();
        // config/sites.toml -> hop.toml -> ../dotfiles/real.toml
        symlink("../dotfiles/real.toml", config.join("hop.toml")).unwrap();
        symlink("hop.toml", config.join("sites.toml")).unwrap();

        write_atomic(&config.join("sites.toml"), b"new").unwrap();

        assert_eq!(fs::read(dotfiles.join("real.toml")).unwrap(), b"new");
        let link = fs::symlink_metadata(config.join("sites.toml")).unwrap();
        assert!(link.file_type().is_symlink());
        assert_eq!(names(&config), ["hop.toml", "sites.toml"]);
        assert_eq!(names(&dotfiles), ["real.toml"]);
    }

    #[cfg(unix)]
    #[test]
    fn dangling_symlink_creates_its_target() {
        use std::os::unix::fs::symlink;

        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().join("dotfiles").join("sites.toml");
        let link = tmp.path().join("sites.toml");
        symlink(&real, &link).unwrap();

        write_atomic(&link, b"new").unwrap();

        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read(&real).unwrap(), b"new");
        assert_eq!(mode(&real), 0o600, "new file is private");
    }

    #[cfg(unix)]
    #[test]
    fn symlink_loop_is_an_error() {
        use std::os::unix::fs::symlink;

        let tmp = tempfile::tempdir().unwrap();
        symlink("b", tmp.path().join("a")).unwrap();
        symlink("a", tmp.path().join("b")).unwrap();
        assert!(write_atomic(&tmp.path().join("a"), b"x").is_err());
        assert_eq!(names(tmp.path()), ["a", "b"]);
    }
}
