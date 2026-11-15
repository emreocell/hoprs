//! Folder compare: the per-tab state machine and its triggers (Plan 12, Task 5).

use std::time::SystemTime;

use hoprs_core::{ConnectionState, Error, ErrorKind};
use hoprs_remote::{Difference, Progress, Status, Summary};

use super::*;
use crate::app::compare::{CompareState, Group, Phase};
use crate::app::{CompareMsg, CompareRunId};

fn msg(m: CompareMsg) -> Message {
    Message::Compare(m)
}

/// Like `settle`, but keeps every effect the messages cause (listings included).
async fn settle_all(
    app: &mut App,
    fake: &Arc<FakeBackend>,
    mut pending: Vec<Message>,
) -> Vec<Effect> {
    let mut all = Vec::new();
    while !pending.is_empty() {
        let mut next = Vec::new();
        for message in pending {
            let effects = app.update(message);
            next.extend(run_listings(fake, &effects).await);
            all.extend(effects);
        }
        pending = next;
    }
    all
}

/// A connected FTP tab (remote `/srv`, local `/home/me`, both listed) and the effects the
/// opening caused.
async fn open(auto: bool) -> (App, Arc<FakeBackend>, Site, Vec<Effect>) {
    let mut a = site("a", "h", "u", Protocol::Ftp);
    a.remote_dir = Some("/srv".into());
    let fake = Arc::new(FakeBackend::new().with_site(a.clone()));
    fake.remote_fs(&a.id)
        .add_file("/srv/x", b"x", SystemTime::UNIX_EPOCH);
    fake.local_fs().add_dir("/home/me/sub");
    let mut app = loaded(&fake).await;
    app.settings.compare_auto = auto;
    app.home = Some(rpath("/home/me"));
    let effects = app.update(Message::ConnectSite(a.id.clone()));
    let mut pending = run_listings(&fake, &effects).await;
    let mut all = effects;
    let backend: Arc<dyn BackendApi> = fake.clone();
    pending.extend(
        effects::connect_sites(backend, vec![a.id.clone()])
            .collect::<Vec<_>>()
            .await,
    );
    all.extend(settle_all(&mut app, &fake, pending).await);
    (app, fake, a, all)
}

fn debounces(effects: &[Effect]) -> Vec<(TabId, u64, std::time::Duration)> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::Later { after, message } => match **message {
                Message::Compare(CompareMsg::Debounced { tab, generation }) => {
                    Some((tab, generation, *after))
                }
                _ => None,
            },
            _ => None,
        })
        .collect()
}

fn started(effects: &[Effect]) -> Vec<(CompareRunId, String, String)> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::Compare {
                run,
                local_root,
                remote_root,
                ..
            } => Some((*run, local_root.to_string(), remote_root.to_string())),
            _ => None,
        })
        .collect()
}

fn cancelled(effects: &[Effect]) -> Vec<CompareRunId> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::CompareCancel { run, .. } => Some(*run),
            _ => None,
        })
        .collect()
}

fn aborted(effects: &[Effect]) -> Vec<CompareRunId> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::CompareAbort { run, .. } => Some(*run),
            _ => None,
        })
        .collect()
}

fn state(app: &App) -> &CompareState {
    app.tabs[0].compare().expect("a files tab")
}

fn found(tab: TabId, run: CompareRunId, path: &str) -> Message {
    msg(CompareMsg::Progress {
        tab,
        run,
        progress: Progress::Found(Difference {
            path: path.to_owned(),
            kind: hoprs_core::EntryKind::File,
            status: Status::OnlyLocal,
            local: None,
            remote: None,
            entries_inside: None,
        }),
    })
}

/// Opens the tab and delivers the debounce: a run is in flight.
async fn running() -> (App, Arc<FakeBackend>, Site, TabId, CompareRunId) {
    let (mut app, fake, a, effects) = open(true).await;
    let tab = app.tabs[0].id;
    let (_, generation, _) = *debounces(&effects).last().expect("a debounce");
    let effects = app.update(msg(CompareMsg::Debounced { tab, generation }));
    let run = started(&effects)[0].0;
    (app, fake, a, tab, run)
}

#[tokio::test]
async fn auto_runs_when_both_panes_listed_after_the_debounce() {
    let (mut app, _fake, _a, effects) = open(true).await;
    let tab = app.tabs[0].id;
    let waits = debounces(&effects);
    assert_eq!(waits.len(), 1, "one debounce: {effects:?}");
    assert_eq!(waits[0].2, std::time::Duration::from_millis(500));
    assert!(started(&effects).is_empty(), "nothing starts before it");
    assert_eq!(state(&app).phase, Phase::Debouncing);

    let effects = app.update(msg(CompareMsg::Debounced {
        tab,
        generation: waits[0].1,
    }));
    let runs = started(&effects);
    assert_eq!(runs.len(), 1);
    assert_eq!(
        (runs[0].1.as_str(), runs[0].2.as_str()),
        ("/home/me", "/srv")
    );
    assert!(matches!(state(&app).phase, Phase::Running { .. }));
    assert_eq!(state(&app).run, Some(runs[0].0));
    let options = effects
        .iter()
        .find_map(|e| match e {
            Effect::Compare { options, .. } => Some(options),
            _ => None,
        })
        .unwrap();
    assert_eq!(options.ignore, app.settings.compare_ignore);
    assert_eq!(options.limit, Some(10_000));
    assert!(cancelled(&effects).is_empty(), "nothing to cancel yet");
}

#[tokio::test]
async fn no_auto_run_when_off_but_start_runs_anyway() {
    let (mut app, _fake, _a, effects) = open(false).await;
    let tab = app.tabs[0].id;
    assert!(debounces(&effects).is_empty() && started(&effects).is_empty());
    assert_eq!(state(&app).phase, Phase::Idle);

    let effects = app.update(msg(CompareMsg::Start { tab }));
    assert_eq!(started(&effects).len(), 1);
    assert!(cancelled(&effects).is_empty());
    assert!(matches!(state(&app).phase, Phase::Running { .. }));
}

#[tokio::test]
async fn folder_change_cancels_and_restarts_after_the_debounce() {
    let (mut app, fake, _a, tab, run1) = running().await;
    app.update(found(tab, run1, "kept"));
    assert_eq!(state(&app).rows.len(), 1);

    // The pane still lists while the run walks, and the run is cancelled cooperatively.
    let nav = app.update(pane_msg(
        tab,
        Side::Local,
        PaneMsg::Navigate(rpath("/home/me/sub")),
    ));
    assert_eq!(
        listings(&nav).len(),
        1,
        "the pane's List is emitted: {nav:?}"
    );
    assert_eq!(cancelled(&nav), [run1]);
    assert!(aborted(&nav).is_empty());

    // Late results of the cancelled run are dropped.
    assert!(app.update(found(tab, run1, "late")).is_empty());
    assert!(
        app.update(msg(CompareMsg::Progress {
            tab,
            run: run1,
            progress: Progress::Seen {
                local: 9,
                remote: 9
            }
        }))
        .is_empty()
    );
    assert!(
        state(&app).rows.iter().all(|r| r.diff.path != "late"),
        "{:?}",
        state(&app).rows
    );
    app.update(msg(CompareMsg::Finished {
        tab,
        run: run1,
        result: Ok(Summary {
            identical: 5,
            differences: 1,
            partial: true,
            stopped: true,
        }),
    }));
    assert_ne!(state(&app).identical, 5, "a stale Finished is dropped");
    assert!(!matches!(state(&app).phase, Phase::Done { .. }));

    // The listing settles, then the debounce starts the new run on the new folder.
    let effects = settle_all(&mut app, &fake, run_listings(&fake, &nav).await).await;
    let waits = debounces(&effects);
    assert_eq!(waits.len(), 1);
    let effects = app.update(msg(CompareMsg::Debounced {
        tab,
        generation: waits[0].1,
    }));
    let runs = started(&effects);
    assert_eq!(runs.len(), 1);
    assert_ne!(runs[0].0, run1);
    assert_eq!(runs[0].1, "/home/me/sub");
}

#[tokio::test]
async fn only_the_latest_debounce_starts_a_run() {
    let (mut app, fake, _a, effects) = open(true).await;
    let tab = app.tabs[0].id;
    let first = debounces(&effects)[0].1;
    // Navigating again before the first fired supersedes it.
    let nav = app.update(pane_msg(
        tab,
        Side::Local,
        PaneMsg::Navigate(rpath("/home/me/sub")),
    ));
    assert!(cancelled(&nav).is_empty(), "no run to cancel: {nav:?}");
    let effects = settle_all(&mut app, &fake, run_listings(&fake, &nav).await).await;
    let second = debounces(&effects)[0].1;
    assert_ne!(first, second);
    assert!(
        started(&app.update(msg(CompareMsg::Debounced {
            tab,
            generation: first
        })))
        .is_empty()
    );
    assert_eq!(
        started(&app.update(msg(CompareMsg::Debounced {
            tab,
            generation: second
        })))
        .len(),
        1
    );
}

#[tokio::test]
async fn refresh_reruns() {
    let (mut app, fake, _a, tab, run1) = running().await;
    let retry = app.update(pane_msg(tab, Side::Remote, PaneMsg::Retry));
    assert_eq!(cancelled(&retry), [run1]);
    let effects = settle_all(&mut app, &fake, run_listings(&fake, &retry).await).await;
    let waits = debounces(&effects);
    assert_eq!(waits.len(), 1);
    let effects = app.update(msg(CompareMsg::Debounced {
        tab,
        generation: waits[0].1,
    }));
    assert_eq!(started(&effects).len(), 1);
}

#[tokio::test]
async fn settings_changes_never_start_runs() {
    let (mut app, _fake, _a, _effects) = open(false).await;
    for m in [
        settings::SettingsMsg::CompareIgnore("a, b".into()),
        settings::SettingsMsg::CompareIgnore("a, b, c".into()),
        settings::SettingsMsg::CompareAuto(true),
    ] {
        let effects = app.update(Message::Settings(m));
        assert!(started(&effects).is_empty() && debounces(&effects).is_empty());
    }
    assert_eq!(app.settings.compare_ignore, ["a", "b", "c"]);
    // The next run uses the changed list.
    let tab = app.tabs[0].id;
    let effects = app.update(msg(CompareMsg::Start { tab }));
    let ignore = effects.iter().find_map(|e| match e {
        Effect::Compare { options, .. } => Some(options.ignore.clone()),
        _ => None,
    });
    assert_eq!(ignore.unwrap(), ["a", "b", "c"]);
}

#[tokio::test]
async fn toggle_auto_off_saves_cancels_and_keeps_rows() {
    let (mut app, _fake, _a, tab, run) = running().await;
    app.update(found(tab, run, "src/a"));
    let effects = app.update(msg(CompareMsg::ToggleAuto { tab, on: false }));
    assert!(!app.settings.compare_auto);
    assert!(effects.iter().any(|e| matches!(e, Effect::SaveSettings(_))));
    assert_eq!(cancelled(&effects), [run]);
    assert_eq!(state(&app).rows.len(), 1, "the current rows stay");
    assert!(!state(&app).hidden);
    assert_eq!(state(&app).run, None);
    assert!(app.update(found(tab, run, "late")).is_empty());

    // Turning it on starts a run.
    app.update(Message::SettingsSaved(Ok(())));
    let effects = app.update(msg(CompareMsg::ToggleAuto { tab, on: true }));
    assert!(app.settings.compare_auto);
    assert!(effects.iter().any(|e| matches!(e, Effect::SaveSettings(_))));
    assert_eq!(started(&effects).len(), 1);
}

#[tokio::test]
async fn toggle_auto_off_with_nothing_running_cancels_nothing() {
    let (mut app, _fake, _a, effects) = open(true).await;
    let tab = app.tabs[0].id;
    let generation = debounces(&effects)[0].1;
    let effects = app.update(msg(CompareMsg::ToggleAuto { tab, on: false }));
    assert!(cancelled(&effects).is_empty());
    // The pending debounce is dropped too.
    assert!(started(&app.update(msg(CompareMsg::Debounced { tab, generation }))).is_empty());
}

#[tokio::test]
async fn hide_hides_until_the_next_run_and_stops_it() {
    let (mut app, _fake, _a, tab, run) = running().await;
    let effects = app.update(msg(CompareMsg::Hide { tab }));
    assert!(state(&app).hidden);
    assert_eq!(cancelled(&effects), [run]);
    assert_eq!(state(&app).run, None);
    let effects = app.update(msg(CompareMsg::Start { tab }));
    assert!(!state(&app).hidden);
    assert_eq!(started(&effects).len(), 1);
}

#[tokio::test]
async fn stop_keeps_the_rows_found_and_cancels() {
    let (mut app, _fake, _a, tab, run) = running().await;
    app.update(found(tab, run, "a"));
    let effects = app.update(msg(CompareMsg::Stop { tab }));
    assert_eq!(cancelled(&effects), [run]);
    assert_eq!(state(&app).phase, Phase::Done { partial: true });
    assert_eq!(state(&app).rows.len(), 1);
}

#[tokio::test]
async fn progress_finishes_and_counts() {
    let (mut app, _fake, _a, tab, run) = running().await;
    app.update(msg(CompareMsg::Progress {
        tab,
        run,
        progress: Progress::Seen {
            local: 3,
            remote: 4,
        },
    }));
    assert_eq!(state(&app).phase, Phase::Running { seen: (3, 4) });
    app.update(found(tab, run, "a"));
    app.update(msg(CompareMsg::Finished {
        tab,
        run,
        result: Ok(Summary {
            identical: 7,
            differences: 1,
            partial: false,
            stopped: false,
        }),
    }));
    assert_eq!(state(&app).phase, Phase::Done { partial: false });
    assert_eq!(state(&app).identical, 7);
    assert_eq!(state(&app).run, None);
}

#[tokio::test]
async fn a_failed_run_stops_with_its_error() {
    let (mut app, _fake, _a, tab, run) = running().await;
    app.update(msg(CompareMsg::Finished {
        tab,
        run,
        result: Err(Error::new(ErrorKind::Permission, "nope")),
    }));
    assert!(matches!(&state(&app).phase, Phase::Stopped(e) if e.message == "nope"));
}

#[tokio::test]
async fn limit_pauses_and_continue_resumes() {
    let (mut app, _fake, _a, tab, run) = running().await;
    app.update(msg(CompareMsg::Progress {
        tab,
        run,
        progress: Progress::LimitReached {
            local: 10_000,
            remote: 5,
        },
    }));
    assert_eq!(state(&app).phase, Phase::Paused { seen: (10_000, 5) });
    let effects = app.update(msg(CompareMsg::Continue { tab }));
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::CompareResume { run: r, .. } if *r == run))
    );
    assert!(matches!(state(&app).phase, Phase::Running { .. }));
    // Continue when not paused does nothing.
    assert!(app.update(msg(CompareMsg::Continue { tab })).is_empty());
}

#[tokio::test]
async fn closing_the_tab_aborts_the_run() {
    let (mut app, _fake, _a, tab, run) = running().await;
    let effects = app.update(Message::CloseTab(tab));
    assert_eq!(aborted(&effects), [run]);
    assert!(cancelled(&effects).is_empty());
}

#[tokio::test]
async fn a_dropped_session_aborts_and_stops_with_the_error() {
    let (mut app, _fake, a, tab, run) = running().await;
    let TabState::Connected(connected) = tab_state(&app, &a.id).clone() else {
        panic!("connected");
    };
    let lost = Error::new(ErrorKind::Network, "connection lost");
    let effects = app.update(Message::SessionState {
        session: connected.session,
        state: ConnectionState::Disconnected(lost.clone()),
        auth: None,
    });
    assert_eq!(aborted(&effects), [run]);
    assert_eq!(state(&app).phase, Phase::Stopped(lost));
    assert_eq!(state(&app).run, None);
    // A late result of the aborted run is dropped.
    assert!(app.update(found(tab, run, "late")).is_empty());
}

#[tokio::test]
async fn select_and_select_group_mark_rows() {
    let (mut app, _fake, _a, tab, run) = running().await;
    app.update(found(tab, run, "a"));
    app.update(found(tab, run, "b"));
    app.update(msg(CompareMsg::Select {
        tab,
        index: 1,
        on: true,
    }));
    assert_eq!(
        state(&app)
            .rows
            .iter()
            .map(|r| r.selected)
            .collect::<Vec<_>>(),
        [false, true]
    );
    app.update(msg(CompareMsg::SelectGroup {
        tab,
        group: Group::OnlyLocal,
        on: true,
    }));
    assert!(state(&app).rows.iter().all(|r| r.selected));
    app.update(msg(CompareMsg::SelectGroup {
        tab,
        group: Group::OnlyLocal,
        on: false,
    }));
    assert!(state(&app).rows.iter().all(|r| !r.selected));
}

/// Task 7's Open: with `suppress_auto`, navigating both panes keeps the run, rows and roots
/// and starts nothing.
#[tokio::test]
async fn suppress_auto_keeps_run_rows_and_roots_over_navigation() {
    let (mut app, fake, _a, tab, run) = running().await;
    app.update(found(tab, run, "kept"));
    let roots = state(&app).roots.clone();
    app.tabs[0].compare_mut().unwrap().suppress_auto = true;
    let mut all = Vec::new();
    let local = app.update(pane_msg(
        tab,
        Side::Local,
        PaneMsg::Navigate(rpath("/home/me/sub")),
    ));
    let remote = app.update(pane_msg(
        tab,
        Side::Remote,
        PaneMsg::Navigate(rpath("/srv")),
    ));
    let mut pending = run_listings(&fake, &local).await;
    pending.extend(run_listings(&fake, &remote).await);
    all.extend(local);
    all.extend(remote);
    all.extend(settle_all(&mut app, &fake, pending).await);
    assert!(cancelled(&all).is_empty() && started(&all).is_empty() && debounces(&all).is_empty());
    assert_eq!(state(&app).rows.len(), 1);
    assert_eq!(state(&app).roots, roots);
    assert_eq!(state(&app).run, Some(run));
    assert!(!state(&app).suppress_auto, "one-shot");
    // The next change behaves as usual again.
    let nav = app.update(pane_msg(
        tab,
        Side::Local,
        PaneMsg::Navigate(rpath("/home/me")),
    ));
    assert_eq!(cancelled(&nav), [run]);
}

/// Ruling R3: a re-list after a file operation neither cancels nor re-arms.
#[tokio::test]
async fn file_operation_refresh_does_not_touch_the_comparison() {
    let (mut app, fake, _a, tab, run) = running().await;
    app.update(found(tab, run, "kept"));
    let effects = app.update(pane_msg(tab, Side::Remote, PaneMsg::NewFolder));
    assert!(effects.is_empty() || started(&effects).is_empty());
    app.update(pane_msg(
        tab,
        Side::Remote,
        PaneMsg::EditChanged("made".into()),
    ));
    let commit = app.update(pane_msg(tab, Side::Remote, PaneMsg::EditCommit));
    let done = run_file_ops(&fake, &commit).await;
    let mut refresh = Vec::new();
    for m in done {
        refresh.extend(app.update(m));
    }
    assert_eq!(
        listings(&refresh).len(),
        1,
        "the pane re-lists: {refresh:?}"
    );
    assert!(cancelled(&refresh).is_empty());
    let after = settle_all(&mut app, &fake, run_listings(&fake, &refresh).await).await;
    assert!(debounces(&after).is_empty() && started(&after).is_empty());
    assert_eq!(state(&app).run, Some(run));
    assert_eq!(state(&app).rows.len(), 1);
    assert!(matches!(state(&app).phase, Phase::Running { .. }));
}

#[tokio::test]
async fn debouncing_never_sticks() {
    // Session gone with nothing running.
    let (mut app, _fake, a, effects) = open(true).await;
    assert_eq!(state(&app).phase, Phase::Debouncing);
    let _ = effects;
    let TabState::Connected(connected) = tab_state(&app, &a.id).clone() else {
        panic!("connected");
    };
    app.update(Message::SessionState {
        session: connected.session,
        state: ConnectionState::Disconnected(Error::new(ErrorKind::Network, "x")),
        auth: None,
    });
    assert_eq!(state(&app).phase, Phase::Idle);

    // Debounce fires, but the run cannot start (not connected any more).
    let (mut app, _fake, _a, effects) = open(true).await;
    let tab = app.tabs[0].id;
    let generation = debounces(&effects)[0].1;
    app.tabs[0].pane_mut(Side::Remote).unwrap().listing = crate::app::files::Listing::Loading;
    assert!(started(&app.update(msg(CompareMsg::Debounced { tab, generation }))).is_empty());
    assert_eq!(state(&app).phase, Phase::Idle);

    // Auto turned off in Settings while debouncing.
    let (mut app, _fake, _a, effects) = open(true).await;
    let tab = app.tabs[0].id;
    let generation = debounces(&effects)[0].1;
    app.update(Message::Settings(settings::SettingsMsg::CompareAuto(false)));
    assert_eq!(state(&app).phase, Phase::Idle);
    assert!(started(&app.update(msg(CompareMsg::Debounced { tab, generation }))).is_empty());
}

#[tokio::test]
async fn new_folders_clear_stale_rows_but_a_refresh_keeps_them() {
    // The previous run finished, auto is off.
    let (mut app, fake, _a, _e) = open(false).await;
    let tab = app.tabs[0].id;
    let effects = app.update(msg(CompareMsg::Start { tab }));
    let run = started(&effects)[0].0;
    app.update(found(tab, run, "old"));
    app.update(msg(CompareMsg::Finished {
        tab,
        run,
        result: Ok(Summary {
            identical: 2,
            differences: 1,
            partial: false,
            stopped: false,
        }),
    }));
    // A refresh keeps the result.
    let retry = app.update(pane_msg(tab, Side::Remote, PaneMsg::Retry));
    settle_all(&mut app, &fake, run_listings(&fake, &retry).await).await;
    assert_eq!(state(&app).rows.len(), 1);
    assert!(state(&app).roots.is_some());
    // A folder change drops it.
    let nav = app.update(pane_msg(
        tab,
        Side::Local,
        PaneMsg::Navigate(rpath("/home/me/sub")),
    ));
    assert!(cancelled(&nav).is_empty());
    assert!(state(&app).rows.is_empty() && state(&app).roots.is_none());
    assert_eq!(
        (state(&app).identical, &state(&app).phase),
        (0, &Phase::Idle)
    );
}

/// Reconnecting mid-run: the run goes on (only a `Disconnected` aborts it); once back, a
/// remote pane that failed meanwhile is re-listed, which cancels the run and re-arms the
/// automatic one.
#[tokio::test]
async fn reconnecting_keeps_the_run_and_the_relist_rearms() {
    let (mut app, fake, a, tab, run) = running().await;
    let TabState::Connected(connected) = tab_state(&app, &a.id).clone() else {
        panic!("connected");
    };
    let session = connected.session;
    let effects = app.update(Message::SessionState {
        session,
        state: ConnectionState::Reconnecting { attempt: 1 },
        auth: None,
    });
    assert!(aborted(&effects).is_empty() && cancelled(&effects).is_empty());
    assert_eq!(state(&app).run, Some(run));
    assert!(matches!(state(&app).phase, Phase::Running { .. }));

    app.tabs[0].pane_mut(Side::Remote).unwrap().listing =
        crate::app::files::Listing::Failed(Error::new(ErrorKind::Network, "lost"));
    let back = app.update(Message::SessionState {
        session,
        state: ConnectionState::Connected,
        auth: None,
    });
    assert_eq!(cancelled(&back), [run]);
    assert_eq!(listings(&back).len(), 1);
    let after = settle_all(&mut app, &fake, run_listings(&fake, &back).await).await;
    let waits = debounces(&after);
    assert_eq!(waits.len(), 1);
    let effects = app.update(msg(CompareMsg::Debounced {
        tab,
        generation: waits[0].1,
    }));
    assert_eq!(started(&effects).len(), 1);
}

/// A suppressed navigation whose listing fails must not swallow the next navigation.
#[tokio::test]
async fn a_failed_suppressed_listing_ends_the_suppression() {
    let (mut app, fake, _a, tab, run) = running().await;
    app.update(found(tab, run, "kept"));
    app.tabs[0].compare_mut().unwrap().suppress_auto = true;
    let nav = app.update(pane_msg(
        tab,
        Side::Local,
        PaneMsg::Navigate(rpath("/home/gone")),
    ));
    assert!(cancelled(&nav).is_empty());
    let failed = settle_all(&mut app, &fake, run_listings(&fake, &nav).await).await;
    assert!(started(&failed).is_empty() && debounces(&failed).is_empty());
    assert!(matches!(
        app.tabs[0].pane(Side::Local).unwrap().listing,
        crate::app::files::Listing::Failed(_)
    ));
    assert!(!state(&app).suppress_auto);

    // The next navigation cancels and re-arms as usual.
    let next = app.update(pane_msg(
        tab,
        Side::Local,
        PaneMsg::Navigate(rpath("/home/me/sub")),
    ));
    assert_eq!(cancelled(&next), [run]);
    assert!(state(&app).rows.is_empty() && state(&app).roots.is_none());
    let after = settle_all(&mut app, &fake, run_listings(&fake, &next).await).await;
    assert_eq!(debounces(&after).len(), 1);
}

#[tokio::test]
async fn collapse_focus_and_scroll_are_the_panels_own_state() {
    let (mut app, _fake, _a, tab, run) = running().await;
    app.update(found(tab, run, "a"));
    assert!(!state(&app).collapsed && !state(&app).focused);
    app.update(msg(CompareMsg::ToggleCollapsed { tab }));
    assert!(state(&app).collapsed);
    app.update(msg(CompareMsg::ToggleCollapsed { tab }));
    assert!(!state(&app).collapsed);
    app.update(msg(CompareMsg::Focus {
        tab,
        cursor: Some(0),
    }));
    assert!(state(&app).focused);
    assert_eq!(state(&app).cursor, Some(0));
    app.update(msg(CompareMsg::Scrolled {
        tab,
        offset_y: 60.0,
    }));
    assert_eq!(state(&app).offset_y, 60.0);
    // A press in a pane gives the keyboard back to the panes.
    app.update(Message::Pane {
        tab,
        side: Side::Local,
        msg: PaneMsg::Focus,
    });
    assert!(!state(&app).focused);
    // The panel's transfer buttons are no-ops until Task 7.
    assert!(
        app.update(msg(CompareMsg::UploadSelected { tab }))
            .is_empty()
    );
    assert!(
        app.update(msg(CompareMsg::DownloadSelected { tab }))
            .is_empty()
    );
}

#[tokio::test]
async fn keyboard_moves_in_group_order_and_space_toggles() {
    let (mut app, _fake, _a, tab, run) = running().await;
    // Arrival order: different, only-local, only-remote; shown: only-local, only-remote,
    // different.
    for (path, status) in [
        ("diff", Status::SizeDiffers),
        ("local", Status::OnlyLocal),
        ("remote", Status::OnlyRemote),
    ] {
        app.update(msg(CompareMsg::Progress {
            tab,
            run,
            progress: Progress::Found(Difference {
                path: path.into(),
                kind: hoprs_core::EntryKind::File,
                status,
                local: None,
                remote: None,
                entries_inside: None,
            }),
        }));
    }
    assert_eq!(state(&app).order(), vec![1, 2, 0]);
    // Not focused: the panel takes no keys.
    assert!(app.compare_key(0, FileKey::Down).is_none());
    app.update(msg(CompareMsg::Focus { tab, cursor: None }));
    let down = |app: &mut App| {
        let effects = app.update(Message::Key {
            key: FileKey::Down,
            shift: false,
        });
        assert!(matches!(&effects[..], [Effect::ScrollCompare { .. }]));
    };
    down(&mut app);
    assert_eq!(state(&app).cursor, Some(1));
    down(&mut app);
    down(&mut app);
    assert_eq!(state(&app).cursor, Some(0));
    down(&mut app);
    assert_eq!(state(&app).cursor, Some(0), "stops at the end");
    app.update(Message::Key {
        key: FileKey::Up,
        shift: false,
    });
    assert_eq!(state(&app).cursor, Some(2));
    app.update(Message::Key {
        key: FileKey::Space,
        shift: false,
    });
    assert!(state(&app).rows[2].selected);
    app.update(Message::Key {
        key: FileKey::Space,
        shift: false,
    });
    assert!(!state(&app).rows[2].selected);
    // Tab leaves the panel instead of switching panes.
    let focus = app.tabs[0].focus();
    app.update(Message::Key {
        key: FileKey::SwitchPane,
        shift: false,
    });
    assert!(!state(&app).focused);
    assert_eq!(app.tabs[0].focus(), focus);
}

#[tokio::test]
async fn leaving_settings_without_closing_clears_the_typed_fields() {
    let mut app = App::new();
    app.form = Some(Form::Settings);
    app.compare_ignore_input = Some("a, b".into());
    app.editor_input = Some("code".into());
    app.update(Message::Modifiers(Mods::default()));
    assert_eq!(
        app.compare_ignore_input.as_deref(),
        Some("a, b"),
        "still open"
    );
    app.form = None;
    app.update(Message::Modifiers(Mods::default()));
    assert_eq!(app.compare_ignore_input, None);
    assert_eq!(app.editor_input, None);
}

#[tokio::test]
async fn a_new_run_and_hide_release_the_panels_keyboard() {
    let (mut app, _fake, _a, tab, run) = running().await;
    app.update(found(tab, run, "a"));
    app.update(msg(CompareMsg::Focus {
        tab,
        cursor: Some(0),
    }));
    app.update(msg(CompareMsg::Hide { tab }));
    assert!(!state(&app).focused);
    assert_eq!(state(&app).cursor, None);
    app.update(msg(CompareMsg::Start { tab }));
    app.update(msg(CompareMsg::Focus { tab, cursor: None }));
    assert!(state(&app).focused);
    app.update(msg(CompareMsg::Start { tab }));
    assert!(!state(&app).focused);
    assert_eq!(state(&app).cursor, None);
}

// ---- Task 7: Upload / Download / Open ----

use hoprs_core::{EntryKind, RemoteEntry};
use hoprs_remote::CompareSide;
use hoprs_transfer::{ConflictPolicy, Direction, EventKind, TransferState};

fn entry(name: &str, kind: EntryKind, size: u64) -> RemoteEntry {
    RemoteEntry {
        name: name.to_owned(),
        path: rpath(&format!("/x/{name}")),
        kind,
        size,
        modified: Some(SystemTime::UNIX_EPOCH),
        mode: None,
    }
}

/// A difference found by the run, selected.
fn diff_row(
    app: &mut App,
    tab: TabId,
    run: CompareRunId,
    path: &str,
    kind: EntryKind,
    status: Status,
    sizes: (Option<u64>, Option<u64>),
) {
    let name = path.rsplit('/').next().unwrap();
    app.update(msg(CompareMsg::Progress {
        tab,
        run,
        progress: Progress::Found(Difference {
            path: path.to_owned(),
            kind,
            status,
            local: sizes.0.map(|s| entry(name, kind, s)),
            remote: sizes.1.map(|s| entry(name, kind, s)),
            entries_inside: None,
        }),
    }));
    let index = state(app).rows.len() - 1;
    app.update(msg(CompareMsg::Select {
        tab,
        index,
        on: true,
    }));
}

fn enqueue_effect(effects: &[Effect]) -> Vec<crate::app::CompareItem> {
    effects
        .iter()
        .find_map(|e| match e {
            Effect::EnqueueCompare { items, .. } => Some(items.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

/// Runs the panel's enqueue effect against the fake and delivers the ids.
async fn deliver_enqueue(
    app: &mut App,
    fake: &Arc<FakeBackend>,
    effects: Vec<Effect>,
) -> Vec<Effect> {
    let backend: Arc<dyn BackendApi> = fake.clone();
    let mut out = Vec::new();
    for effect in effects {
        if let Effect::EnqueueCompare {
            tab,
            run,
            roots,
            items,
        } = effect
        {
            let results = effects::enqueue_compare(backend.clone(), items).await;
            out.extend(app.update(msg(CompareMsg::Enqueued {
                tab,
                run,
                roots,
                results,
            })));
        }
    }
    out
}

fn ended(seq: u64, id: TransferId, state: TransferState) -> Message {
    Message::TransferEvent(hoprs_transfer::Event {
        seq,
        kind: EventKind::StateChanged { id, state },
    })
}

fn row_state(app: &App, path: &str) -> RowState {
    state(app)
        .rows
        .iter()
        .find(|r| r.diff.path == path)
        .unwrap()
        .state
        .clone()
}

use crate::app::compare::RowState;

#[tokio::test]
async fn upload_selected_queues_files_with_overwrite_and_folders_plainly() {
    let (mut app, fake, _a, tab, run) = running().await;
    let sizes = |l, r| (l, r);
    diff_row(
        &mut app,
        tab,
        run,
        "new.txt",
        EntryKind::File,
        Status::OnlyLocal,
        sizes(Some(3), None),
    );
    diff_row(
        &mut app,
        tab,
        run,
        "sub/old.txt",
        EntryKind::File,
        Status::LocalNewer,
        sizes(Some(5), Some(4)),
    );
    diff_row(
        &mut app,
        tab,
        run,
        "dir",
        EntryKind::Dir,
        Status::OnlyLocal,
        sizes(Some(0), None),
    );
    // Not eligible: only on the server, a file against a folder, a symlink, unreadable.
    diff_row(
        &mut app,
        tab,
        run,
        "r.txt",
        EntryKind::File,
        Status::OnlyRemote,
        sizes(None, Some(1)),
    );
    diff_row(
        &mut app,
        tab,
        run,
        "k",
        EntryKind::File,
        Status::KindDiffers,
        sizes(Some(1), Some(1)),
    );
    diff_row(
        &mut app,
        tab,
        run,
        "l",
        EntryKind::Symlink,
        Status::OnlyLocal,
        sizes(Some(1), None),
    );

    let effects = app.update(msg(CompareMsg::UploadSelected { tab }));
    let items = enqueue_effect(&effects);
    assert_eq!(
        items
            .iter()
            .map(|i| (i.path.as_str(), i.overwrite))
            .collect::<Vec<_>>(),
        [("new.txt", true), ("sub/old.txt", true), ("dir", false)]
    );
    assert_eq!(items[1].spec.direction, Direction::Upload);
    assert_eq!(items[1].spec.local.to_string(), "/home/me/sub/old.txt");
    assert_eq!(items[1].spec.remote.to_string(), "/srv/sub/old.txt");
    assert!(
        state(&app)
            .rows
            .iter()
            .filter(|r| r.diff.status == Status::OnlyLocal || r.diff.status == Status::LocalNewer)
            .filter(|r| r.diff.kind != EntryKind::Symlink)
            .all(|r| !r.selected),
        "checkboxes of the queued rows cleared"
    );
    assert!(
        state(&app)
            .rows
            .iter()
            .any(|r| r.diff.path == "r.txt" && r.selected),
        "a row that was not sent stays selected"
    );

    deliver_enqueue(&mut app, &fake, effects).await;
    assert!(matches!(row_state(&app, "new.txt"), RowState::Queued(_)));
    assert!(matches!(row_state(&app, "dir"), RowState::Queued(_)));
    assert_eq!(row_state(&app, "r.txt"), RowState::Idle);
    let policies: Vec<_> = fake
        .enqueued_with_policy()
        .into_iter()
        .map(|(_, p)| p)
        .collect();
    assert_eq!(
        policies,
        [
            Some(ConflictPolicy::Overwrite),
            Some(ConflictPolicy::Overwrite),
            None
        ]
    );
}

#[tokio::test]
async fn download_selected_goes_the_other_way() {
    let (mut app, fake, _a, tab, run) = running().await;
    diff_row(
        &mut app,
        tab,
        run,
        "r.txt",
        EntryKind::File,
        Status::OnlyRemote,
        (None, Some(1)),
    );
    diff_row(
        &mut app,
        tab,
        run,
        "n.txt",
        EntryKind::File,
        Status::OnlyLocal,
        (Some(1), None),
    );
    let effects = app.update(msg(CompareMsg::DownloadSelected { tab }));
    let items = enqueue_effect(&effects);
    assert_eq!(items.len(), 1, "the local-only row is not downloadable");
    assert_eq!(items[0].spec.direction, Direction::Download);
    assert_eq!(items[0].spec.remote.to_string(), "/srv/r.txt");
    deliver_enqueue(&mut app, &fake, effects).await;
    assert!(matches!(row_state(&app, "r.txt"), RowState::Queued(_)));
    // Nothing selected: nothing happens.
    assert!(enqueue_effect(&app.update(msg(CompareMsg::DownloadSelected { tab }))).is_empty());
}

#[tokio::test]
async fn transfer_events_move_rows_and_unreadable_is_never_queued() {
    let (mut app, fake, _a, tab, run) = running().await;
    app.settings.compare_auto = false; // no re-run clearing the rows
    for p in ["a", "b", "c", "d"] {
        diff_row(
            &mut app,
            tab,
            run,
            p,
            EntryKind::File,
            Status::OnlyLocal,
            (Some(1), None),
        );
    }
    diff_row(
        &mut app,
        tab,
        run,
        "u",
        EntryKind::Dir,
        Status::Unreadable {
            side: CompareSide::Remote,
            error: Error::new(ErrorKind::Permission, "no"),
        },
        (Some(0), None),
    );
    let effects = app.update(msg(CompareMsg::UploadSelected { tab }));
    assert_eq!(enqueue_effect(&effects).len(), 4);
    deliver_enqueue(&mut app, &fake, effects).await;
    let ids: std::collections::HashMap<String, TransferId> = ["a", "b", "c", "d"]
        .iter()
        .map(|p| match row_state(&app, p) {
            RowState::Queued(id) => (p.to_string(), id),
            other => panic!("{other:?}"),
        })
        .collect();
    let id = |p: &str| ids[p];
    app.update(ended(1, id("a"), TransferState::Active));
    assert!(matches!(row_state(&app, "a"), RowState::Queued(_)));
    app.update(ended(2, id("a"), TransferState::Done));
    app.update(ended(
        3,
        id("b"),
        TransferState::Failed(Error::new(ErrorKind::Io, "boom")),
    ));
    app.update(ended(4, id("c"), TransferState::Skipped("exists".into())));
    app.update(ended(5, id("d"), TransferState::Cancelled));
    assert_eq!(row_state(&app, "a"), RowState::Done);
    assert!(matches!(row_state(&app, "b"), RowState::Failed(_)));
    assert_eq!(row_state(&app, "c"), RowState::Skipped("exists".into()));
    assert_eq!(row_state(&app, "d"), RowState::Idle, "selectable again");
    assert_eq!(row_state(&app, "u"), RowState::Idle);
}

#[tokio::test]
async fn an_ending_that_beats_the_ids_is_not_lost() {
    let (mut app, fake, _a, tab, run) = running().await;
    app.settings.compare_auto = false; // no re-run clearing the rows
    diff_row(
        &mut app,
        tab,
        run,
        "a",
        EntryKind::File,
        Status::OnlyLocal,
        (Some(1), None),
    );
    let effects = app.update(msg(CompareMsg::UploadSelected { tab }));
    let backend: Arc<dyn BackendApi> = fake.clone();
    let items = enqueue_effect(&effects);
    let roots = match &effects[0] {
        Effect::EnqueueCompare { roots, .. } => roots.clone(),
        other => panic!("{other:?}"),
    };
    let results = effects::enqueue_compare(backend, items).await;
    let id = *results[0].result.as_ref().unwrap();
    // The engine's event comes before the result.
    app.update(ended(1, id, TransferState::Done));
    app.update(msg(CompareMsg::Enqueued {
        tab,
        run,
        roots,
        results,
    }));
    assert_eq!(row_state(&app, "a"), RowState::Done);
}

#[tokio::test]
async fn done_files_are_remembered_and_the_next_run_gets_them() {
    let (mut app, fake, a, tab, run) = running().await;
    diff_row(
        &mut app,
        tab,
        run,
        "d/f.bin",
        EntryKind::File,
        Status::LocalNewer,
        (Some(7), Some(5)),
    );
    diff_row(
        &mut app,
        tab,
        run,
        "dir",
        EntryKind::Dir,
        Status::OnlyLocal,
        (Some(0), None),
    );
    let effects = app.update(msg(CompareMsg::UploadSelected { tab }));
    deliver_enqueue(&mut app, &fake, effects).await;
    let (RowState::Queued(f), RowState::Queued(d)) =
        (row_state(&app, "d/f.bin"), row_state(&app, "dir"))
    else {
        panic!()
    };
    app.update(ended(1, f, TransferState::Done));
    app.update(ended(2, d, TransferState::Done));
    assert_eq!(state(&app).synced.len(), 1, "folders are not remembered");
    assert_eq!(
        state(&app).synced[&(rpath("/home/me/d/f.bin"), rpath("/srv/d/f.bin"))],
        (7, 7)
    );

    let effects = app.update(msg(CompareMsg::Start { tab }));
    let known = effects
        .iter()
        .find_map(|e| match e {
            Effect::Compare { options, .. } => Some(options.known_synced.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(known["d/f.bin"], (7, 7));

    // Visit the subfolder: the run there benefits (its own relative path), and coming back
    // the memory is still there.
    fake.local_fs().add_dir("/home/me/d");
    fake.remote_fs(&a.id).add_dir("/srv/d");
    for (path, expect) in [("d", "f.bin"), ("..", "d/f.bin")] {
        for side in [Side::Local, Side::Remote] {
            let root = if side == Side::Local {
                "/home/me"
            } else {
                "/srv"
            };
            let target = if path == ".." {
                root.to_owned()
            } else {
                format!("{root}/{path}")
            };
            let nav = app.update(pane_msg(tab, side, PaneMsg::Navigate(rpath(&target))));
            settle_all(&mut app, &fake, run_listings(&fake, &nav).await).await;
        }
        let effects = app.update(msg(CompareMsg::Start { tab }));
        let known = effects
            .iter()
            .find_map(|e| match e {
                Effect::Compare { options, .. } => Some(options.known_synced.clone()),
                _ => None,
            })
            .expect("a run starts");
        assert_eq!(known.get(expect), Some(&(7, 7)), "{path}: {known:?}");
        assert_eq!(known.len(), 1);
    }
}

#[tokio::test]
async fn auto_reruns_once_when_the_whole_batch_has_ended() {
    let (mut app, fake, _a, tab, run) = running().await;
    for p in ["a", "b"] {
        diff_row(
            &mut app,
            tab,
            run,
            p,
            EntryKind::File,
            Status::OnlyLocal,
            (Some(1), None),
        );
    }
    let effects = app.update(msg(CompareMsg::UploadSelected { tab }));
    deliver_enqueue(&mut app, &fake, effects).await;
    let (RowState::Queued(a), RowState::Queued(b)) = (row_state(&app, "a"), row_state(&app, "b"))
    else {
        panic!()
    };
    let first = app.update(ended(1, a, TransferState::Done));
    assert!(started(&first).is_empty(), "b is still going");
    let last = app.update(ended(
        2,
        b,
        TransferState::Failed(Error::new(ErrorKind::Io, "x")),
    ));
    assert_eq!(started(&last).len(), 1, "one re-run: {last:?}");
    assert_eq!(
        cancelled(&last),
        [run],
        "it replaces the finished-looking run"
    );
    // Late duplicates start nothing more.
    assert!(started(&app.update(ended(3, b, TransferState::Done))).is_empty());

    // With automatic off there is no re-run.
    app.settings.compare_auto = false;
    let run2 = state(&app).run.unwrap();
    diff_row(
        &mut app,
        tab,
        run2,
        "c",
        EntryKind::File,
        Status::OnlyLocal,
        (Some(1), None),
    );
    let effects = app.update(msg(CompareMsg::UploadSelected { tab }));
    deliver_enqueue(&mut app, &fake, effects).await;
    let RowState::Queued(c) = row_state(&app, "c") else {
        panic!()
    };
    assert!(started(&app.update(ended(4, c, TransferState::Done))).is_empty());
}

#[tokio::test]
async fn open_goes_to_the_items_folder_without_a_new_run() {
    let (mut app, fake, _a, tab, run) = running().await;
    app.update(msg(CompareMsg::Progress {
        tab,
        run,
        progress: Progress::Found(Difference {
            path: "sub/deep/x.txt".into(),
            kind: EntryKind::File,
            status: Status::OnlyLocal,
            local: Some(entry("x.txt", EntryKind::File, 1)),
            remote: None,
            entries_inside: None,
        }),
    }));
    let nav = app.update(msg(CompareMsg::Open { tab, index: 0 }));
    assert_eq!(listings(&nav).len(), 2, "both panes list: {nav:?}");
    assert!(cancelled(&nav).is_empty(), "the run stays");
    assert_eq!(
        app.tabs[0].pane(Side::Local).unwrap().path.to_string(),
        "/home/me/sub/deep"
    );
    assert_eq!(
        app.tabs[0].pane(Side::Remote).unwrap().path.to_string(),
        "/srv/sub/deep"
    );
    let effects = settle_all(&mut app, &fake, run_listings(&fake, &nav).await).await;
    assert!(debounces(&effects).is_empty() && started(&effects).is_empty());
    assert_eq!(state(&app).run, Some(run));
    assert_eq!(state(&app).rows.len(), 1);
    assert_eq!(
        state(&app).roots.as_ref().map(|r| r.0.to_string()),
        Some("/home/me".into())
    );
    assert!(!state(&app).suppress_auto);
}

#[tokio::test]
async fn open_inside_a_one_sided_folder_goes_up_to_the_nearest_existing_one() {
    let (mut app, _fake, _a, tab, run) = running().await;
    // `sub` exists only locally: the server side stops at the root.
    diff_row(
        &mut app,
        tab,
        run,
        "sub",
        EntryKind::Dir,
        Status::OnlyLocal,
        (Some(0), None),
    );
    diff_row(
        &mut app,
        tab,
        run,
        "sub/in/f",
        EntryKind::File,
        Status::OnlyLocal,
        (Some(1), None),
    );
    let nav = app.update(msg(CompareMsg::Open { tab, index: 1 }));
    assert_eq!(
        app.tabs[0].pane(Side::Local).unwrap().path.to_string(),
        "/home/me/sub/in"
    );
    assert_eq!(
        app.tabs[0].pane(Side::Remote).unwrap().path.to_string(),
        "/srv"
    );
    assert_eq!(listings(&nav).len(), 1, "the remote pane is already there");
}

#[tokio::test]
async fn open_at_the_roots_does_not_arm_the_suppression() {
    let (mut app, _fake, _a, tab, run) = running().await;
    diff_row(
        &mut app,
        tab,
        run,
        "top.txt",
        EntryKind::File,
        Status::OnlyLocal,
        (Some(1), None),
    );
    let nav = app.update(msg(CompareMsg::Open { tab, index: 0 }));
    assert!(nav.is_empty());
    assert!(!state(&app).suppress_auto, "no listing would consume it");
}

#[tokio::test]
async fn ids_of_an_older_run_are_not_attached_to_a_newer_runs_rows() {
    let (mut app, fake, _a, tab, run) = running().await;
    diff_row(
        &mut app,
        tab,
        run,
        "a",
        EntryKind::File,
        Status::OnlyLocal,
        (Some(1), None),
    );
    let effects = app.update(msg(CompareMsg::UploadSelected { tab }));
    // A new run finds the same path before the ids come back.
    let restarted = app.update(msg(CompareMsg::Start { tab }));
    let run2 = started(&restarted)[0].0;
    diff_row(
        &mut app,
        tab,
        run2,
        "a",
        EntryKind::File,
        Status::OnlyLocal,
        (Some(1), None),
    );
    deliver_enqueue(&mut app, &fake, effects).await;
    assert_eq!(row_state(&app, "a"), RowState::Idle);
    assert!(state(&app).queued.is_empty());
    assert_eq!(state(&app).enqueuing, 0);
}
