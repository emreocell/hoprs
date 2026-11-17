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

use std::path::PathBuf;
use std::time::Duration;

use hoprs_core::Error;
use semver::Version;

use super::{App, Effect, Message};
use crate::update::download::DownloadEvent;
use crate::update::package::{Os, PackageKind, current_package};
use crate::update::{CHECK_EVERY, Release, http, is_newer, parse_version};

/// What this process is and may do.
#[derive(Debug, Clone)]
pub struct UpdateEnv {
    /// The version running (in a debug build, `HOPRS_UPDATE_AS_VERSION` when set).
    pub running: Version,
    /// Whether the app checks by itself: always in a release build, in a debug build only
    /// with `HOPRS_UPDATE_URL`.
    pub auto: bool,
    // `os` and `package` choose the installer and its hand-off (Task 4).
    #[allow(dead_code)]
    pub os: Os,
    /// The installer that updates this install; `None` opens the release page instead.
    #[allow(dead_code)]
    pub package: Option<PackageKind>,
}

impl UpdateEnv {
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
            package: current_package(),
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

/// Settings → General's Check now, and its inline result.
#[derive(Debug, Clone, PartialEq)]
pub enum ManualCheck {
    Idle,
    Checking,
    UpToDate,
    Available(Version),
    Failed(Error),
}

/// Where Update is (driven by Task 4; `Idle` until Update is clicked).
#[derive(Debug, Clone, PartialEq)]
// Constructed by the download and hand-off (Task 4), read by the card (Task 5).
#[allow(dead_code)]
pub enum Phase {
    Idle,
    Downloading {
        done: u64,
        total: Option<u64>,
    },
    Verifying,
    /// Verified, waiting for "Close and install" (Windows) or being handed off.
    Ready(PathBuf),
    HandedOff {
        file: PathBuf,
        open_error: Option<Error>,
    },
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
    pub manual: ManualCheck,
    /// Opening the release page failed.
    pub page_error: Option<Error>,
    /// A check runs.
    in_flight: bool,
    /// The latest schedule's generation: only its `CheckDue` acts.
    generation: u64,
}

impl Updater {
    pub fn new(env: UpdateEnv) -> Self {
        Self {
            env,
            known: None,
            card: false,
            phase: Phase::Idle,
            manual: ManualCheck::Idle,
            page_error: None,
            in_flight: false,
            generation: 0,
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
        )
    }
}

/// The release page of `version`, when no check has returned it yet.
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
                // Only `update_seen` so far (no check yet this run): ask now.
                self.updater.manual = ManualCheck::Checking;
                self.request_check(true)
            }
            UpdateMsg::Dismiss => {
                self.updater.card = false;
                Vec::new()
            }
            UpdateMsg::WhatsNew => {
                let page = match &self.updater.known {
                    Some(release) => release.page.clone(),
                    None => match self.update_offer() {
                        Some(version) => release_page(version),
                        None => return Vec::new(),
                    },
                };
                vec![Effect::OpenPage(page)]
            }
            UpdateMsg::PageOpened(result) => {
                self.updater.page_error = result.err();
                Vec::new()
            }
            // The download and the hand-off (Task 4).
            UpdateMsg::Start
            | UpdateMsg::Download(_)
            | UpdateMsg::Install
            | UpdateMsg::Later
            | UpdateMsg::CopyCommand
            | UpdateMsg::InstallerOpened(_) => Vec::new(),
        }
    }

    fn checked(&mut self, manual: bool, result: Result<Release, Error>) -> Vec<Effect> {
        self.updater.in_flight = false;
        // Check now clicked while an automatic check ran: this result answers it.
        let manual = manual || self.updater.manual == ManualCheck::Checking;
        let mut effects = Vec::new();
        match result {
            Ok(release) if is_newer(&release.version, &self.updater.env.running) => {
                let version = release.version.clone();
                if manual {
                    self.updater.manual = ManualCheck::Available(version.clone());
                }
                // A download under way keeps the release it fetches (Task 4).
                if matches!(self.updater.phase, Phase::Idle | Phase::Failed(_)) {
                    let unseen = self
                        .settings
                        .update_seen
                        .as_ref()
                        .is_none_or(|seen| seen < &version);
                    self.updater.known = Some(release);
                    if unseen || manual {
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
                if manual {
                    self.updater.manual = ManualCheck::UpToDate;
                }
            }
            Err(error) => {
                if manual {
                    self.updater.manual = ManualCheck::Failed(error);
                } else {
                    tracing::info!(%error, "update check failed");
                }
            }
        }
        effects.extend(self.schedule_update_check(CHECK_EVERY));
        effects
    }
}
