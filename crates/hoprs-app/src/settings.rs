//! The app's settings: `settings.toml`, beside the profiles.
//!
//! A missing file means the defaults. A file that cannot be read or parsed also gives
//! the defaults, with the error to show inline; it is never overwritten (the app does not
//! save settings while it is unreadable). A workers value out of range is clamped, not an
//! error.

use std::path::Path;

use hoprs_core::{Error, ErrorKind, Result};
use hoprs_transfer::ConflictPolicy;
use serde::{Deserialize, Serialize};

pub const FILE_NAME: &str = "settings.toml";
pub const WORKERS_MIN: usize = 1;
pub const WORKERS_MAX: usize = 16;
pub const WORKERS_DEFAULT: usize = 4;
/// Largest `settings.toml` read; a real one is under 100 bytes.
const MAX_BYTES: u64 = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    /// Transfer workers per site (`1..=16`), for the connections made from now on.
    pub workers_per_site: usize,
    /// What a transfer does when its destination exists.
    pub default_policy: ConflictPolicy,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            workers_per_site: WORKERS_DEFAULT,
            default_policy: ConflictPolicy::Ask,
        }
    }
}

/// `n` within `1..=16`.
pub fn clamp_workers(n: usize) -> usize {
    n.clamp(WORKERS_MIN, WORKERS_MAX)
}

/// What [`load`] found: the settings to use and, if the file was unusable, why (the
/// settings are then the defaults).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SettingsLoad {
    pub settings: Settings,
    pub error: Option<Error>,
}

/// The file's layout. Unknown keys are ignored (a newer hoprs may add some).
#[derive(Debug, Default, Deserialize, Serialize)]
struct File {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    workers_per_site: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    default_policy: Option<Policy>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
enum Policy {
    Ask,
    Overwrite,
    Skip,
    OverwriteIfNewer,
    Resume,
}

impl From<ConflictPolicy> for Policy {
    fn from(policy: ConflictPolicy) -> Self {
        match policy {
            ConflictPolicy::Ask => Self::Ask,
            ConflictPolicy::Overwrite => Self::Overwrite,
            ConflictPolicy::Skip => Self::Skip,
            ConflictPolicy::OverwriteIfNewer => Self::OverwriteIfNewer,
            ConflictPolicy::Resume => Self::Resume,
        }
    }
}

impl From<Policy> for ConflictPolicy {
    fn from(policy: Policy) -> Self {
        match policy {
            Policy::Ask => Self::Ask,
            Policy::Overwrite => Self::Overwrite,
            Policy::Skip => Self::Skip,
            Policy::OverwriteIfNewer => Self::OverwriteIfNewer,
            Policy::Resume => Self::Resume,
        }
    }
}

/// Parses `settings.toml` content; missing keys take their defaults.
pub fn parse(content: &str) -> Result<Settings> {
    let file: File = toml::from_str(content).map_err(|err| {
        Error::new(ErrorKind::Protocol, "Can't read settings.toml").with_detail(err.message())
    })?;
    let defaults = Settings::default();
    Ok(Settings {
        workers_per_site: file
            .workers_per_site
            .map_or(defaults.workers_per_site, |n| {
                clamp_workers(usize::try_from(n.max(0)).unwrap_or(WORKERS_MAX))
            }),
        default_policy: file
            .default_policy
            .map_or(defaults.default_policy, ConflictPolicy::from),
    })
}

/// The file content for `settings`.
pub fn render(settings: &Settings) -> String {
    let file = File {
        workers_per_site: i64::try_from(clamp_workers(settings.workers_per_site)).ok(),
        default_policy: Some(settings.default_policy.into()),
    };
    toml::to_string(&file).expect("plain keys always serialize")
}

/// Reads `path`. Blocking (small file): the backend calls it once when it opens.
pub fn load(path: &Path) -> SettingsLoad {
    match read(path) {
        Ok(Some(content)) => match parse(&content) {
            Ok(settings) => SettingsLoad {
                settings,
                error: None,
            },
            Err(error) => SettingsLoad {
                settings: Settings::default(),
                error: Some(error),
            },
        },
        Ok(None) => SettingsLoad::default(),
        Err(error) => SettingsLoad {
            settings: Settings::default(),
            error: Some(error),
        },
    }
}

/// The file's text, `None` if it does not exist.
fn read(path: &Path) -> Result<Option<String>> {
    use std::io::Read;

    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(cannot_read(err.to_string())),
    };
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|err| cannot_read(err.to_string()))?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(cannot_read(format!("larger than {MAX_BYTES} bytes")));
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| cannot_read("not valid UTF-8".to_owned()))
}

fn cannot_read(detail: String) -> Error {
    Error::new(ErrorKind::Io, "Can't read settings.toml").with_detail(detail)
}

/// Writes `settings` to `path` atomically (the vault's crash-safe write). Blocking.
pub fn save(path: &Path, settings: &Settings) -> Result<()> {
    hoprs_vault::write_atomic(path, render(settings).as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_roundtrip_and_invalid_file_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("profiles").join(FILE_NAME);

        // No file: the defaults, no error, nothing created.
        assert_eq!(load(&path), SettingsLoad::default());
        assert_eq!(
            Settings::default(),
            Settings {
                workers_per_site: 4,
                default_policy: ConflictPolicy::Ask,
            }
        );
        assert!(!path.exists());

        // Every policy survives a save and a load.
        for policy in [
            ConflictPolicy::Ask,
            ConflictPolicy::Overwrite,
            ConflictPolicy::Skip,
            ConflictPolicy::OverwriteIfNewer,
            ConflictPolicy::Resume,
        ] {
            let settings = Settings {
                workers_per_site: 7,
                default_policy: policy,
            };
            save(&path, &settings).unwrap();
            assert_eq!(
                load(&path),
                SettingsLoad {
                    settings,
                    error: None,
                }
            );
        }
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("workers_per_site = 7"), "{text}");
        assert!(text.contains("default_policy = \"resume\""), "{text}");

        // Missing keys take their defaults; unknown keys are ignored.
        std::fs::write(&path, "default_policy = \"skip\"\nfuture_key = 1\n").unwrap();
        assert_eq!(
            load(&path).settings,
            Settings {
                workers_per_site: 4,
                default_policy: ConflictPolicy::Skip,
            }
        );

        // Invalid files: the defaults plus an error, and the file is left as it was.
        for bad in [
            "workers_per_site = \"many\"\n",
            "default_policy = \"sometimes\"\n",
            "this is not toml",
        ] {
            std::fs::write(&path, bad).unwrap();
            let loaded = load(&path);
            assert_eq!(loaded.settings, Settings::default(), "{bad}");
            let error = loaded.error.expect("an invalid file is reported");
            assert_eq!(error.message, "Can't read settings.toml");
            assert!(error.detail.is_some());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), bad);
        }
        std::fs::write(&path, [0xff, 0xfe, 0x00]).unwrap();
        assert!(load(&path).error.is_some());
        assert_eq!(std::fs::read(&path).unwrap(), [0xff, 0xfe, 0x00]);
    }

    #[test]
    fn workers_clamped() {
        assert_eq!(clamp_workers(0), 1);
        assert_eq!(clamp_workers(1), 1);
        assert_eq!(clamp_workers(9), 9);
        assert_eq!(clamp_workers(16), 16);
        assert_eq!(clamp_workers(17), 16);
        assert_eq!(clamp_workers(usize::MAX), 16);
        for (file, expected) in [
            ("workers_per_site = 0", 1),
            ("workers_per_site = -3", 1),
            ("workers_per_site = 99", 16),
            ("workers_per_site = 9223372036854775807", 16),
            ("workers_per_site = 12", 12),
        ] {
            assert_eq!(parse(file).unwrap().workers_per_site, expected, "{file}");
        }
        // Saving clamps too: the file never holds an out-of-range value.
        let rendered = render(&Settings {
            workers_per_site: 40,
            default_policy: ConflictPolicy::Ask,
        });
        assert_eq!(parse(&rendered).unwrap().workers_per_site, 16);
    }
}
