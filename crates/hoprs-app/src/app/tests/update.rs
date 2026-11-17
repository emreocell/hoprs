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
