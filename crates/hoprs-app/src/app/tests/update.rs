//! Plan 15 Task 3: the update check's timing, the card's state, the Settings rows and the
//! palette entry. No I/O: the check is an effect, its result a message.

use hoprs_core::ErrorKind;
use semver::Version;

use super::*;
use crate::app::palette::PaletteItem;
use crate::app::settings::SettingsMsg;
use crate::app::update::{CardCheck, ManualCheck, UpdateEnv, UpdateMsg, Updater};
use crate::update::package::{Os, PackageKind};
use crate::update::{Asset, CHECK_EVERY, Release, START_DELAY};

fn v(s: &str) -> Version {
    Version::parse(s).unwrap()
}

/// A release with the three installers and `SHA256SUMS`.
fn release(version: &str) -> Release {
    let base = format!("https://github.com/engviniciusbs/hoprs/releases/download/v{version}");
    let asset = |name: String| Asset {
        url: format!("{base}/{name}"),
        name,
        size: 1000,
    };
    Release {
        version: v(version),
        page: format!("https://github.com/engviniciusbs/hoprs/releases/tag/v{version}"),
        assets: vec![
            asset(format!("hoprs_{version}-1_amd64.deb")),
            asset(format!("hoprs_{version}_windows_x64.msi")),
            asset(format!("hoprs_{version}_macos_universal.dmg")),
            asset("SHA256SUMS".to_owned()),
        ],
    }
}

/// A loaded app running 1.3.1 from a `.deb`, with automatic checks possible.
async fn fresh() -> App {
    app_running("1.3.1").await
}

async fn app_running(running: &str) -> App {
    let mut app = loaded(&FakeBackend::new()).await;
    app.updater = Updater::new(UpdateEnv::test(running, Os::Linux, Some(PackageKind::Deb)));
    app
}

fn upd(msg: UpdateMsg) -> Message {
    Message::Update(msg)
}

fn checked(manual: bool, result: Result<Release, Error>) -> Message {
    upd(UpdateMsg::Checked { manual, result })
}

fn failure() -> Error {
    Error::coded(
        ErrorKind::Network,
        "update-check-failed",
        "Could not check for updates",
    )
}

fn settings_loaded(settings: Settings) -> Message {
    Message::SettingsLoaded(SettingsLoad {
        settings,
        error: None,
    })
}

fn seen(version: &str) -> Settings {
    Settings {
        update_seen: Some(v(version)),
        ..Settings::default()
    }
}

/// The `CheckDue` messages of `Later` effects, with their delay.
fn check_dues(effects: &[Effect]) -> Vec<(std::time::Duration, Message)> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::Later { after, message } => match &**message {
                Message::Update(UpdateMsg::CheckDue(g)) => {
                    Some((*after, upd(UpdateMsg::CheckDue(*g))))
                }
                _ => None,
            },
            _ => None,
        })
        .collect()
}

fn check_requests(effects: &[Effect]) -> Vec<bool> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::CheckUpdate { manual } => Some(*manual),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn settings_loaded_schedules_one_check_after_start_delay() {
    let mut app = fresh().await;
    let effects = app.update(settings_loaded(Settings::default()));
    let dues = check_dues(&effects);
    assert_eq!(dues.len(), 1, "{effects:?}");
    assert_eq!(effects.len(), 1, "{effects:?}");
    let (after, due) = dues.into_iter().next().unwrap();
    assert_eq!(after, START_DELAY);
    let effects = app.update(due);
    assert!(
        matches!(effects[..], [Effect::CheckUpdate { manual: false }]),
        "{effects:?}"
    );
}

#[tokio::test]
async fn setting_off_means_no_automatic_check() {
    let mut app = fresh().await;
    let effects = app.update(settings_loaded(Settings {
        update_check: false,
        ..Settings::default()
    }));
    assert!(check_dues(&effects).is_empty(), "{effects:?}");
    for g in 0..5 {
        assert!(app.update(upd(UpdateMsg::CheckDue(g))).is_empty());
    }

    // A build that does not check by itself (a debug build without HOPRS_UPDATE_URL).
    let mut app = fresh().await;
    app.updater.env.auto = false;
    let effects = app.update(settings_loaded(Settings::default()));
    assert!(check_dues(&effects).is_empty(), "{effects:?}");
    for g in 0..5 {
        assert!(app.update(upd(UpdateMsg::CheckDue(g))).is_empty());
    }
    // Turning the setting off and on schedules nothing either.
    app.update(Message::Settings(SettingsMsg::UpdateCheck(false)));
    let effects = app.update(Message::Settings(SettingsMsg::UpdateCheck(true)));
    assert!(check_dues(&effects).is_empty(), "{effects:?}");
}

#[tokio::test]
async fn check_due_rearms_after_24h() {
    for result in [Ok(release("1.3.1")), Err(failure())] {
        let mut app = fresh().await;
        let effects = app.update(settings_loaded(Settings::default()));
        let (_, due) = check_dues(&effects).pop().unwrap();
        assert_eq!(check_requests(&app.update(due)), [false]);
        let effects = app.update(checked(false, result));
        let dues = check_dues(&effects);
        assert_eq!(dues.len(), 1, "{effects:?}");
        assert_eq!(dues[0].0, CHECK_EVERY);
        // The new timer checks again.
        let (_, due) = dues.into_iter().next().unwrap();
        assert_eq!(check_requests(&app.update(due)), [false]);
    }
}

#[tokio::test]
async fn toggling_setting_keeps_one_timer() {
    let mut app = fresh().await;
    let mut dues = check_dues(&app.update(settings_loaded(Settings::default())));
    for on in [false, true, false, true] {
        let effects = app.update(Message::Settings(SettingsMsg::UpdateCheck(on)));
        assert_eq!(app.settings.update_check, on);
        dues.extend(check_dues(&effects));
    }
    // The start-up timer, then the two "on" ones: only the newest acts. The stale ones
    // fire first, with nothing in flight, so only the generation stops them.
    assert_eq!(dues.len(), 3, "{dues:?}");
    let (_, newest) = dues.pop().unwrap();
    for (_, stale) in dues.clone() {
        assert!(app.update(stale).is_empty());
    }
    assert_eq!(check_requests(&app.update(newest)), [false]);
    // Its result arms the next timer; the old ones still do nothing.
    let next = check_dues(&app.update(checked(false, Ok(release("1.3.1")))));
    for (_, stale) in dues {
        assert!(app.update(stale).is_empty());
    }
    let (_, next) = next.into_iter().next().unwrap();
    assert_eq!(check_requests(&app.update(next)), [false]);
}

#[test]
fn unit_tests_never_check_by_themselves() {
    // Whatever HOPRS_UPDATE_URL says: a test opts in with its own `UpdateEnv`.
    assert!(!App::new().updater.env.auto);
}

#[tokio::test]
async fn check_now_while_checking_sends_no_second_request() {
    let mut app = fresh().await;
    let (_, due) = check_dues(&app.update(settings_loaded(Settings::default())))
        .pop()
        .unwrap();
    assert_eq!(check_requests(&app.update(due)), [false]);
    let effects = app.update(Message::Settings(SettingsMsg::CheckNow));
    assert!(check_requests(&effects).is_empty(), "{effects:?}");
    assert_eq!(app.updater.manual, ManualCheck::Checking);
    // Another click while it runs: still one request.
    assert!(check_requests(&app.update(upd(UpdateMsg::CheckNow))).is_empty());
    // The automatic request's result answers Check now.
    app.update(checked(false, Ok(release("1.3.1"))));
    assert_eq!(app.updater.manual, ManualCheck::UpToDate);
}

#[tokio::test]
async fn newer_release_shows_card_once_and_saves_seen() {
    let mut app = fresh().await;
    let effects = app.update(checked(false, Ok(release("1.4.0"))));
    assert!(app.updater.card);
    assert_eq!(app.settings.update_seen, Some(v("1.4.0")));
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::SaveSettings(s) if s.update_seen == Some(v("1.4.0")))),
        "{effects:?}"
    );
    assert_eq!(app.update_offer(), Some(&v("1.4.0")));
    // Learning it again (the next 24 h check) does not show the card again.
    app.update(Message::SettingsSaved(Ok(())));
    app.update(upd(UpdateMsg::Dismiss));
    let effects = app.update(checked(false, Ok(release("1.4.0"))));
    assert!(!app.updater.card);
    assert!(
        !effects.iter().any(|e| matches!(e, Effect::SaveSettings(_))),
        "{effects:?}"
    );
}

#[tokio::test]
async fn card_takes_no_focus() {
    let mut app = fresh().await;
    let palette = app.palette.clone();
    let menu = app.menu.clone();
    let dialogs = format!("{:?}", app.dialogs);
    let effects = app.update(checked(false, Ok(release("1.4.0"))));
    assert!(app.updater.card);
    for effect in &effects {
        assert!(
            !matches!(
                effect,
                Effect::Unfocus
                    | Effect::FocusEdit { .. }
                    | Effect::FocusPalette
                    | Effect::FocusTerminalSearch(_)
                    | Effect::FocusDialogField(_)
            ),
            "{effect:?}"
        );
    }
    assert_eq!(app.palette, palette);
    assert_eq!(app.menu, menu);
    assert_eq!(format!("{:?}", app.dialogs), dialogs);
}

#[tokio::test]
async fn restart_shows_only_the_footer_line() {
    let mut app = fresh().await;
    app.update(settings_loaded(seen("1.4.0")));
    let effects = app.update(checked(false, Ok(release("1.4.0"))));
    assert!(!app.updater.card);
    assert_eq!(app.update_offer(), Some(&v("1.4.0")));
    assert!(
        !effects.iter().any(|e| matches!(e, Effect::SaveSettings(_))),
        "{effects:?}"
    );
}

#[tokio::test]
async fn footer_line_from_seen_before_any_check() {
    for (running, offer) in [
        ("1.3.1", Some(v("1.4.0"))),
        ("1.4.0", None),
        ("1.5.0", None),
    ] {
        let mut app = app_running(running).await;
        app.update(settings_loaded(seen("1.4.0")));
        assert_eq!(app.update_offer(), offer.as_ref(), "{running}");
        assert!(!app.updater.card);
    }
}

#[tokio::test]
async fn newer_version_shows_card_again() {
    let mut app = fresh().await;
    app.update(settings_loaded(seen("1.4.0")));
    app.update(checked(false, Ok(release("1.5.0"))));
    assert!(app.updater.card);
    assert_eq!(app.settings.update_seen, Some(v("1.5.0")));
    assert_eq!(app.update_offer(), Some(&v("1.5.0")));
}

#[tokio::test]
async fn same_or_older_release_shows_nothing() {
    for version in ["1.3.1", "1.2.0"] {
        let mut app = fresh().await;
        let effects = app.update(checked(false, Ok(release(version))));
        assert!(!app.updater.card, "{version}");
        assert_eq!(app.update_offer(), None, "{version}");
        assert_eq!(app.settings.update_seen, None);
        assert!(
            !effects.iter().any(|e| matches!(e, Effect::SaveSettings(_))),
            "{effects:?}"
        );
    }
}

#[tokio::test]
async fn automatic_check_failure_shows_nothing() {
    let mut app = fresh().await;
    let effects = app.update(checked(false, Err(failure())));
    assert!(!app.updater.card);
    assert_eq!(app.updater.manual, ManualCheck::Idle);
    assert_eq!(app.update_offer(), None);
    assert!(
        effects.iter().all(|e| matches!(e, Effect::Later { .. })),
        "{effects:?}"
    );
}

#[tokio::test]
async fn check_now_results() {
    let mut app = fresh().await;
    let effects = app.update(upd(UpdateMsg::CheckNow));
    assert_eq!(check_requests(&effects), [true]);
    assert_eq!(app.updater.manual, ManualCheck::Checking);
    app.update(checked(true, Ok(release("1.3.1"))));
    assert_eq!(app.updater.manual, ManualCheck::UpToDate);
    assert!(!app.updater.card);

    assert_eq!(
        check_requests(&app.update(upd(UpdateMsg::CheckNow))),
        [true]
    );
    app.update(checked(true, Ok(release("1.4.0"))));
    assert_eq!(app.updater.manual, ManualCheck::Available(v("1.4.0")));
    assert!(app.updater.card);

    // Check now opens the card even for a version already seen.
    app.update(Message::SettingsSaved(Ok(())));
    app.update(upd(UpdateMsg::Dismiss));
    app.update(Message::Settings(SettingsMsg::CheckNow));
    app.update(checked(true, Ok(release("1.4.0"))));
    assert!(app.updater.card);

    assert_eq!(
        check_requests(&app.update(Message::Settings(SettingsMsg::CheckNow))),
        [true]
    );
    app.update(checked(true, Err(failure())));
    assert_eq!(app.updater.manual, ManualCheck::Failed(failure()));
}

#[tokio::test]
async fn dismiss_then_show_card() {
    let mut app = fresh().await;
    app.update(checked(false, Ok(release("1.4.0"))));
    assert!(app.updater.card);
    assert!(app.update(upd(UpdateMsg::Dismiss)).is_empty());
    assert!(!app.updater.card);
    assert_eq!(app.update_offer(), Some(&v("1.4.0")));
    let effects = app.update(upd(UpdateMsg::ShowCard));
    assert!(app.updater.card);
    assert!(effects.is_empty(), "{effects:?}");
}

#[tokio::test]
async fn show_card_without_release_checks_first() {
    let mut app = fresh().await;
    app.update(settings_loaded(seen("1.4.0")));
    assert!(app.updater.known.is_none());
    let effects = app.update(upd(UpdateMsg::ShowCard));
    assert!(app.updater.card);
    assert_eq!(check_requests(&effects), [true]);
    assert_eq!(effects.len(), 1, "{effects:?}");
    // The card shows this check; Settings' Check now row is not touched.
    assert_eq!(app.updater.card_check, CardCheck::Checking);
    assert_eq!(app.updater.manual, ManualCheck::Idle);
    // Clicked again while it checks: no second request.
    assert!(app.update(upd(UpdateMsg::ShowCard)).is_empty());
    app.update(checked(true, Err(failure())));
    assert_eq!(app.updater.card_check, CardCheck::Failed(failure()));
    assert_eq!(app.updater.manual, ManualCheck::Idle);
    assert!(app.updater.card);
    // A retry, and its success.
    assert_eq!(
        check_requests(&app.update(upd(UpdateMsg::ShowCard))),
        [true]
    );
    assert_eq!(app.updater.card_check, CardCheck::Checking);
    app.update(checked(true, Ok(release("1.4.0"))));
    assert_eq!(app.updater.card_check, CardCheck::Idle);
    assert_eq!(app.updater.manual, ManualCheck::Idle);
    assert!(app.updater.card);
    assert_eq!(app.updater.known, Some(release("1.4.0")));
}

#[tokio::test]
async fn dismiss_during_footer_check_wins() {
    let mut app = fresh().await;
    app.update(settings_loaded(seen("1.4.0")));
    assert_eq!(
        check_requests(&app.update(upd(UpdateMsg::ShowCard))),
        [true]
    );
    app.update(upd(UpdateMsg::Dismiss));
    app.update(checked(true, Ok(release("1.4.0"))));
    assert!(!app.updater.card);
    assert_eq!(app.updater.card_check, CardCheck::Idle);
    assert_eq!(app.update_offer(), Some(&v("1.4.0")));
    // Its failure, after a ✕, shows nowhere either.
    let mut app = fresh().await;
    app.update(settings_loaded(seen("1.4.0")));
    app.update(upd(UpdateMsg::ShowCard));
    app.update(upd(UpdateMsg::Dismiss));
    app.update(checked(true, Err(failure())));
    assert!(!app.updater.card);
    assert_eq!(app.updater.card_check, CardCheck::Idle);
    assert_eq!(app.updater.manual, ManualCheck::Idle);
}

#[tokio::test]
async fn whats_new_opens_the_release_page() {
    let mut app = fresh().await;
    // The URL is built from the version, whatever page the server named.
    let mut odd = release("1.4.0");
    odd.page = "https://github.com/someone/else?x=1".to_owned();
    app.update(checked(false, Ok(odd)));
    let effects = app.update(upd(UpdateMsg::WhatsNew));
    match &effects[..] {
        [Effect::OpenPage(url)] => assert_eq!(
            url,
            "https://github.com/engviniciusbs/hoprs/releases/tag/v1.4.0"
        ),
        other => panic!("{other:?}"),
    }
    let error = Error::new(ErrorKind::Io, "xdg-open failed");
    app.update(upd(UpdateMsg::PageOpened(Err(error.clone()))));
    assert_eq!(app.updater.page_error, Some(error));
    // Opened the next time: the error goes.
    app.update(upd(UpdateMsg::WhatsNew));
    app.update(upd(UpdateMsg::PageOpened(Ok(()))));
    assert_eq!(app.updater.page_error, None);
}

#[tokio::test]
async fn palette_lists_update_while_newer_known() {
    let mut app = fresh().await;
    let update_rows = |app: &App| -> Vec<_> {
        app.palette_entries()
            .into_iter()
            .filter(|e| e.item == PaletteItem::Update)
            .collect()
    };
    assert!(update_rows(&app).is_empty());
    app.update(checked(false, Ok(release("1.3.1"))));
    assert!(update_rows(&app).is_empty());

    app.update(checked(false, Ok(release("1.4.0"))));
    let rows = update_rows(&app);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].label, "Update hoprs to 1.4.0");
    assert!(matches!(
        app.item_message(&PaletteItem::Update),
        Some(Message::Update(UpdateMsg::Start))
    ));
    // Reachable without typing.
    app.update(Message::Palette(PaletteMsg::Open));
    assert!(
        app.palette_results()
            .iter()
            .any(|e| e.item == PaletteItem::Update)
    );
}

#[tokio::test]
async fn update_input_ignored_while_closing_or_behind_dialog() {
    let mut app = fresh().await;
    app.update(checked(false, Ok(release("1.4.0"))));
    app.update(upd(UpdateMsg::Dismiss));

    // Behind the palette: the card's buttons take no input.
    app.update(Message::Palette(PaletteMsg::Open));
    assert!(app.update(upd(UpdateMsg::ShowCard)).is_empty());
    assert!(!app.updater.card);
    app.update(Message::Palette(PaletteMsg::Close));

    // Results still land behind it.
    app.update(Message::Palette(PaletteMsg::Open));
    app.update(upd(UpdateMsg::PageOpened(Err(failure()))));
    assert_eq!(app.updater.page_error, Some(failure()));
    app.update(Message::Palette(PaletteMsg::Close));

    // A timer firing while the window closes starts no request.
    let mut closing = fresh().await;
    let (_, due) = check_dues(&closing.update(settings_loaded(Settings::default())))
        .pop()
        .unwrap();
    closing.closing = true;
    assert!(closing.update(due).is_empty());

    app.closing = true;
    for msg in [
        UpdateMsg::Start,
        UpdateMsg::ShowCard,
        UpdateMsg::CheckNow,
        UpdateMsg::WhatsNew,
        UpdateMsg::Install,
    ] {
        assert!(msg.is_user(), "{msg:?}");
        assert!(app.update(upd(msg)).is_empty());
    }
    assert!(!app.updater.card);
    assert_eq!(app.updater.manual, ManualCheck::Idle);
    assert!(!UpdateMsg::CheckDue(0).is_user());
    assert!(!UpdateMsg::PageOpened(Ok(())).is_user());
}

// ---- Plan 15 Task 4: Update downloads, verifies and hands off. ----

use std::path::{Path, PathBuf};

use crate::app::update::{Phase, install_command};
use crate::update::download::{DownloadEvent, DownloadJob};

/// An app running 1.3.1 on `os` from `package` that has learned of 1.4.0 (card open).
async fn knowing(os: Os, package: Option<PackageKind>) -> App {
    let mut app = loaded(&FakeBackend::new()).await;
    app.updater = Updater::new(UpdateEnv::test("1.3.1", os, package));
    app.update(checked(false, Ok(release("1.4.0"))));
    assert!(app.updater.card);
    app
}

fn download(event: DownloadEvent) -> Message {
    upd(UpdateMsg::Download(event))
}

fn jobs(effects: &[Effect]) -> Vec<DownloadJob> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::DownloadUpdate(job) => Some(job.clone()),
            _ => None,
        })
        .collect()
}

fn no_close(effects: &[Effect]) {
    assert!(
        !effects.iter().any(|e| matches!(e, Effect::RequestClose)),
        "{effects:?}"
    );
}

fn page_of(effects: &[Effect]) -> Option<&str> {
    match effects {
        [Effect::OpenPage(url)] => Some(url),
        _ => None,
    }
}

const PAGE_140: &str = "https://github.com/engviniciusbs/hoprs/releases/tag/v1.4.0";

#[tokio::test]
async fn update_without_package_opens_release_page() {
    let mut app = knowing(Os::Linux, None).await;
    let effects = app.update(upd(UpdateMsg::Start));
    assert_eq!(page_of(&effects), Some(PAGE_140), "{effects:?}");
    assert!(jobs(&effects).is_empty());
    assert_eq!(app.updater.phase, Phase::Idle);
}

#[tokio::test]
async fn missing_asset_opens_release_page() {
    for missing in ["hoprs_1.4.0-1_amd64.deb", "SHA256SUMS"] {
        let mut app = knowing(Os::Linux, Some(PackageKind::Deb)).await;
        let mut r = release("1.4.0");
        r.assets.retain(|a| a.name != missing);
        app.updater.known = Some(r);
        let effects = app.update(upd(UpdateMsg::Start));
        assert_eq!(page_of(&effects), Some(PAGE_140), "{missing}: {effects:?}");
        assert_eq!(app.updater.phase, Phase::Idle);
    }
}

#[tokio::test]
async fn linux_downloads_verifies_and_hands_off() {
    let mut app = knowing(Os::Linux, Some(PackageKind::Deb)).await;
    app.update(upd(UpdateMsg::Dismiss));
    let mut all = Vec::new();
    let effects = app.update(upd(UpdateMsg::Start));
    let job = match &jobs(&effects)[..] {
        [job] => job.clone(),
        other => panic!("{other:?} / {effects:?}"),
    };
    assert_eq!(effects.len(), 1, "{effects:?}");
    assert_eq!(job.asset.name, "hoprs_1.4.0-1_amd64.deb");
    assert_eq!(
        job.sums_url,
        "https://github.com/engviniciusbs/hoprs/releases/download/v1.4.0/SHA256SUMS"
    );
    assert!(app.updater.card, "Update shows the card");
    assert_eq!(
        app.updater.phase,
        Phase::Downloading {
            done: 0,
            total: Some(1000)
        }
    );
    all.extend(effects);

    all.extend(app.update(download(DownloadEvent::Progress {
        done: 10,
        total: Some(100),
    })));
    assert_eq!(
        app.updater.phase,
        Phase::Downloading {
            done: 10,
            total: Some(100)
        }
    );
    all.extend(app.update(download(DownloadEvent::Verifying)));
    assert_eq!(app.updater.phase, Phase::Verifying);

    let file = PathBuf::from("Downloads").join("hoprs_1.4.0-1_amd64.deb");
    let effects = app.update(download(DownloadEvent::Done(Ok(file.clone()))));
    assert!(
        matches!(&effects[..], [Effect::OpenInstaller(f)] if f == &file),
        "{effects:?}"
    );
    assert_eq!(
        app.updater.phase,
        Phase::HandedOff {
            file: file.clone(),
            open_error: None
        }
    );
    all.extend(effects);
    all.extend(app.update(upd(UpdateMsg::InstallerOpened(Ok(())))));
    no_close(&all);
    assert!(matches!(app.updater.phase, Phase::HandedOff { .. }));
}

#[test]
fn install_command_quotes_the_full_path() {
    assert_eq!(
        install_command(Path::new("/home/u/My Downloads/hoprs_1.4.0-1_amd64.deb")),
        "sudo apt install '/home/u/My Downloads/hoprs_1.4.0-1_amd64.deb'"
    );
    assert_eq!(
        install_command(Path::new("/home/u/Downloads/hoprs_1.4.0-1_amd64.deb")),
        "sudo apt install /home/u/Downloads/hoprs_1.4.0-1_amd64.deb"
    );
}

#[tokio::test]
async fn linux_open_failure_keeps_command() {
    let mut app = knowing(Os::Linux, Some(PackageKind::Deb)).await;
    app.update(upd(UpdateMsg::Start));
    let file = PathBuf::from("/home/u/Downloads/hoprs_1.4.0-1_amd64.deb");
    app.update(download(DownloadEvent::Done(Ok(file.clone()))));
    let error = Error::new(ErrorKind::Io, "xdg-open failed");
    let effects = app.update(upd(UpdateMsg::InstallerOpened(Err(error.clone()))));
    no_close(&effects);
    assert_eq!(
        app.updater.phase,
        Phase::HandedOff {
            file: file.clone(),
            open_error: Some(error)
        }
    );
    let effects = app.update(upd(UpdateMsg::CopyCommand));
    match &effects[..] {
        [Effect::WriteClipboard(text)] => assert_eq!(text.0, install_command(&file)),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn macos_hands_off_without_closing() {
    let mut app = knowing(Os::MacOs, Some(PackageKind::Dmg)).await;
    let mut all = app.update(upd(UpdateMsg::Start));
    let job = jobs(&all).pop().expect("a download");
    assert_eq!(job.asset.name, "hoprs_1.4.0_macos_universal.dmg");
    let file = PathBuf::from("hoprs_1.4.0_macos_universal.dmg");
    let effects = app.update(download(DownloadEvent::Done(Ok(file.clone()))));
    assert!(
        matches!(&effects[..], [Effect::OpenInstaller(f)] if f == &file),
        "{effects:?}"
    );
    all.extend(effects);
    all.extend(app.update(upd(UpdateMsg::InstallerOpened(Ok(())))));
    all.extend(app.update(upd(UpdateMsg::InstallerOpened(Err(failure())))));
    no_close(&all);
    assert!(matches!(
        app.updater.phase,
        Phase::HandedOff {
            open_error: Some(_),
            ..
        }
    ));
}

#[tokio::test]
async fn windows_waits_for_second_click() {
    use crate::app::tests::transfers::{item, synced};
    use hoprs_core::SiteId;
    use hoprs_transfer::TransferState;

    let mut app = knowing(Os::Windows, Some(PackageKind::Msi)).await;
    let s = SiteId::new();
    synced(
        &mut app,
        3,
        vec![
            item(1, &s, TransferState::Active),
            item(2, &s, TransferState::Active),
            item(3, &s, TransferState::Queued),
            item(4, &s, TransferState::Done),
        ],
    );
    assert_eq!(app.transfers_to_stop(), 3);

    let effects = app.update(upd(UpdateMsg::Start));
    assert_eq!(
        jobs(&effects).pop().expect("a download").asset.name,
        "hoprs_1.4.0_windows_x64.msi"
    );
    let file = PathBuf::from("hoprs_1.4.0_windows_x64.msi");
    let effects = app.update(download(DownloadEvent::Done(Ok(file.clone()))));
    assert!(effects.is_empty(), "{effects:?}");
    assert_eq!(app.updater.phase, Phase::Ready(file.clone()));

    let effects = app.update(upd(UpdateMsg::Install));
    assert!(
        matches!(&effects[..], [Effect::OpenInstaller(f)] if f == &file),
        "{effects:?}"
    );
    // A second click while the installer opens does not open it twice.
    assert!(app.update(upd(UpdateMsg::Install)).is_empty());
    let effects = app.update(upd(UpdateMsg::InstallerOpened(Ok(()))));
    assert!(matches!(effects[..], [Effect::RequestClose]), "{effects:?}");

    // The opener failing: the error, and hoprs stays.
    let mut app = knowing(Os::Windows, Some(PackageKind::Msi)).await;
    app.update(upd(UpdateMsg::Start));
    app.update(download(DownloadEvent::Done(Ok(file.clone()))));
    app.update(upd(UpdateMsg::Install));
    let effects = app.update(upd(UpdateMsg::InstallerOpened(Err(failure()))));
    no_close(&effects);
    assert_eq!(app.updater.phase, Phase::Failed(failure()));
}

#[tokio::test]
async fn windows_later_keeps_the_file() {
    let mut app = knowing(Os::Windows, Some(PackageKind::Msi)).await;
    app.update(upd(UpdateMsg::Start));
    let file = PathBuf::from("hoprs_1.4.0_windows_x64.msi");
    app.update(download(DownloadEvent::Done(Ok(file.clone()))));
    assert!(app.update(upd(UpdateMsg::Later)).is_empty());
    assert!(!app.updater.card);
    assert_eq!(app.updater.phase, Phase::Ready(file.clone()));

    app.update(upd(UpdateMsg::ShowCard));
    assert!(app.updater.card);
    let effects = app.update(upd(UpdateMsg::Start));
    assert!(
        matches!(&effects[..], [Effect::OpenInstaller(f)] if f == &file),
        "{effects:?}"
    );
}

#[tokio::test]
async fn download_error_offers_try_again() {
    let mut app = knowing(Os::Linux, Some(PackageKind::Deb)).await;
    app.update(upd(UpdateMsg::Start));
    let error = Error::coded(ErrorKind::Protocol, "update-checksum-mismatch", "bad");
    let effects = app.update(download(DownloadEvent::Done(Err(error.clone()))));
    assert!(effects.is_empty(), "{effects:?}");
    assert_eq!(app.updater.phase, Phase::Failed(error));
    let effects = app.update(upd(UpdateMsg::Start));
    assert_eq!(jobs(&effects).len(), 1, "{effects:?}");
    assert!(matches!(app.updater.phase, Phase::Downloading { .. }));
}

#[tokio::test]
async fn second_update_click_is_ignored_while_downloading() {
    let mut app = knowing(Os::Linux, Some(PackageKind::Deb)).await;
    let mut all = app.update(upd(UpdateMsg::Start));
    all.extend(app.update(upd(UpdateMsg::Start)));
    app.update(download(DownloadEvent::Verifying));
    all.extend(app.update(upd(UpdateMsg::Start)));
    assert_eq!(jobs(&all).len(), 1, "{all:?}");
    assert_eq!(all.len(), 1, "{all:?}");
}

#[tokio::test]
async fn newer_release_during_download_waits() {
    let mut app = knowing(Os::Linux, Some(PackageKind::Deb)).await;
    app.update(upd(UpdateMsg::Start));
    let effects = app.update(checked(false, Ok(release("1.5.0"))));
    assert!(
        effects.iter().all(|e| matches!(e, Effect::Later { .. })),
        "only the next timer: {effects:?}"
    );
    assert_eq!(app.updater.known.as_ref().unwrap().version, v("1.4.0"));
    assert!(matches!(app.updater.phase, Phase::Downloading { .. }));
}

#[tokio::test]
async fn start_before_any_check_checks_then_continues() {
    // Only `update_seen` (the palette before any check this run).
    let mut app = fresh().await;
    app.update(settings_loaded(seen("1.4.0")));
    assert!(app.updater.known.is_none());
    let effects = app.update(upd(UpdateMsg::Start));
    assert_eq!(check_requests(&effects), [true], "{effects:?}");
    assert!(jobs(&effects).is_empty());
    let effects = app.update(checked(true, Ok(release("1.4.0"))));
    assert_eq!(jobs(&effects).len(), 1, "{effects:?}");
    assert!(matches!(app.updater.phase, Phase::Downloading { .. }));

    // The check failing opens the release page of the version seen.
    let mut app = fresh().await;
    app.update(settings_loaded(seen("1.4.0")));
    app.update(upd(UpdateMsg::Start));
    let effects = app.update(checked(true, Err(failure())));
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::OpenPage(url) if url == PAGE_140)),
        "{effects:?}"
    );
    assert!(jobs(&effects).is_empty());

    // ✕ before the result: nothing starts.
    let mut app = fresh().await;
    app.update(settings_loaded(seen("1.4.0")));
    app.update(upd(UpdateMsg::Start));
    app.update(upd(UpdateMsg::Dismiss));
    let effects = app.update(checked(true, Ok(release("1.4.0"))));
    assert!(jobs(&effects).is_empty(), "{effects:?}");
    assert_eq!(app.updater.phase, Phase::Idle);
}

#[tokio::test]
async fn stale_download_events_are_ignored() {
    let mut app = knowing(Os::Linux, Some(PackageKind::Deb)).await;
    // No download running: a late event changes nothing.
    let effects = app.update(download(DownloadEvent::Done(Ok(PathBuf::from("x.deb")))));
    assert!(effects.is_empty());
    assert_eq!(app.updater.phase, Phase::Idle);
    app.update(download(DownloadEvent::Progress {
        done: 1,
        total: None,
    }));
    assert_eq!(app.updater.phase, Phase::Idle);
}

#[tokio::test]
async fn installer_opened_while_closing_does_not_close_again() {
    let mut app = knowing(Os::Windows, Some(PackageKind::Msi)).await;
    app.update(upd(UpdateMsg::Start));
    let file = PathBuf::from("hoprs_1.4.0_windows_x64.msi");
    app.update(download(DownloadEvent::Done(Ok(file))));
    app.update(upd(UpdateMsg::Install));
    app.closing = true;
    let effects = app.update(upd(UpdateMsg::InstallerOpened(Ok(()))));
    no_close(&effects);
}
