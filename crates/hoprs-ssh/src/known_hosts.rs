//! OpenSSH-compatible `known_hosts` handling: check, learn, remove and SHA256 fingerprints.

use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD};
use hmac::{Hmac, Mac};
use hoprs_core::Result;
use russh::keys::{Algorithm, PublicKey};
use sha1::Sha1;
use sha2::{Digest, Sha256};

/// Outcome of looking a host key up in a `known_hosts` file.
#[derive(Debug, PartialEq, Eq)]
pub enum HostKeyStatus {
    /// The exact key is recorded for this host.
    Known,
    /// No entry with this key algorithm exists for this host. `other_key_types` lists the
    /// algorithms (`ssh-ed25519`, `ecdsa-sha2-nistp256`, ...) of the keys recorded for it, in
    /// file order (`@revoked` entries excluded): empty means the host is really new; otherwise
    /// the server presents a key of a type not seen before, which deserves a warning.
    Unknown { other_key_types: Vec<String> },
    /// An entry with the same algorithm but a different key exists (1-based line).
    Changed { line: usize },
    /// A `@revoked` entry lists this exact key for this host (1-based line). Must refuse.
    Revoked { line: usize },
}

/// `SHA256:<base64 without padding>` of the key's wire-format blob, like `ssh-keygen -lf`.
pub fn fingerprint(key: &PublicKey) -> String {
    let digest = Sha256::digest(wire_blob(key));
    format!("SHA256:{}", STANDARD_NO_PAD.encode(digest))
}

/// Looks `host:port` up in the `known_hosts` file at `path`. A missing file is `Unknown`
/// with no other key types.
pub fn check(path: &Path, host: &str, port: u16, key: &PublicKey) -> Result<HostKeyStatus> {
    let Some(data) = read_optional(path)? else {
        return Ok(HostKeyStatus::Unknown {
            other_key_types: Vec::new(),
        });
    };
    let name = host_name(host, port);
    let mut changed = None;
    let mut known = false;
    let mut others: Vec<String> = Vec::new();
    for (idx, raw) in data.split_inclusive(|b| *b == b'\n').enumerate() {
        let Some(entry) = parse_line(raw) else {
            continue;
        };
        if !patterns_match(entry.patterns, &name) {
            continue;
        }
        if entry.revoked {
            if entry.key.key_data() == key.key_data() {
                return Ok(HostKeyStatus::Revoked { line: idx + 1 });
            }
            continue;
        }
        if entry.key.key_data() == key.key_data() {
            known = true;
        } else if entry.key.algorithm() == key.algorithm() {
            changed.get_or_insert(idx + 1);
        } else {
            let other = entry.key.algorithm().as_str().to_owned();
            if !others.contains(&other) {
                others.push(other);
            }
        }
    }
    if known {
        return Ok(HostKeyStatus::Known);
    }
    Ok(match changed {
        Some(line) => HostKeyStatus::Changed { line },
        None => HostKeyStatus::Unknown {
            other_key_types: others,
        },
    })
}

/// Algorithms of the keys recorded for `host:port` (`@revoked` lines excluded), in file
/// order, without duplicates. A missing or unreadable file yields none.
pub fn known_algorithms(path: &Path, host: &str, port: u16) -> Vec<Algorithm> {
    let Ok(Some(data)) = read_optional(path) else {
        return Vec::new();
    };
    let name = host_name(host, port);
    let mut found: Vec<Algorithm> = Vec::new();
    for raw in data.split_inclusive(|b| *b == b'\n') {
        let Some(entry) = parse_line(raw) else {
            continue;
        };
        let algorithm = entry.key.algorithm();
        if !entry.revoked && patterns_match(entry.patterns, &name) && !found.contains(&algorithm) {
            found.push(algorithm);
        }
    }
    found
}

/// Appends a plain-text entry for `host:port`, creating the file and parent directory if needed.
pub fn learn(path: &Path, host: &str, port: u16, key: &PublicKey) -> Result<()> {
    validate_host(host)?;
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        create_parent_dir(parent)?;
    }
    let mut options = fs::OpenOptions::new();
    options.read(true).append(true).create(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(path)?;

    let mut entry = String::new();
    if file.metadata()?.len() > 0 {
        let mut last = [0u8; 1];
        file.seek(SeekFrom::End(-1))?;
        file.read_exact(&mut last)?;
        if last[0] != b'\n' {
            entry.push('\n');
        }
    }
    entry.push_str(&format!(
        "{} {} {}\n",
        host_name(host, port),
        key.algorithm().as_str(),
        STANDARD.encode(wire_blob(key))
    ));
    file.write_all(entry.as_bytes())?;
    Ok(())
}

/// Deletes every line matching `host:port` (hashed ones included); returns how many were removed.
pub fn remove(path: &Path, host: &str, port: u16) -> Result<usize> {
    let Some(data) = read_optional(path)? else {
        return Ok(0);
    };
    let name = host_name(host, port);
    let mut kept = Vec::with_capacity(data.len());
    let mut removed = 0;
    for raw in data.split_inclusive(|b| *b == b'\n') {
        match parse_line(raw) {
            Some(entry) if !entry.revoked && patterns_match(entry.patterns, &name) => removed += 1,
            _ => kept.extend_from_slice(raw),
        }
    }
    if removed > 0 {
        write_atomic(path, &kept)?;
    }
    Ok(removed)
}

/// One `known_hosts` line that applies to a host, as [`entries_for`] reports it: what
/// [`remove`] would delete, for the UI to show before it does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnownHostEntry {
    /// 1-based line number.
    pub line: usize,
    /// The key's algorithm (`ssh-ed25519`, `ecdsa-sha2-nistp256`, ...).
    pub key_type: String,
    /// The line also names other hosts: a comma list with other names, or a wildcard or
    /// negated pattern. Removing it forgets the key for those hosts too.
    pub covers_other_hosts: bool,
    /// The host field is hashed (`|1|salt|hash`): it names exactly one host, which cannot
    /// be read back from the file.
    pub hashed: bool,
}

/// The lines [`remove`] would delete for `host:port` (same matching: plain, pattern and
/// hashed lines; `@revoked` and other marker lines excluded), in file order. Read-only. A
/// missing file has none.
pub fn entries_for(path: &Path, host: &str, port: u16) -> Result<Vec<KnownHostEntry>> {
    let Some(data) = read_optional(path)? else {
        return Ok(Vec::new());
    };
    let name = host_name(host, port);
    let mut entries = Vec::new();
    for (idx, raw) in data.split_inclusive(|b| *b == b'\n').enumerate() {
        let Some(entry) = parse_line(raw) else {
            continue;
        };
        if entry.revoked || !patterns_match(entry.patterns, &name) {
            continue;
        }
        let hashed = entry.patterns.starts_with("|1|");
        // A hashed field is one host; otherwise any pattern other than this exact name
        // (another name, a wildcard, a negation) reaches other hosts.
        let covers_other_hosts = !hashed
            && entry
                .patterns
                .split(',')
                .any(|pattern| !pattern.eq_ignore_ascii_case(&name));
        entries.push(KnownHostEntry {
            line: idx + 1,
            key_type: entry.key.algorithm().as_str().to_owned(),
            covers_other_hosts,
            hashed,
        });
    }
    Ok(entries)
}

struct Entry<'a> {
    revoked: bool,
    patterns: &'a str,
    key: PublicKey,
}

fn wire_blob(key: &PublicKey) -> Vec<u8> {
    // Encoding an in-memory key to a Vec cannot fail.
    key.to_bytes().unwrap_or_default()
}

/// Canonical entry name: ASCII-lowercased (OpenSSH hashes the lowercase name), bracketed off port 22.
fn host_name(host: &str, port: u16) -> String {
    let host = host.to_ascii_lowercase();
    if port == 22 {
        host
    } else {
        format!("[{host}]:{port}")
    }
}

fn validate_host(host: &str) -> Result<()> {
    let bad = host.is_empty()
        || host.starts_with(['@', '#', '|'])
        || host.contains(|c: char| c.is_whitespace() || c.is_control() || ",*?!".contains(c));
    if bad {
        return Err(hoprs_core::Error::new(
            hoprs_core::ErrorKind::Io,
            "Invalid host name for known_hosts",
        ));
    }
    Ok(())
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(data) => Ok(Some(data)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err.into()),
    }
}

/// Parses one line. Comments, blank lines, `@marker` lines and malformed lines yield `None`.
fn parse_line(raw: &[u8]) -> Option<Entry<'_>> {
    let line = std::str::from_utf8(raw).ok()?.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let mut fields = line.split_whitespace().peekable();
    let revoked = match fields.peek() {
        Some(&"@revoked") => {
            fields.next();
            true
        }
        Some(f) if f.starts_with('@') => return None,
        _ => false,
    };
    let patterns = fields.next()?;
    let algorithm = fields.next()?;
    let blob = STANDARD.decode(fields.next()?).ok()?;
    let key = PublicKey::from_bytes(&blob).ok()?;
    (key.algorithm().as_str() == algorithm).then_some(Entry {
        revoked,
        patterns,
        key,
    })
}

fn patterns_match(patterns: &str, name: &str) -> bool {
    if patterns.starts_with("|1|") {
        return hashed_match(patterns, name);
    }
    let mut matched = false;
    for pattern in patterns.split(',') {
        if let Some(negated) = pattern.strip_prefix('!') {
            if glob_match(negated.as_bytes(), name.as_bytes()) {
                return false;
            }
        } else if glob_match(pattern.as_bytes(), name.as_bytes()) {
            matched = true;
        }
    }
    matched
}

fn hashed_match(field: &str, name: &str) -> bool {
    let mut parts = field.split('|').skip(2);
    let (Some(salt), Some(hash)) = (parts.next(), parts.next()) else {
        return false;
    };
    let (Ok(salt), Ok(hash)) = (STANDARD.decode(salt), STANDARD.decode(hash)) else {
        return false;
    };
    let Ok(mut mac) = Hmac::<Sha1>::new_from_slice(&salt) else {
        return false;
    };
    mac.update(name.as_bytes());
    mac.verify_slice(&hash).is_ok()
}

/// Case-insensitive glob supporting `*` and `?`, as OpenSSH does for host patterns.
fn glob_match(pattern: &[u8], text: &[u8]) -> bool {
    let (mut p, mut t) = (0, 0);
    let mut backtrack: Option<(usize, usize)> = None;
    while t < text.len() {
        if p < pattern.len() && pattern[p] == b'*' {
            backtrack = Some((p, t));
            p += 1;
        } else if p < pattern.len()
            && (pattern[p] == b'?' || pattern[p].eq_ignore_ascii_case(&text[t]))
        {
            p += 1;
            t += 1;
        } else if let Some((star_p, star_t)) = backtrack {
            p = star_p + 1;
            t = star_t + 1;
            backtrack = Some((star_p, star_t + 1));
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|b| *b == b'*')
}

fn create_parent_dir(dir: &Path) -> Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(dir)?;
    Ok(())
}

/// Writes `data` to a temp file next to `path`, then renames it over `path`.
fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".hoprs-tmp-{}", std::process::id()));
    let tmp = path.with_file_name(name);
    let result = (|| -> std::io::Result<()> {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let mut file = options.open(&tmp)?;
        file.write_all(data)?;
        file.set_permissions(fs::metadata(path)?.permissions())?;
        file.sync_all()?;
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    Ok(result?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use russh::keys::{Algorithm, PrivateKey};
    use tempfile::TempDir;

    const FIXTURE_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");

    fn key(algorithm: Algorithm) -> PublicKey {
        PrivateKey::random(&mut rand::rng(), algorithm)
            .unwrap()
            .public_key()
            .clone()
    }

    /// `Unknown` with no other key type known for the host.
    fn new_host() -> HostKeyStatus {
        HostKeyStatus::Unknown {
            other_key_types: Vec::new(),
        }
    }

    fn ed() -> PublicKey {
        key(Algorithm::Ed25519)
    }

    fn fixture_key() -> PublicKey {
        let text = fs::read_to_string(format!("{FIXTURE_DIR}/id_ed25519.pub")).unwrap();
        PublicKey::from_openssh(&text).unwrap()
    }

    fn kh_path(dir: &TempDir) -> std::path::PathBuf {
        dir.path().join("known_hosts")
    }

    fn line_for(host: &str, key: &PublicKey) -> String {
        let b64 = STANDARD.encode(key.to_bytes().unwrap());
        format!("{host} {} {b64}\n", key.algorithm().as_str())
    }

    #[test]
    fn unknown_when_file_missing() {
        let dir = TempDir::new().unwrap();
        let status = check(&kh_path(&dir), "h", 22, &ed()).unwrap();
        assert_eq!(status, new_host());
    }

    #[test]
    fn learn_then_known() {
        let dir = TempDir::new().unwrap();
        let p = kh_path(&dir);
        let k = ed();
        learn(&p, "h", 22, &k).unwrap();
        assert_eq!(check(&p, "h", 22, &k).unwrap(), HostKeyStatus::Known);
        assert_eq!(fs::read_to_string(&p).unwrap(), line_for("h", &k));
    }

    #[test]
    fn learn_creates_parent_dir() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("a").join("b").join("known_hosts");
        let k = ed();
        learn(&p, "h", 22, &k).unwrap();
        assert_eq!(check(&p, "h", 22, &k).unwrap(), HostKeyStatus::Known);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let file_mode = fs::metadata(&p).unwrap().permissions().mode() & 0o777;
            let dir_mode = fs::metadata(p.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(file_mode, 0o600);
            assert_eq!(dir_mode, 0o700);
        }
    }

    #[test]
    fn learn_appends_newline_to_unterminated_file() {
        let dir = TempDir::new().unwrap();
        let p = kh_path(&dir);
        fs::write(&p, "# no trailing newline").unwrap();
        let k = ed();
        learn(&p, "h", 22, &k).unwrap();
        let text = fs::read_to_string(&p).unwrap();
        assert_eq!(
            text,
            format!("# no trailing newline\n{}", line_for("h", &k))
        );
        assert_eq!(check(&p, "h", 22, &k).unwrap(), HostKeyStatus::Known);
    }

    #[test]
    fn port_22_plain_name_and_other_port_bracketed() {
        let dir = TempDir::new().unwrap();
        let p = kh_path(&dir);
        let k = ed();
        learn(&p, "h", 2222, &k).unwrap();
        assert_eq!(fs::read_to_string(&p).unwrap(), line_for("[h]:2222", &k));
        assert_eq!(check(&p, "h", 22, &k).unwrap(), new_host());
        assert_eq!(check(&p, "h", 2222, &k).unwrap(), HostKeyStatus::Known);
    }

    #[test]
    fn changed_reports_line() {
        let dir = TempDir::new().unwrap();
        let p = kh_path(&dir);
        let (a, b) = (ed(), ed());
        let text = format!(
            "# c\nother {}\n{}",
            line_for("x", &a).trim_end().split_once(' ').unwrap().1,
            line_for("h", &a)
        );
        fs::write(&p, text).unwrap();
        assert_eq!(check(&p, "h", 22, &a).unwrap(), HostKeyStatus::Known);
        assert_eq!(
            check(&p, "h", 22, &b).unwrap(),
            HostKeyStatus::Changed { line: 3 }
        );
    }

    #[test]
    fn other_algorithm_is_unknown_not_changed() {
        let dir = TempDir::new().unwrap();
        let p = kh_path(&dir);
        let ecdsa = key(Algorithm::Ecdsa {
            curve: russh::keys::EcdsaCurve::NistP256,
        });
        learn(&p, "h", 22, &ecdsa).unwrap();
        assert_eq!(
            check(&p, "h", 22, &ed()).unwrap(),
            HostKeyStatus::Unknown {
                other_key_types: vec!["ecdsa-sha2-nistp256".to_owned()]
            }
        );
    }

    #[test]
    fn hashed_entry_matches() {
        let p = format!("{FIXTURE_DIR}/known_hosts");
        let p = Path::new(&p);
        let k = fixture_key();
        assert_eq!(
            check(p, "hashed.example", 22, &k).unwrap(),
            HostKeyStatus::Known
        );
        assert_eq!(check(p, "other.example", 22, &k).unwrap(), new_host());
        assert_eq!(
            check(p, "hashed.example", 22, &ed()).unwrap(),
            HostKeyStatus::Changed { line: 2 }
        );
    }

    #[test]
    fn comma_list_matches() {
        let dir = TempDir::new().unwrap();
        let p = kh_path(&dir);
        let k = ed();
        fs::write(&p, line_for("alpha.example,192.0.2.7,[beta]:2200", &k)).unwrap();
        assert_eq!(
            check(&p, "alpha.example", 22, &k).unwrap(),
            HostKeyStatus::Known
        );
        assert_eq!(
            check(&p, "192.0.2.7", 22, &k).unwrap(),
            HostKeyStatus::Known
        );
        assert_eq!(check(&p, "beta", 2200, &k).unwrap(), HostKeyStatus::Known);
        assert_eq!(check(&p, "gamma", 22, &k).unwrap(), new_host());
    }

    #[test]
    fn glob_patterns_match() {
        let dir = TempDir::new().unwrap();
        let p = kh_path(&dir);
        let k = ed();
        fs::write(&p, line_for("*.example.com,10.0.0.?", &k)).unwrap();
        assert_eq!(
            check(&p, "a.example.com", 22, &k).unwrap(),
            HostKeyStatus::Known
        );
        assert_eq!(check(&p, "10.0.0.5", 22, &k).unwrap(), HostKeyStatus::Known);
        assert_eq!(check(&p, "10.0.0.55", 22, &k).unwrap(), new_host());
        assert_eq!(check(&p, "example.org", 22, &k).unwrap(), new_host());
    }

    #[test]
    fn negation_excludes_host() {
        let dir = TempDir::new().unwrap();
        let p = kh_path(&dir);
        let k = ed();
        fs::write(&p, line_for("*.example.com,!bad.example.com", &k)).unwrap();
        assert_eq!(
            check(&p, "ok.example.com", 22, &k).unwrap(),
            HostKeyStatus::Known
        );
        assert_eq!(check(&p, "bad.example.com", 22, &k).unwrap(), new_host());
    }

    #[test]
    fn markers_comments_and_garbage_are_ignored() {
        let dir = TempDir::new().unwrap();
        let p = kh_path(&dir);
        let k = ed();
        let mut text = String::from("# comment\n\ngarbage line\nh ssh-ed25519 !!!notbase64\n");
        text.push_str(&format!("@cert-authority {}", line_for("h", &k)));
        fs::write(&p, text).unwrap();
        assert_eq!(check(&p, "h", 22, &k).unwrap(), new_host());
    }

    #[test]
    fn revoked_key_blocks() {
        let dir = TempDir::new().unwrap();
        let p = kh_path(&dir);
        let k = ed();
        let text = format!(
            "{}@revoked h {}",
            line_for("h", &k),
            &line_for("h", &k)[2..]
        );
        fs::write(&p, text).unwrap();
        assert_eq!(
            check(&p, "h", 22, &k).unwrap(),
            HostKeyStatus::Revoked { line: 2 }
        );
    }

    #[test]
    fn revoked_other_key_does_not_affect() {
        let dir = TempDir::new().unwrap();
        let p = kh_path(&dir);
        let (k, other) = (ed(), ed());
        let text = format!("@revoked {}{}", line_for("h", &other), line_for("h", &k));
        fs::write(&p, text).unwrap();
        assert_eq!(check(&p, "h", 22, &k).unwrap(), HostKeyStatus::Known);
        assert_eq!(
            check(&p, "h", 22, &ed()).unwrap(),
            HostKeyStatus::Changed { line: 2 }
        );
    }

    #[test]
    fn mixed_case_host_matches_hashed_entry() {
        let dir = TempDir::new().unwrap();
        let p = kh_path(&dir);
        fs::copy(format!("{FIXTURE_DIR}/known_hosts"), &p).unwrap();
        assert_eq!(
            check(&p, "Hashed.Example", 22, &fixture_key()).unwrap(),
            HostKeyStatus::Known
        );
        assert_eq!(
            check(&p, "HASHED.example", 22, &ed()).unwrap(),
            HostKeyStatus::Changed { line: 2 }
        );
        assert_eq!(remove(&p, "Hashed.EXAMPLE", 22).unwrap(), 1);
    }

    #[test]
    fn learn_rejects_corrupting_host_names() {
        let dir = TempDir::new().unwrap();
        let p = kh_path(&dir);
        let k = ed();
        for bad in [
            "a b", "a\tb", "a\nb", "a\x07b", "a,b", "a*", "a?", "!a", "@a", "#a", "|a", "",
        ] {
            let err = learn(&p, bad, 22, &k).unwrap_err();
            assert_eq!(err.kind, hoprs_core::ErrorKind::Io, "{bad:?}");
        }
        assert!(!p.exists());
    }

    #[test]
    fn second_key_for_same_host_is_known() {
        let dir = TempDir::new().unwrap();
        let p = kh_path(&dir);
        let (a, b) = (ed(), ed());
        fs::write(&p, format!("{}{}", line_for("h", &a), line_for("h", &b))).unwrap();
        assert_eq!(check(&p, "h", 22, &b).unwrap(), HostKeyStatus::Known);
    }

    #[test]
    fn remove_deletes_plain_and_hashed() {
        let dir = TempDir::new().unwrap();
        let p = kh_path(&dir);
        fs::copy(format!("{FIXTURE_DIR}/known_hosts"), &p).unwrap();
        let k = fixture_key();
        learn(&p, "hashed.example", 22, &k).unwrap();
        learn(&p, "keep.example", 22, &k).unwrap();
        assert_eq!(remove(&p, "hashed.example", 22).unwrap(), 2);
        assert_eq!(check(&p, "hashed.example", 22, &k).unwrap(), new_host());
        assert_eq!(
            check(&p, "keep.example", 22, &k).unwrap(),
            HostKeyStatus::Known
        );
    }

    #[test]
    fn remove_preserves_other_lines_byte_for_byte() {
        let dir = TempDir::new().unwrap();
        let p = kh_path(&dir);
        let k = ed();
        let mut data = b"# odd \xff bytes\r\nkeep ssh-ed25519 AAAA\r\n".to_vec();
        data.extend_from_slice(line_for("gone", &k).as_bytes());
        data.extend_from_slice(b"tail without newline");
        fs::write(&p, &data).unwrap();
        assert_eq!(remove(&p, "gone", 22).unwrap(), 1);
        assert_eq!(
            fs::read(&p).unwrap(),
            b"# odd \xff bytes\r\nkeep ssh-ed25519 AAAA\r\ntail without newline"
        );
        assert_eq!(remove(&p, "gone", 22).unwrap(), 0);
        assert_eq!(remove(&dir.path().join("missing"), "gone", 22).unwrap(), 0);
    }

    #[test]
    fn fingerprint_matches_ssh_keygen() {
        let expected = fs::read_to_string(format!("{FIXTURE_DIR}/fingerprint.txt")).unwrap();
        let expected = expected.split_whitespace().nth(1).unwrap();
        assert_eq!(fingerprint(&fixture_key()), expected);
    }

    #[test]
    fn unknown_reports_other_key_types() {
        let dir = TempDir::new().unwrap();
        let p = kh_path(&dir);
        let p256 = Algorithm::Ecdsa {
            curve: russh::keys::EcdsaCurve::NistP256,
        };
        let mut data = String::new();
        data.push_str(&line_for("h.example", &key(p256.clone())));
        let p384 = Algorithm::Ecdsa {
            curve: russh::keys::EcdsaCurve::NistP384,
        };
        data.push_str(&line_for("other.example", &key(p384.clone())));
        // Revoked entries are not "known".
        data.push_str(&format!("@revoked {}", line_for("h.example", &key(p384))));
        data.push_str(&line_for("h.example", &key(p256)));
        fs::write(&p, data).unwrap();
        assert_eq!(
            check(&p, "h.example", 22, &ed()).unwrap(),
            HostKeyStatus::Unknown {
                other_key_types: vec!["ecdsa-sha2-nistp256".to_owned()]
            }
        );
        // A host with no entry at all is really new.
        assert_eq!(
            check(&p, "new.example", 22, &ed()).unwrap(),
            HostKeyStatus::Unknown {
                other_key_types: vec![]
            }
        );
    }

    #[test]
    fn known_algorithms_lists_host_entries_in_order() {
        let dir = TempDir::new().unwrap();
        let p = kh_path(&dir);
        assert!(known_algorithms(&p, "h.example", 22).is_empty());
        let p256 = Algorithm::Ecdsa {
            curve: russh::keys::EcdsaCurve::NistP256,
        };
        let mut data = String::new();
        data.push_str(&line_for("other.example", &ed()));
        data.push_str(&line_for("h.example", &key(p256.clone())));
        data.push_str(&format!(
            "@revoked {}",
            line_for("h.example", &key(Algorithm::Ed25519))
        ));
        data.push_str(&line_for("h.example,x", &ed()));
        data.push_str(&line_for("h.example", &key(p256.clone())));
        fs::write(&p, data).unwrap();
        assert_eq!(
            known_algorithms(&p, "h.example", 22),
            vec![p256, Algorithm::Ed25519]
        );
        assert!(known_algorithms(&p, "h.example", 2222).is_empty());
    }

    #[test]
    fn entries_for_lists_what_remove_deletes() {
        let dir = TempDir::new().unwrap();
        let p = kh_path(&dir);
        assert!(entries_for(&p, "h.example", 22).unwrap().is_empty());
        let p256 = Algorithm::Ecdsa {
            curve: russh::keys::EcdsaCurve::NistP256,
        };
        let mut data = String::from("# comment\n");
        // 2: exactly this host.
        data.push_str(&line_for("h.example", &ed()));
        // 3: another host only.
        data.push_str(&line_for("other.example", &ed()));
        // 4: a comma list naming another host too.
        data.push_str(&line_for("h.example,192.0.2.7", &key(p256.clone())));
        // 5: a wildcard pattern.
        data.push_str(&line_for("*.example", &ed()));
        // 6: revoked lines are never removed.
        data.push_str(&format!("@revoked {}", line_for("h.example", &ed())));
        // 7: the same name twice is still only this host (any case).
        data.push_str(&line_for("h.example,H.EXAMPLE", &ed()));
        // 8: a negation excludes this host.
        data.push_str(&line_for("*.example,!h.example", &ed()));
        fs::write(&p, data).unwrap();

        let entries = entries_for(&p, "H.example", 22).unwrap();
        let summary: Vec<_> = entries
            .iter()
            .map(|e| (e.line, e.key_type.as_str(), e.covers_other_hosts, e.hashed))
            .collect();
        assert_eq!(
            summary,
            [
                (2, "ssh-ed25519", false, false),
                (4, "ecdsa-sha2-nistp256", true, false),
                (5, "ssh-ed25519", true, false),
                (7, "ssh-ed25519", false, false),
            ]
        );
        // Read-only, and exactly the lines `remove` deletes.
        let before = fs::read(&p).unwrap();
        entries_for(&p, "h.example", 22).unwrap();
        assert_eq!(fs::read(&p).unwrap(), before);
        assert_eq!(remove(&p, "h.example", 22).unwrap(), entries.len());
        assert!(entries_for(&p, "h.example", 22).unwrap().is_empty());
    }

    #[test]
    fn entries_for_bracketed_port_and_hashed_lines() {
        let dir = TempDir::new().unwrap();
        let p = kh_path(&dir);
        fs::copy(format!("{FIXTURE_DIR}/known_hosts"), &p).unwrap();
        let k = ed();
        learn(&p, "h", 2222, &k).unwrap();
        let mut text = fs::read_to_string(&p).unwrap();
        text.push_str(&line_for("[h]:2222,[h]:2200", &k));
        fs::write(&p, text).unwrap();

        // A hashed line names one host: removing it touches no other.
        assert_eq!(
            entries_for(&p, "hashed.example", 22).unwrap(),
            [KnownHostEntry {
                line: 2,
                key_type: "ssh-ed25519".into(),
                covers_other_hosts: false,
                hashed: true,
            }]
        );
        // `[host]:port` off port 22; the plain name on port 22 is another entry.
        let entries = entries_for(&p, "h", 2222).unwrap();
        let lines: Vec<_> = entries
            .iter()
            .map(|e| (e.line, e.covers_other_hosts, e.hashed))
            .collect();
        assert_eq!(lines, [(3, false, false), (4, true, false)]);
        assert!(entries_for(&p, "h", 22).unwrap().is_empty());
    }
}
