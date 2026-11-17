//! SHA256SUMS parsing and file verification.

use std::io::Read;
use std::path::Path;

use hoprs_core::{Error, ErrorKind};
use sha2::{Digest, Sha256};

fn decode(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 || !hex.is_ascii() {
        return None;
    }
    let mut out = [0u8; 32];
    for (byte, pair) in out.iter_mut().zip(hex.as_bytes().chunks(2)) {
        *byte = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
    }
    Some(out)
}

/// The checksum listed for `name`: lines `<64 hex>  <name>` or `<64 hex> *<name>`;
/// other lines are skipped.
pub fn expected_for(sums: &str, name: &str) -> Result<[u8; 32], Error> {
    for line in sums.lines() {
        let Some((hex, rest)) = line.split_once(' ') else {
            continue;
        };
        let listed = rest.strip_prefix(' ').or_else(|| rest.strip_prefix('*'));
        if listed == Some(name)
            && let Some(sum) = decode(hex)
        {
            return Ok(sum);
        }
    }
    Err(Error::coded(
        ErrorKind::Protocol,
        "update-checksum-missing",
        format!("{name} is not in the release's SHA256SUMS"),
    )
    .arg("name", name))
}

/// SHA-256 of the file, read in 64 KiB pieces.
pub fn sha256_file(path: &Path) -> std::io::Result<[u8; 32]> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    Ok(hasher.finalize().into())
}

/// Ok when the file's SHA-256 is `expected`.
pub fn verify(path: &Path, expected: &[u8; 32]) -> Result<(), Error> {
    if &sha256_file(path)? == expected {
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
    use sha2::{Digest, Sha256};

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn sums_parse_and_verify() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.deb");
        std::fs::write(&file, b"hello").unwrap();
        let other = dir.path().join("b.deb");
        std::fs::write(&other, b"world").unwrap();
        let sha = hex(&Sha256::digest(b"hello"));
        let sums = format!("garbage line\n{sha}  a.deb\n{sha} *b.msi\n");
        let a = expected_for(&sums, "a.deb").unwrap();
        let b = expected_for(&sums, "b.msi").unwrap();
        assert_eq!(a, b);
        assert_eq!(sha256_file(&file).unwrap(), a);
        verify(&file, &a).unwrap();
        let err = verify(&other, &a).unwrap_err();
        assert_eq!(err.code, Some("update-checksum-mismatch"));
        let err = expected_for(&sums, "c.dmg").unwrap_err();
        assert_eq!(err.code, Some("update-checksum-missing"));
    }
}
