//! The app's settings: `settings.toml`, beside the profiles.
//!
//! A missing file means the defaults. A file that cannot be read or parsed also gives
//! the defaults, with the error to show inline; it is never overwritten (the app does not
//! save settings while it is unreadable). A value out of range is clamped, not an error;
//! an unknown terminal theme falls back to the default. Saving keeps the keys this
//! version does not know (a newer hoprs may have written them).

use std::path::Path;

use hoprs_core::{Error, ErrorKind, Result};
use hoprs_transfer::ConflictPolicy;
use serde::{Deserialize, Serialize};

use crate::terminal::TermTheme;

pub const FILE_NAME: &str = "settings.toml";
pub const WORKERS_MIN: usize = 1;
pub const WORKERS_MAX: usize = 16;
pub const WORKERS_DEFAULT: usize = 4;
/// Terminal font size, in px (spec §8).
pub const FONT_SIZE_MIN: u16 = 10;
pub const FONT_SIZE_MAX: u16 = 24;
pub const FONT_SIZE_DEFAULT: u16 = 13;
/// Terminal scrollback, in lines (spec §8).
pub const SCROLLBACK_MIN: usize = 1_000;
pub const SCROLLBACK_MAX: usize = 100_000;
pub const SCROLLBACK_DEFAULT: usize = 10_000;
/// Keepalive of SSH and FTP sessions, in seconds (spec §4.3).
pub const KEEPALIVE_MIN: u64 = 10;
pub const KEEPALIVE_MAX: u64 = 300;
pub const KEEPALIVE_DEFAULT: u64 = 30;
/// The editor remote edits open files with: `system` is the OS's default editor; any other
/// value is a command, the path added as its last argument.
pub const EDITOR_DEFAULT: &str = SYSTEM_EDITOR;
/// The editor setting that means the OS's default editor ([`is_system_editor`]).
pub const SYSTEM_EDITOR: &str = "system";
/// Largest `settings.toml` read; a real one is under 200 bytes.
const MAX_BYTES: u64 = 64 * 1024;

/// The terminal's colours. Only one for now: the setting exists so that adding themes
/// later does not change the file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TerminalTheme {
    #[default]
    HoprsDark,
}

impl TerminalTheme {
    pub const ALL: [TerminalTheme; 1] = [TerminalTheme::HoprsDark];

    /// The name in `settings.toml`.
    fn key(self) -> &'static str {
        match self {
            Self::HoprsDark => "hoprs-dark",
        }
    }

    /// The name the settings page shows.
    pub fn label(self) -> &'static str {
        match self {
            Self::HoprsDark => crate::ui::strings::THEME_HOPRS_DARK,
        }
    }

    /// The colours.
    pub fn colors(self) -> TermTheme {
        match self {
            Self::HoprsDark => TermTheme::hoprs_dark(),
        }
    }

    /// The theme named `key`; an unknown name (a newer hoprs) is the default.
    fn from_key(key: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|theme| theme.key() == key)
            .unwrap_or_default()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    /// Transfer workers per site (`1..=16`), for the connections made from now on.
    pub workers_per_site: usize,
    /// What a transfer does when its destination exists.
    pub default_policy: ConflictPolicy,
    /// The terminal's font size in px (`10..=24`).
    pub terminal_font_size: u16,
    /// Lines of history each terminal keeps (`1 000..=100 000`), for the terminals opened
    /// from now on.
    pub terminal_scrollback: usize,
    pub terminal_theme: TerminalTheme,
    /// Seconds between keepalives on an idle SSH or FTP connection (`10..=300`), for the
    /// connections made from now on.
    pub keepalive_secs: u64,
    /// The editor of remote edits: `system` (the OS's default editor), or a command split
    /// with shell quoting rules (Windows command-line rules on Windows; no shell runs), the
    /// file's path added last. Never empty ([`valid_editor`]).
    pub editor_command: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            workers_per_site: WORKERS_DEFAULT,
            default_policy: ConflictPolicy::Ask,
            terminal_font_size: FONT_SIZE_DEFAULT,
            terminal_scrollback: SCROLLBACK_DEFAULT,
            terminal_theme: TerminalTheme::HoprsDark,
            keepalive_secs: KEEPALIVE_DEFAULT,
            editor_command: EDITOR_DEFAULT.to_owned(),
        }
    }
}

/// Whether the editor setting `command` means the OS's default editor: `system`, trimmed,
/// in any case.
pub fn is_system_editor(command: &str) -> bool {
    command.trim().eq_ignore_ascii_case(SYSTEM_EDITOR)
}

/// Whether `command` can be the editor command: something besides blanks.
pub fn valid_editor(command: &str) -> bool {
    !command.trim().is_empty()
}

/// `size` within `10..=24`.
pub fn clamp_font_size(size: u16) -> u16 {
    size.clamp(FONT_SIZE_MIN, FONT_SIZE_MAX)
}

/// `lines` within `1 000..=100 000`.
pub fn clamp_scrollback(lines: usize) -> usize {
    lines.clamp(SCROLLBACK_MIN, SCROLLBACK_MAX)
}

/// `secs` within `10..=300`.
pub fn clamp_keepalive(secs: u64) -> u64 {
    secs.clamp(KEEPALIVE_MIN, KEEPALIVE_MAX)
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

/// The file's layout. Unknown keys are ignored here (a newer hoprs may add some) and
/// kept by [`save`].
#[derive(Debug, Default, Deserialize, Serialize)]
struct File {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    workers_per_site: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    default_policy: Option<Policy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    terminal_font_size: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    terminal_scrollback: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    terminal_theme: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    keepalive_secs: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    editor_command: Option<String>,
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
        terminal_font_size: file
            .terminal_font_size
            .map_or(defaults.terminal_font_size, |n| {
                clamp_font_size(u16::try_from(n.max(0)).unwrap_or(FONT_SIZE_MAX))
            }),
        terminal_scrollback: file
            .terminal_scrollback
            .map_or(defaults.terminal_scrollback, |n| {
                clamp_scrollback(usize::try_from(n.max(0)).unwrap_or(SCROLLBACK_MAX))
            }),
        terminal_theme: file
            .terminal_theme
            .as_deref()
            .map_or(defaults.terminal_theme, TerminalTheme::from_key),
        keepalive_secs: file.keepalive_secs.map_or(defaults.keepalive_secs, |n| {
            clamp_keepalive(u64::try_from(n.max(0)).unwrap_or(KEEPALIVE_MAX))
        }),
        // An empty command is not a command: the default.
        editor_command: file
            .editor_command
            .filter(|command| valid_editor(command))
            .unwrap_or(defaults.editor_command),
    })
}

/// The file's keys for `settings` (clamped).
fn file_of(settings: &Settings) -> File {
    File {
        workers_per_site: i64::try_from(clamp_workers(settings.workers_per_site)).ok(),
        default_policy: Some(settings.default_policy.into()),
        terminal_font_size: Some(clamp_font_size(settings.terminal_font_size).into()),
        terminal_scrollback: i64::try_from(clamp_scrollback(settings.terminal_scrollback)).ok(),
        terminal_theme: Some(settings.terminal_theme.key().to_owned()),
        keepalive_secs: i64::try_from(clamp_keepalive(settings.keepalive_secs)).ok(),
        editor_command: Some(if valid_editor(&settings.editor_command) {
            settings.editor_command.clone()
        } else {
            EDITOR_DEFAULT.to_owned()
        }),
    }
}

/// The file content for `settings`.
pub fn render(settings: &Settings) -> String {
    toml::to_string(&file_of(settings)).expect("plain keys always serialize")
}

/// `existing` (a file's text) with the keys of `settings` set: every other key, table
/// and value stays as it was. Fails if `existing` is not TOML.
fn render_over(existing: &str, settings: &Settings) -> Result<String> {
    let mut table: toml::Table = toml::from_str(existing).map_err(|err| {
        Error::new(ErrorKind::Protocol, "Can't read settings.toml").with_detail(err.message())
    })?;
    let mut ours = toml::Table::try_from(file_of(settings)).expect("plain keys always serialize");
    // A theme this version does not know reads as the default: unless the user picked
    // another one, the name stays as written (a newer hoprs may know it).
    if let Some(existing) = table.get("terminal_theme").and_then(toml::Value::as_str)
        && TerminalTheme::from_key(existing) == settings.terminal_theme
    {
        ours.remove("terminal_theme");
    }
    table.extend(ours);
    Ok(toml::to_string(&table).expect("a parsed table always serializes"))
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

/// Writes `settings` to `path` atomically (the vault's crash-safe write), keeping the
/// keys of the file there this version does not know. A file that can't be read or
/// parsed is left alone (an error). Blocking.
pub fn save(path: &Path, settings: &Settings) -> Result<()> {
    let content = match read(path)? {
        Some(existing) => render_over(&existing, settings)?,
        None => render(settings),
    };
    hoprs_vault::write_atomic(path, content.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh install opens remote edits with the OS's default editor.
    #[test]
    fn editor_default_is_system() {
        assert_eq!(Settings::default().editor_command, "system");
    }

    #[test]
    fn system_editor_is_recognised() {
        assert!(is_system_editor(" System "));
        assert!(is_system_editor("system"));
        assert!(!is_system_editor("xdg-open"));
        assert!(!is_system_editor("system-editor"));
    }

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
                ..Settings::default()
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
                ..Settings::default()
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
                ..Settings::default()
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
    fn settings_terminal_fields_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        let defaults = Settings::default();
        assert_eq!(defaults.terminal_font_size, 13);
        assert_eq!(defaults.terminal_scrollback, 10_000);
        assert_eq!(defaults.terminal_theme, TerminalTheme::HoprsDark);

        // A file from before the terminal: the terminal defaults.
        std::fs::write(&path, "workers_per_site = 2\n").unwrap();
        let loaded = load(&path).settings;
        assert_eq!(loaded.workers_per_site, 2);
        assert_eq!(loaded.terminal_font_size, 13);
        assert_eq!(loaded.terminal_scrollback, 10_000);

        // Saved and read back, next to keys this version does not know (kept as they were).
        std::fs::write(
            &path,
            "future_key = 1\nworkers_per_site = 2\n\n[future_table]\nx = \"y\"\n",
        )
        .unwrap();
        let settings = Settings {
            terminal_font_size: 18,
            terminal_scrollback: 50_000,
            ..loaded
        };
        save(&path, &settings).unwrap();
        assert_eq!(
            load(&path),
            SettingsLoad {
                settings,
                error: None
            }
        );
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("terminal_font_size = 18"), "{text}");
        assert!(text.contains("terminal_scrollback = 50000"), "{text}");
        assert!(text.contains("terminal_theme = \"hoprs-dark\""), "{text}");
        assert!(
            text.contains("future_key = 1"),
            "unknown keys survive: {text}"
        );
        assert!(text.contains("[future_table]"), "{text}");

        // Out of range is clamped, on load and on save: 10..=24 and 1 000..=100 000.
        for (file, font, lines) in [
            ("terminal_font_size = 2\nterminal_scrollback = 5", 10, 1_000),
            (
                "terminal_font_size = 99\nterminal_scrollback = 900000000",
                24,
                100_000,
            ),
            (
                "terminal_font_size = -4\nterminal_scrollback = -1",
                10,
                1_000,
            ),
        ] {
            let parsed = parse(file).unwrap();
            assert_eq!(
                (parsed.terminal_font_size, parsed.terminal_scrollback),
                (font, lines),
                "{file}"
            );
        }
        let rendered = render(&Settings {
            terminal_font_size: 40,
            terminal_scrollback: 1,
            ..Settings::default()
        });
        let parsed = parse(&rendered).unwrap();
        assert_eq!(
            (parsed.terminal_font_size, parsed.terminal_scrollback),
            (24, 1_000)
        );
        assert_eq!(clamp_font_size(9), 10);
        assert_eq!(clamp_scrollback(250_000), 100_000);

        // A theme this version does not know falls back to "hoprs dark", without an error,
        // and saving keeps its name.
        let parsed = parse("terminal_theme = \"solarized\"").unwrap();
        assert_eq!(parsed.terminal_theme, TerminalTheme::HoprsDark);
        std::fs::write(&path, "terminal_theme = \"solarized\"\n").unwrap();
        let loaded = load(&path).settings;
        save(
            &path,
            &Settings {
                terminal_font_size: 15,
                ..loaded
            },
        )
        .unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("terminal_theme = \"solarized\""), "{text}");
        assert!(text.contains("terminal_font_size = 15"), "{text}");
        assert_eq!(TerminalTheme::HoprsDark.label(), "hoprs dark");

        // An unreadable file is still never written over: save only merges into a file
        // it can parse, else it would lose the user's keys.
        std::fs::write(&path, "this is not toml").unwrap();
        assert!(save(&path, &Settings::default()).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "this is not toml");
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
            ..Settings::default()
        });
        assert_eq!(parse(&rendered).unwrap().workers_per_site, 16);
    }

    /// The keepalive of SSH and FTP sessions: `10..=300` s, 30 by default, clamped on load
    /// and on save.
    #[test]
    fn keepalive_clamped() {
        assert_eq!(Settings::default().keepalive_secs, 30);
        assert_eq!(clamp_keepalive(0), 10);
        assert_eq!(clamp_keepalive(45), 45);
        assert_eq!(clamp_keepalive(301), 300);
        for (file, expected) in [
            ("", 30),
            ("keepalive_secs = 5", 10),
            ("keepalive_secs = -1", 10),
            ("keepalive_secs = 60", 60),
            ("keepalive_secs = 99999", 300),
        ] {
            assert_eq!(parse(file).unwrap().keepalive_secs, expected, "{file}");
        }
        let rendered = render(&Settings {
            keepalive_secs: 1,
            ..Settings::default()
        });
        assert!(rendered.contains("keepalive_secs = 10"), "{rendered}");
    }
}
