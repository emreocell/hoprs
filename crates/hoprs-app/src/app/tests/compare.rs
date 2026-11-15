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
