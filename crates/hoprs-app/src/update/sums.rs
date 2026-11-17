//! SHA256SUMS parsing and file verification.

use std::io::Read;
use std::path::Path;

use hoprs_core::{Error, ErrorKind};
use sha2::{Digest, Sha256};

fn decode(hex: &str) -> Option<[u8; 32]> {
    // Checked first: `from_str_radix` would also take a leading `+`.
    if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = [0u8; 32];
    for (byte, pair) in out.iter_mut().zip(hex.as_bytes().chunks(2)) {
        *byte = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
    }
    Some(out)
}

/// The checksum listed for `name`: lines `<64 hex>  <name>` or `<64 hex> *<name>` (a
/// leading BOM and CRLF line ends are fine); other lines are skipped. The same name listed
/// twice with different checksums is ambiguous and an error.
pub fn expected_for(sums: &str, name: &str) -> Result<[u8; 32], Error> {
    let sums = sums.strip_prefix('\u{feff}').unwrap_or(sums);
    let mut found: Option<[u8; 32]> = None;
    for line in sums.lines() {
        let Some((hex, rest)) = line.split_once(' ') else {
            continue;
        };
        let listed = rest.strip_prefix(' ').or_else(|| rest.strip_prefix('*'));
        if listed != Some(name) {
            continue;
        }
        let Some(sum) = decode(hex) else { continue };
        match found {
            Some(other) if other != sum => {
                return Err(Error::coded(
                    ErrorKind::Protocol,
                    "update-bad-response",
                    "The update server gave an unexpected answer",
                )
                .with_detail(format!(
                    "{name} is listed twice in SHA256SUMS with different checksums"
                )));
            }
            _ => found = Some(sum),
        }
    }
    found.ok_or_else(|| {
        Error::coded(
            ErrorKind::Protocol,
            "update-checksum-missing",
            format!("{name} is not in the release's SHA256SUMS"),
        )
        .arg("name", name)
    })
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

    #[test]
    fn sums_parser_edge_cases() {
        let h1 = "a".repeat(64);
        let h2 = "b".repeat(64);
        let sum = |c: char| [u8::from_str_radix(&format!("{c}{c}"), 16).unwrap(); 32];
        // CRLF, uppercase hex, BOM.
        let text = format!("\u{feff}{}  a.deb\r\n{h2} *b.msi\r\n", h1.to_uppercase());
        assert_eq!(expected_for(&text, "a.deb").unwrap(), sum('a'));
        assert_eq!(expected_for(&text, "b.msi").unwrap(), sum('b'));
        // Exact name match; names with spaces.
        let text = format!("{h1}  xa.deb\n{h2}  my app.dmg\n");
        assert_eq!(
            expected_for(&text, "a.deb").unwrap_err().code,
            Some("update-checksum-missing")
        );
        assert_eq!(expected_for(&text, "my app.dmg").unwrap(), sum('b'));
        // Short, long and non-hex hashes do not count.
        for bad in [
            "a".repeat(63),
            "a".repeat(65),
            format!("+{}", "a".repeat(63)),
            "g".repeat(64),
        ] {
            let text = format!("{bad}  a.deb\n");
            assert_eq!(
                expected_for(&text, "a.deb").unwrap_err().code,
                Some("update-checksum-missing"),
                "{bad}"
            );
        }
        // Duplicates: identical are fine, conflicting are ambiguous.
        let same = format!("{h1}  a.deb\n{h1} *a.deb\n");
        assert_eq!(expected_for(&same, "a.deb").unwrap(), sum('a'));
        let conflict = format!("{h1}  a.deb\n{h2}  a.deb\n");
        assert_eq!(
            expected_for(&conflict, "a.deb").unwrap_err().code,
            Some("update-bad-response")
        );
    }
}
