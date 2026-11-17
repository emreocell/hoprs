//! The update notice's state (Plan 15): when the app asks for the latest release, what
//! it learned, and the card in the bottom left. No I/O here: the check is
//! [`Effect::CheckUpdate`], the release page [`Effect::OpenPage`].
//!
//! **Checking.** [`START_DELAY`] after the settings load, then every [`CHECK_EVERY`] while
//! the app runs, only when the build checks by itself ([`UpdateEnv::auto`]) and
//! "Check for updates automatically" is on. Every schedule carries a generation and only
//! the latest one acts, so toggling the setting never leaves two timers; at most one
//! request is in flight, and Check now while one runs waits for its result. A finished
//! check arms the next one from now (an older timer then does nothing).
//!
//! **The card** shows once per version: the first time a newer release than the running
//! one is learned (newer than `update_seen`), which is then saved. ✕ collapses it to the
//! footer line, which stays while a newer version is known ([`App::update_offer`]). Check
//! now finding a newer release opens it too. It never takes the keyboard focus. A failed
//! automatic check only logs; Check now shows its result in Settings.
//!
//! **Updating** ([`Phase`]): Update downloads this install's installer and verifies it
//! against the release's `SHA256SUMS` ([`Effect::DownloadUpdate`]); one download at a
//! time, and a newer release learned meanwhile waits. No package for this install, or
//! the asset or `SHA256SUMS` missing, opens the release page instead. The verified file
//! is opened with the system handler ([`Effect::OpenInstaller`]) at once on macOS and
//! Linux (hoprs stays; Linux also shows [`install_command`] with Copy); Windows waits for
//! "Close and install", then closes hoprs the normal way ([`Effect::RequestClose`]); a
//! close the user cancels leaves [`Phase::InstallerOpen`] with "Close hoprs".

use std::path::{Path, PathBuf};
use std::time::Duration;

use hoprs_core::Error;
use semver::Version;

use super::{App, Effect, Message};
use crate::terminal::Redacted;
use crate::update::download::{DownloadEvent, DownloadJob};
use crate::update::package::{Os, PackageKind, asset_name, current_package};
use crate::update::{CHECK_EVERY, Release, SUMS_NAME, http, is_newer, parse_version};

/// What this process is and may do.
#[derive(Debug, Clone)]
pub struct UpdateEnv {
    /// The version running (in a debug build, `HOPRS_UPDATE_AS_VERSION` when set).
    pub running: Version,
    /// Whether the app checks by itself: always in a release build, in a debug build only
    /// with `HOPRS_UPDATE_URL`.
    pub auto: bool,
    /// Chooses the hand-off: Windows waits for "Close and install", then closes hoprs.
    pub os: Os,
    /// The installer that updates this install; `None` opens the release page instead
    /// (in a debug build, `HOPRS_UPDATE_AS_PACKAGE` when set).
    pub package: Option<PackageKind>,
}

impl UpdateEnv {
    /// What `App::new` uses: the process, except in unit tests, where nothing checks by
    /// itself unless a test sets its own environment (an exported `HOPRS_UPDATE_URL`
    /// changes no unrelated test).
    pub fn for_app() -> Self {
        #[cfg(not(test))]
        return Self::from_process();
        #[cfg(test)]
        return Self {
            auto: false,
            ..Self::test(env!("CARGO_PKG_VERSION"), Os::current(), None)
        };
    }

    #[cfg_attr(test, allow(dead_code))]
    pub fn from_process() -> Self {
        let built = Version::parse(env!("CARGO_PKG_VERSION")).unwrap_or(Version::new(0, 0, 0));
        let running = if cfg!(debug_assertions) {
            std::env::var("HOPRS_UPDATE_AS_VERSION")
                .ok()
                .and_then(|v| parse_version(&v))
                .unwrap_or(built)
        } else {
            built
        };
        Self {
            running,
            auto: !cfg!(debug_assertions) || http::api_url().is_some(),
            os: Os::current(),
            package: package_override().or_else(current_package),
        }
    }

    #[cfg(test)]
    pub fn test(running: &str, os: Os, package: Option<PackageKind>) -> Self {
        Self {
            running: Version::parse(running).expect("a version"),
            auto: true,
            os,
            package,
        }
    }
}

/// The package a debug build pretends to be installed from: `HOPRS_UPDATE_AS_PACKAGE`
/// (`deb`, `msi` or `dmg`), so the real download and hand-off can be tried from
/// `cargo run`. Release builds never read it: `cfg!` makes this a property of the build,
/// as for `HOPRS_UPDATE_URL` and `HOPRS_UPDATE_AS_VERSION`.
fn package_override() -> Option<PackageKind> {
    if !cfg!(debug_assertions) {
        return None;
    }
    let value = std::env::var("HOPRS_UPDATE_AS_PACKAGE").ok()?;
    let kind = parse_package(&value);
    if kind.is_none() {
        tracing::warn!(%value, "HOPRS_UPDATE_AS_PACKAGE ignored: not deb, msi or dmg");
    }
    kind
}

/// `deb`, `msi` or `dmg`, exactly; anything else is no package.
fn parse_package(value: &str) -> Option<PackageKind> {
    match value {
        "deb" => Some(PackageKind::Deb),
        "msi" => Some(PackageKind::Msi),
        "dmg" => Some(PackageKind::Dmg),
        _ => None,
    }
}

/// Settings → General's Check now, and its inline result.
#[derive(Debug, Clone, PartialEq)]
pub enum ManualCheck {
    Idle,
    Checking,
    UpToDate,
    Available(Version),
    Failed(Error),
}

/// The check the footer line starts (its click before any check this run returned the
/// release), shown in the card; Settings' Check now has [`ManualCheck`].
#[derive(Debug, Clone, PartialEq)]
pub enum CardCheck {
    Idle,
    Checking,
    Failed(Error),
}

/// Where Update is (`Idle` until Update is clicked).
#[derive(Debug, Clone, PartialEq)]
pub enum Phase {
    Idle,
    Downloading {
        done: u64,
        total: Option<u64>,
    },
    Verifying,
    /// Verified, waiting for "Close and install" (Windows only).
    Ready(PathBuf),
    /// The installer was opened (or is being opened; `open_error` if that failed on
    /// macOS or Linux, where the card keeps the file and, on Linux, the command).
    HandedOff {
        file: PathBuf,
        open_error: Option<Error>,
    },
    /// Windows: the installer opened and hoprs was asked to close. A close the user
    /// cancelled (the unsent-edits question) leaves it here: "Close hoprs" asks again.
    InstallerOpen(PathBuf),
    Failed(Error),
}

/// The update notice's state.
#[derive(Debug)]
pub struct Updater {
    pub env: UpdateEnv,
    /// The newest release a check returned that is newer than the running version.
    pub known: Option<Release>,
    /// The card is shown (not collapsed to the footer line).
    pub card: bool,
    pub phase: Phase,
    /// Settings' Check now and its result.
    pub manual: ManualCheck,
    /// The footer line's check, shown in the card.
    pub card_check: CardCheck,
    /// Opening the release page failed.
    pub page_error: Option<Error>,
    /// A check runs.
    in_flight: bool,
    /// The latest schedule's generation: only its `CheckDue` acts.
    generation: u64,
    /// Update was asked for before any check returned the release: the check's result
    /// continues it.
    start_pending: bool,
}

impl Updater {
    pub fn new(env: UpdateEnv) -> Self {
        Self {
            env,
            known: None,
            card: false,
            phase: Phase::Idle,
            manual: ManualCheck::Idle,
            card_check: CardCheck::Idle,
            page_error: None,
            in_flight: false,
            generation: 0,
            start_pending: false,
        }
    }
}

#[derive(Debug, Clone)]
pub enum UpdateMsg {
    /// The automatic check of schedule `generation` is due.
    CheckDue(u64),
    /// Settings → General's Check now.
    CheckNow,
    /// A check finished; `manual` if Check now (or the footer line) asked for it.
    Checked {
        manual: bool,
        result: Result<Release, Error>,
    },
    /// The footer line: shows the card again.
    ShowCard,
    /// The card's ✕: collapses it to the footer line.
    Dismiss,
    /// "What's new": the release page.
    WhatsNew,
    PageOpened(Result<(), Error>),
    /// The card's Update (or the palette's entry).
    Start,
    Download(DownloadEvent),
    /// "Close and install" (Windows).
    Install,
    /// "Later": keeps the verified file.
    Later,
    /// Copies the Linux install command.
    CopyCommand,
    /// "Close hoprs" (Windows, the installer open): the normal close request again.
    CloseForInstall,
    InstallerOpened(Result<(), Error>),
}

impl UpdateMsg {
    /// Sent by the user's click or key (dropped behind a dialog or the palette, and
    /// while the window closes), not by work finishing.
    pub fn is_user(&self) -> bool {
        matches!(
            self,
            UpdateMsg::CheckNow
                | UpdateMsg::ShowCard
                | UpdateMsg::Dismiss
                | UpdateMsg::WhatsNew
                | UpdateMsg::Start
                | UpdateMsg::Install
                | UpdateMsg::Later
                | UpdateMsg::CopyCommand
                | UpdateMsg::CloseForInstall
        )
    }
}

/// The Linux install command for the verified `.deb`: the absolute path, quoted for the
/// shell (it always holds a `/`, so apt reads it as a file, not a package name).
pub fn install_command(file: &Path) -> String {
    let path = file.to_string_lossy();
    // `try_quote` fails only on a NUL byte, which no path holds.
    let quoted = shlex::try_quote(&path).unwrap_or_default();
    format!("sudo apt install {quoted}")
}

/// The release page of `version`, built from the parsed version (never a URL the server
/// sent).
fn release_page(version: &Version) -> String {
    format!("https://github.com/engviniciusbs/hoprs/releases/tag/v{version}")
}

impl App {
    /// The newer version the app knows of: the release a check returned, else the one a
    /// card was shown for (`update_seen`, which survives restarts), if newer than the
    /// running version.
    pub fn update_offer(&self) -> Option<&Version> {
        let running = &self.updater.env.running;
        self.updater
            .known
            .as_ref()
            .map(|release| &release.version)
            .filter(|version| is_newer(version, running))
            .or_else(|| {
                self.settings
                    .update_seen
                    .as_ref()
                    .filter(|version| is_newer(version, running))
            })
    }

    /// Arms the automatic check `after` from now. A new generation always: an older
    /// timer does nothing when it fires. No timer unless the build checks by itself and
    /// the setting is on.
    pub(super) fn schedule_update_check(&mut self, after: Duration) -> Vec<Effect> {
        self.updater.generation += 1;
        if !(self.updater.env.auto && self.settings.update_check) {
            return Vec::new();
        }
        vec![Effect::Later {
            after,
            message: Box::new(Message::Update(UpdateMsg::CheckDue(
                self.updater.generation,
            ))),
        }]
    }

    /// A check, unless one runs: its result answers both.
    fn request_check(&mut self, manual: bool) -> Vec<Effect> {
        if self.updater.in_flight {
            return Vec::new();
        }
        self.updater.in_flight = true;
        vec![Effect::CheckUpdate { manual }]
    }

    pub(super) fn update_msg(&mut self, msg: UpdateMsg) -> Vec<Effect> {
        match msg {
            UpdateMsg::CheckDue(generation) => {
                let updater = &self.updater;
                if generation != updater.generation
                    || !updater.env.auto
                    || !self.settings.update_check
                {
                    return Vec::new();
                }
                // One in flight: its result arms the next timer.
                self.request_check(false)
            }
            UpdateMsg::CheckNow => {
                self.updater.manual = ManualCheck::Checking;
                self.request_check(true)
            }
            UpdateMsg::Checked { manual, result } => self.checked(manual, result),
            UpdateMsg::ShowCard => {
                self.updater.card = true;
                if self.updater.known.is_some() {
                    return Vec::new();
                }
                // Only `update_seen` so far (no check yet this run): ask now. The card
                // shows this check's state, not Settings' Check now.
                self.updater.card_check = CardCheck::Checking;
                self.request_check(true)
            }
            UpdateMsg::Dismiss => {
                self.updater.card = false;
                self.updater.start_pending = false;
                // Nobody waits for the footer's check any more: its result leaves the
                // card closed.
                self.updater.card_check = CardCheck::Idle;
                Vec::new()
            }
            UpdateMsg::WhatsNew => match self.update_offer() {
                // Built from the parsed version: no server-supplied path reaches the
                // opener.
                Some(version) => vec![Effect::OpenPage(release_page(version))],
                None => Vec::new(),
            },
            UpdateMsg::PageOpened(result) => {
                self.updater.page_error = result.err();
                Vec::new()
            }
            UpdateMsg::Start => self.start_update(),
            UpdateMsg::Download(event) => self.download_event(event),
            UpdateMsg::Install => self.install(),
            UpdateMsg::Later => {
                // The verified file stays: Update (or the palette) opens it.
                self.updater.card = false;
                Vec::new()
            }
            // The command is apt's: only for the `.deb`.
            UpdateMsg::CopyCommand => match &self.updater.phase {
                Phase::HandedOff { file, .. }
                    if self.updater.env.package == Some(PackageKind::Deb) =>
                {
                    vec![Effect::WriteClipboard(Redacted(install_command(file)))]
                }
                _ => Vec::new(),
            },
            UpdateMsg::InstallerOpened(result) => self.installer_opened(result),
            UpdateMsg::CloseForInstall => self.close_for_install(),
        }
    }

    /// How many transfers stop when hoprs closes for the Windows installer (running,
    /// paused or waiting, and queued: they continue on the next start).
    pub fn transfers_to_stop(&self) -> usize {
        let [active, queued, ..] = self.queue.counts();
        active + queued
    }

    /// Update (the card's button, Try again, the palette).
    fn start_update(&mut self) -> Vec<Effect> {
        match &self.updater.phase {
            // One download at a time; the release it fetches stays.
            Phase::Downloading { .. } | Phase::Verifying => return Vec::new(),
            Phase::Ready(_) => {
                self.updater.card = true;
                return self.install();
            }
            // The installer is open: closing hoprs is what is left (never a second one).
            Phase::InstallerOpen(_) => {
                self.updater.card = true;
                return self.close_for_install();
            }
            // Idle, Failed (Try again), HandedOff (again: the download step finds the
            // file there, verifies and reuses it).
            _ => {}
        }
        let Some(release) = self.updater.known.clone() else {
            // Only `update_seen` so far: check first, the result continues.
            self.updater.start_pending = true;
            return self.update_msg(UpdateMsg::ShowCard);
        };
        let page = || vec![Effect::OpenPage(release_page(&release.version))];
        let Some(kind) = self.updater.env.package else {
            return page();
        };
        let (Some(asset), Some(sums)) = (
            release.asset(&asset_name(kind, &release.version)),
            release.asset(SUMS_NAME),
        ) else {
            return page();
        };
        self.updater.card = true;
        self.updater.phase = Phase::Downloading {
            done: 0,
            total: Some(asset.size),
        };
        vec![Effect::DownloadUpdate(DownloadJob {
            asset: asset.clone(),
            sums_url: sums.url.clone(),
        })]
    }

    /// A download's event; one that comes while no download runs is stale.
    fn download_event(&mut self, event: DownloadEvent) -> Vec<Effect> {
        let phase = &mut self.updater.phase;
        if !matches!(phase, Phase::Downloading { .. } | Phase::Verifying) {
            return Vec::new();
        }
        match event {
            DownloadEvent::Progress { done, total } => {
                if matches!(phase, Phase::Downloading { .. }) {
                    *phase = Phase::Downloading { done, total };
                }
                Vec::new()
            }
            DownloadEvent::Verifying => {
                *phase = Phase::Verifying;
                Vec::new()
            }
            // A second request for the download under way: that one goes on.
            DownloadEvent::Done(Err(error)) if error.code == Some("update-download-busy") => {
                Vec::new()
            }
            DownloadEvent::Done(Err(error)) => {
                *phase = Phase::Failed(error);
                Vec::new()
            }
            // Windows: hoprs must close for the installer, so it waits for a second
            // click ("Close and install").
            DownloadEvent::Done(Ok(file)) if self.updater.env.os == Os::Windows => {
                *phase = Phase::Ready(file);
                Vec::new()
            }
            // macOS and Linux: the card opens (also after a ✕ during the download), so the
            // steps or the command are seen.
            DownloadEvent::Done(Ok(file)) => {
                self.updater.card = true;
                self.updater.phase = Phase::HandedOff {
                    file: file.clone(),
                    open_error: None,
                };
                vec![Effect::OpenInstaller(file)]
            }
        }
    }

    /// "Close and install": opens the verified installer (hoprs closes once it opened).
    fn install(&mut self) -> Vec<Effect> {
        let Phase::Ready(file) = &self.updater.phase else {
            return Vec::new();
        };
        let file = file.clone();
        // Opening: a second click opens nothing more.
        self.updater.phase = Phase::HandedOff {
            file: file.clone(),
            open_error: None,
        };
        vec![Effect::OpenInstaller(file)]
    }

    fn installer_opened(&mut self, result: Result<(), Error>) -> Vec<Effect> {
        let Phase::HandedOff { file, open_error } = &mut self.updater.phase else {
            return Vec::new();
        };
        let windows = self.updater.env.os == Os::Windows;
        match result {
            Ok(()) if windows => {
                self.updater.phase = Phase::InstallerOpen(file.clone());
                self.close_for_install()
            }
            Ok(()) => {
                *open_error = None;
                Vec::new()
            }
            // Windows: hoprs stays open, Try again downloads (reuses) and asks again.
            Err(error) if windows => {
                self.updater.phase = Phase::Failed(error);
                Vec::new()
            }
            Err(error) => {
                *open_error = Some(error);
                Vec::new()
            }
        }
    }

    /// Windows, the installer open: the window's normal close request (the unsent-edits
    /// question included). Already closing: the close in progress is enough (a second
    /// request would force the exit).
    fn close_for_install(&mut self) -> Vec<Effect> {
        if !matches!(self.updater.phase, Phase::InstallerOpen(_)) || self.closing {
            return Vec::new();
        }
        vec![Effect::RequestClose]
    }

    /// A check finished. Settings' Check now (`manual == Checking`, also when it was
    /// clicked while an automatic check ran) gets its result in [`ManualCheck`]; the
    /// footer's check (`card_check == Checking`) in [`CardCheck`]. A failure neither
    /// waits for is only logged. (The message's `manual` flag says who asked; the state
    /// says who still waits, which is what counts after a ✕ or a merged Check now.)
    fn checked(&mut self, _manual: bool, result: Result<Release, Error>) -> Vec<Effect> {
        self.updater.in_flight = false;
        let settings = self.updater.manual == ManualCheck::Checking;
        let card = self.updater.card_check == CardCheck::Checking;
        if card {
            self.updater.card_check = CardCheck::Idle;
        }
        let mut effects = Vec::new();
        // Update clicked before the release was known continues now (or, if the check
        // failed, opens the release page of the version offered).
        // Nothing new starts once the window closes.
        let start = std::mem::take(&mut self.updater.start_pending) && !self.closing;
        let failed = result.is_err();
        match result {
            Ok(release) if is_newer(&release.version, &self.updater.env.running) => {
                let version = release.version.clone();
                if settings {
                    self.updater.manual = ManualCheck::Available(version.clone());
                }
                // A download under way keeps the release it fetches (Task 4).
                if matches!(self.updater.phase, Phase::Idle | Phase::Failed(_)) {
                    // A failure was about the release known then: a newer one starts over
                    // (the card never shows the new version above the old error).
                    let replaced = self
                        .updater
                        .known
                        .as_ref()
                        .is_none_or(|known| known.version < version);
                    if replaced && matches!(self.updater.phase, Phase::Failed(_)) {
                        self.updater.phase = Phase::Idle;
                    }
                    let unseen = self
                        .settings
                        .update_seen
                        .as_ref()
                        .is_none_or(|seen| seen < &version);
                    self.updater.known = Some(release);
                    // Check now opens the card; the footer's check found it open (a ✕
                    // since then wins); a version never shown opens it once.
                    if unseen || settings {
                        self.updater.card = true;
                    }
                    if unseen {
                        self.settings.update_seen = Some(version);
                        effects.extend(self.save_settings_effect());
                    }
                }
            }
            Ok(release) => {
                tracing::debug!(latest = %release.version, "hoprs is up to date");
                if settings {
                    self.updater.manual = ManualCheck::UpToDate;
                }
            }
            Err(error) => {
                if !(settings || card) {
                    tracing::info!(%error, "update check failed");
                }
                if card {
                    self.updater.card_check = CardCheck::Failed(error.clone());
                }
                if settings {
                    self.updater.manual = ManualCheck::Failed(error);
                }
            }
        }
        if start {
            if failed {
                effects.extend(
                    self.update_offer()
                        .map(|version| Effect::OpenPage(release_page(version))),
                );
            } else if self.updater.known.is_some() {
                effects.extend(self.start_update());
            }
        }
        effects.extend(self.schedule_update_check(CHECK_EVERY));
        effects
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_override_parses_strictly() {
        assert_eq!(parse_package("deb"), Some(PackageKind::Deb));
        assert_eq!(parse_package("msi"), Some(PackageKind::Msi));
        assert_eq!(parse_package("dmg"), Some(PackageKind::Dmg));
        for bad in [
            "", "DEB", " deb", "deb ", ".deb", "zip", "appimage", "dmg\n",
        ] {
            assert_eq!(parse_package(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn package_override_follows_the_build() {
        if cfg!(debug_assertions) {
            // Debug: only what HOPRS_UPDATE_AS_PACKAGE says (the test runner does not set it).
            let set = std::env::var("HOPRS_UPDATE_AS_PACKAGE").ok();
            assert_eq!(package_override(), set.as_deref().and_then(parse_package));
        } else {
            // Release: never, whatever the environment says.
            assert_eq!(package_override(), None);
        }
    }
}
