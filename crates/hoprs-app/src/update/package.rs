//! Which package this install came from, and what its release asset is called.

use semver::Version;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)] // `MacOs` is the OS's name, not a repeat of `Os`.
pub enum Os {
    Linux,
    Windows,
    MacOs,
    Other,
}

impl Os {
    pub fn current() -> Os {
        if cfg!(target_os = "linux") {
            Os::Linux
        } else if cfg!(windows) {
            Os::Windows
        } else if cfg!(target_os = "macos") {
            Os::MacOs
        } else {
            Os::Other
        }
    }
}

/// The installer format that updates this install.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PackageKind {
    Deb,
    Msi,
    Dmg,
}

/// The package `exe` was installed by, if any. Strings, not paths, so Windows paths are
/// testable everywhere. `.deb` and `.msi` ship for x86_64 only; Windows counts only
/// `<program files>\hoprs\hoprs.exe`.
pub fn package_for(os: Os, x86_64: bool, exe: &str, program_files: &[&str]) -> Option<PackageKind> {
    match os {
        Os::Linux => (x86_64 && exe == "/usr/bin/hoprs").then_some(PackageKind::Deb),
        Os::MacOs => exe
            .ends_with(".app/Contents/MacOS/hoprs")
            .then_some(PackageKind::Dmg),
        Os::Windows => {
            let norm = |s: &str| {
                // `canonicalize` gives `\\?\C:\...` (or `\\?\UNC\server\...`) on Windows.
                let s = s.replace('/', "\\");
                let s = match s.strip_prefix(r"\\?\UNC\") {
                    Some(rest) => format!(r"\\{rest}"),
                    None => s.strip_prefix(r"\\?\").unwrap_or(&s).to_owned(),
                };
                s.trim_end_matches('\\').to_lowercase()
            };
            let exe = norm(exe);
            (x86_64
                && program_files.iter().any(|dir| {
                    let dir = norm(dir);
                    !dir.is_empty() && exe == format!("{dir}\\hoprs\\hoprs.exe")
                }))
            .then_some(PackageKind::Msi)
        }
        Os::Other => None,
    }
}

/// The release asset for `kind` at `version` (`.github/workflows/release.yml` names them).
pub fn asset_name(kind: PackageKind, version: &Version) -> String {
    match kind {
        PackageKind::Deb => format!("hoprs_{version}-1_amd64.deb"),
        PackageKind::Msi => format!("hoprs_{version}_windows_x64.msi"),
        PackageKind::Dmg => format!("hoprs_{version}_macos_universal.dmg"),
    }
}

/// The package the running binary came from.
pub fn current_package() -> Option<PackageKind> {
    let exe = std::env::current_exe().ok()?;
    let exe = exe.canonicalize().unwrap_or(exe);
    let exe = exe.to_str()?;
    let env: Vec<String> = ["ProgramW6432", "ProgramFiles"]
        .iter()
        .filter_map(|name| std::env::var(name).ok())
        .collect();
    let dirs: Vec<&str> = env.iter().map(String::as_str).collect();
    package_for(Os::current(), cfg!(target_arch = "x86_64"), exe, &dirs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use semver::Version;

    #[test]
    fn package_per_os_and_path() {
        let pf = [r"C:\Program Files"];
        assert_eq!(
            package_for(Os::Linux, true, "/usr/bin/hoprs", &[]),
            Some(PackageKind::Deb)
        );
        assert_eq!(package_for(Os::Linux, false, "/usr/bin/hoprs", &[]), None);
        assert_eq!(
            package_for(
                Os::Linux,
                true,
                "/home/u/src/hoprs/target/release/hoprs",
                &[]
            ),
            None
        );
        assert_eq!(
            package_for(Os::Windows, true, r"C:\Program Files\hoprs\hoprs.exe", &pf),
            Some(PackageKind::Msi)
        );
        assert_eq!(
            package_for(Os::Windows, true, r"c:\program files\HOPRS\hoprs.exe", &pf),
            Some(PackageKind::Msi)
        );
        assert_eq!(
            package_for(
                Os::Windows,
                true,
                r"\\?\C:\Program Files\hoprs\hoprs.exe",
                &pf
            ),
            Some(PackageKind::Msi)
        );
        assert_eq!(
            package_for(Os::Windows, true, r"\\?\C:\Users\u\hoprs\hoprs.exe", &pf),
            None
        );
        assert_eq!(
            package_for(Os::Windows, true, r"\\?\UNC\srv\share\hoprs\hoprs.exe", &pf),
            None
        );
        assert_eq!(
            package_for(
                Os::Windows,
                true,
                r"C:\Users\u\Downloads\hoprs_1.3.1_windows_x64\hoprs.exe",
                &pf
            ),
            None
        );
        assert_eq!(
            package_for(
                Os::MacOs,
                false,
                "/Applications/hoprs.app/Contents/MacOS/hoprs",
                &[]
            ),
            Some(PackageKind::Dmg)
        );
        assert_eq!(package_for(Os::MacOs, true, "/Users/u/hoprs", &[]), None);
        assert_eq!(package_for(Os::Other, true, "/usr/bin/hoprs", &[]), None);
    }

    #[test]
    fn asset_names_follow_release_naming() {
        let v = Version::new(1, 4, 0);
        assert_eq!(asset_name(PackageKind::Deb, &v), "hoprs_1.4.0-1_amd64.deb");
        assert_eq!(
            asset_name(PackageKind::Msi, &v),
            "hoprs_1.4.0_windows_x64.msi"
        );
        assert_eq!(
            asset_name(PackageKind::Dmg, &v),
            "hoprs_1.4.0_macos_universal.dmg"
        );
    }
}
