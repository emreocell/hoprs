//! Plan 6 Task 8: remote editing in the UI (the Edit action, the notices, the inline
//! conflict choice, and the questions before a session or the window closes: Review
//! Focus 4). The backend's edits are the real `EditManager` behind the fake.

use super::*;

use crate::app::edit::{
    CloseAnswer, EDIT_MAX_BYTES, EditStatus, EditsFor, FLUSH_LIMIT, QuestionFile, Settle,
    TOAST_TIME,
};
use crate::backend::fake::Call;
use crate::backend::{EditChoice, EditEvent, EditId};
use crate::ui::edits::{clock, edit_line};
use tokio::sync::broadcast;

fn key(key: FileKey) -> Message {
    Message::Key { key, shift: false }
}

fn edit(msg: EditMsg) -> Message {
    Message::Edit(msg)
}

fn srv_site(protocol: Protocol) -> Site {
    let mut a = site("a", "h", "u", protocol);
    a.remote_dir = Some("/srv".into());
    a
}

/// A connected files tab of `a`, the remote pane focused.
async fn remote_focused(fake: &Arc<FakeBackend>, a: &Site) -> App {
    let mut app = connected_tab(fake, a).await;
    app.update(key(FileKey::SwitchPane));
    assert_eq!(app.tabs[0].focus(), Some(Side::Remote));
    app
}

/// Clicks the remote entry `name` of the first tab (selects it).
fn click(app: &mut App, name: &str) {
    let pane = app.tabs[0].remote();
    let row = usize::from(pane.has_up())
        + pane
            .entries
            .iter()
            .position(|e| e.name == name)
            .unwrap_or_else(|| panic!("no {name}"));
    let tab = app.tabs[0].id;
    app.update(pane_msg(tab, Side::Remote, PaneMsg::Click(row)));
}

fn start_effects(effects: &[Effect]) -> Vec<(TabId, SessionId, String, String)> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::StartEdit {
                tab,
                session,
                remote,
                editor,
            } => Some((*tab, *session, remote.as_str().to_owned(), editor.clone())),
            _ => None,
        })
        .collect()
}

/// Runs the one `StartEdit` of `effects` against the fake, as `effects::run` does, and
/// feeds its result.
async fn run_start(app: &mut App, fake: &Arc<FakeBackend>, effects: &[Effect]) -> EditId {
    let [(tab, session, remote, editor)] = &start_effects(effects)[..] else {
        panic!("one StartEdit expected: {effects:?}");
    };
    let remote = rpath(remote);
    let result = fake
        .start_edit(*session, remote.clone(), editor.clone())
        .await;
    let id = *result.as_ref().expect("the edit starts");
    app.update(edit(EditMsg::Started {
        tab: *tab,
        session: *session,
        remote,
        result,
    }));
    id
}

async fn next_event(events: &mut broadcast::Receiver<EditEvent>) -> EditEvent {
    tokio::time::timeout(Duration::from_secs(10), events.recv())
        .await
        .expect("an edit event in time")
        .expect("the events go on")
}

/// A connected tab of `a` editing `/srv/a.txt` ("v1"): the app, the session and the edit.
async fn editing(fake: &Arc<FakeBackend>, a: &Site) -> (App, SessionId, EditId) {
    fake.remote_fs(&a.id)
        .add_file("/srv/a.txt", b"v1", SystemTime::UNIX_EPOCH);
    let mut app = remote_focused(fake, a).await;
    click(&mut app, "a.txt");
    let effects = app.update(key(FileKey::Edit));
    let id = run_start(&mut app, fake, &effects).await;
    let session = app.tab_session(0).unwrap();
    (app, session, id)
}

/// The file of the edit holds `data`, saved by the editor but not seen by the watcher
/// yet: unsent, as the backend reports it.
fn save_unseen(fake: &FakeBackend, id: EditId, data: &[u8]) -> PathBuf {
    let local = fake.edit_local(id).expect("a live edit");
    std::fs::write(&local, data).unwrap();
    local
}

fn the_edit(app: &App) -> &crate::app::edit::RemoteEdit {
    assert_eq!(app.edits.len(), 1, "{:?}", app.edits);
    &app.edits[0]
}

/// F4 / "Edit" on the remote pane: off for folders, links and files over 50 MB (the
/// reason inline, nothing started); a file opens with the editor of the settings.
#[tokio::test]
async fn edit_action_rules() {
    let a = srv_site(Protocol::Ftp);
    let fake = Arc::new(FakeBackend::new().with_site(a.clone()));
    let remote = fake.remote_fs(&a.id);
    remote.add_file("/srv/a.txt", b"text", SystemTime::UNIX_EPOCH);
    remote.add_file("/srv/big.bin", b"x", SystemTime::UNIX_EPOCH);
    remote.add_file("/srv/max.bin", b"x", SystemTime::UNIX_EPOCH);
    remote.add_dir("/srv/dir");
    remote.add_symlink("/srv/link", "/srv/a.txt");
    let mut app = remote_focused(&fake, &a).await;
    let tab = app.tabs[0].id;
    // The sizes as listed (a 50 MB file is not worth writing for a test).
    for entry in &mut app.tabs[0].remote_mut().entries {
        match entry.name.as_str() {
            "big.bin" => entry.size = EDIT_MAX_BYTES + 1,
            "max.bin" => entry.size = EDIT_MAX_BYTES,
            _ => {}
        }
    }
    assert_eq!(EDIT_MAX_BYTES, 50 * 1024 * 1024);

    // Nothing selected: nothing to edit.
    assert_eq!(app.edit_target(0, Side::Remote), None);
    assert!(app.update(key(FileKey::Edit)).is_empty());

    for (name, reason) in [
        ("dir", strings::edit_no_folders()),
        ("link", strings::edit_no_links()),
        ("big.bin", strings::edit_too_large()),
    ] {
        click(&mut app, name);
        assert_eq!(
            app.edit_target(0, Side::Remote),
            Some(Err(reason)),
            "{name}: the toolbar's Edit is off, with the reason"
        );
        let effects = app.update(key(FileKey::Edit));
        assert!(start_effects(&effects).is_empty(), "{name}: {effects:?}");
        let effects = app.update(pane_msg(tab, Side::Remote, PaneMsg::Edit));
        assert!(start_effects(&effects).is_empty(), "{name}: {effects:?}");
        assert_eq!(
            app.tabs[0].remote().note.as_deref(),
            Some(reason),
            "{name}: the reason shows inline"
        );
    }

    // Exactly 50 MB is fine; F4 opens it with the default editor.
    click(&mut app, "max.bin");
    assert_eq!(
        app.edit_target(0, Side::Remote),
        Some(Ok(rpath("/srv/max.bin")))
    );
    let session = app.tab_session(0).unwrap();
    let effects = app.update(key(FileKey::Edit));
    assert_eq!(
        start_effects(&effects),
        vec![(tab, session, "/srv/max.bin".into(), "system".into())]
    );
    // Meanwhile the strip says it is opening.
    assert_eq!(app.edit_opening.len(), 1);

    // The toolbar's button does the same, with the editor the settings name.
    app.update(Message::Settings(settings::SettingsMsg::EditorCommand(
        "gedit --wait".into(),
    )));
    click(&mut app, "a.txt");
    let effects = app.update(pane_msg(tab, Side::Remote, PaneMsg::Edit));
    assert_eq!(
        start_effects(&effects),
        vec![(tab, session, "/srv/a.txt".into(), "gedit --wait".into())]
    );
    let id = run_start(&mut app, &fake, &effects).await;
    assert_eq!(
        fake.edits_launched()
            .last()
            .map(|(editor, _)| editor.as_str()),
        Some("gedit --wait")
    );
    assert_eq!(app.edits[0].id, id);

    // The local pane has no Edit.
    app.update(key(FileKey::SwitchPane));
    assert_eq!(app.edit_target(0, Side::Local), None);
    assert!(app.update(key(FileKey::Edit)).is_empty());
}

/// Each upload: the line says when it was saved, and a toast shows for 3 s (a newer
/// toast is not ended by an older one's timer). A held save and a failure say so.
#[tokio::test]
async fn uploaded_shows_toast_and_status() {
    let a = srv_site(Protocol::Ssh);
    let fake = Arc::new(FakeBackend::new().with_site(a.clone()));
    let mut events = fake.subscribe_edits();
    let (mut app, session, id) = editing(&fake, &a).await;
    assert!(app.edit_opening.is_empty());
    assert_eq!(the_edit(&app).status, EditStatus::Editing { saved: None });
    assert_eq!(edit_line(the_edit(&app)), "Editing a.txt");

    fake.save_edit(session, id, b"v2");
    let uploaded = next_event(&mut events).await;
    let EditEvent::Uploaded { at, .. } = uploaded else {
        panic!("{uploaded:?}");
    };
    let effects = app.update(edit(EditMsg::Event(uploaded)));
    assert_eq!(
        fake.remote_fs(&a.id).read_file("/srv/a.txt").unwrap(),
        b"v2"
    );
    assert_eq!(
        the_edit(&app).status,
        EditStatus::Editing { saved: Some(at) }
    );
    assert_eq!(
        edit_line(the_edit(&app)),
        format!("Editing a.txt — saved {}", clock(at))
    );
    // The file name is its own (mono) span.
    let (before, name, after) = crate::ui::edits::edit_line_parts(the_edit(&app));
    assert_eq!((before.as_str(), name.as_str()), ("Editing ", "a.txt"));
    assert_eq!(after, format!(" — saved {}", clock(at)));
    let toast = app.toast.clone().expect("a toast");
    assert_eq!((toast.text, toast.name.as_str()), ("Uploaded", "a.txt"));
    assert_eq!(TOAST_TIME, Duration::from_secs(3));
    let [Effect::Later { after, message }] = &effects[..] else {
        panic!("{effects:?}");
    };
    assert_eq!(*after, TOAST_TIME);
    assert!(matches!(**message, Message::Edit(EditMsg::ToastExpired(t)) if t == toast.id));

    // A second upload: a new toast, which the first one's timer leaves alone.
    app.update(edit(EditMsg::Event(EditEvent::Uploaded {
        id,
        at: SystemTime::now(),
    })));
    let second = app.toast.clone().unwrap();
    assert_ne!(second.id, toast.id);
    app.update(edit(EditMsg::ToastExpired(toast.id)));
    assert_eq!(app.toast.as_ref(), Some(&second));
    app.update(edit(EditMsg::ToastExpired(second.id)));
    assert_eq!(app.toast, None);

    // Held while the connection is down; a possibly truncated copy is said too.
    app.update(edit(EditMsg::Event(EditEvent::Pending {
        id,
        remote_may_be_truncated: true,
    })));
    assert_eq!(
        edit_line(the_edit(&app)),
        format!(
            "Editing a.txt — {} {}",
            strings::waiting_for_connection(),
            strings::remote_may_be_truncated()
        )
    );
    assert_eq!(app.toast, None, "only uploads make a toast");

    // A failure: the reason and "Retry".
    app.update(edit(EditMsg::Event(EditEvent::Failed {
        id,
        error: Error::new(ErrorKind::Io, "Disk full"),
        remote_may_be_truncated: false,
        recovered: None,
    })));
    assert_eq!(edit_line(the_edit(&app)), "a.txt: Upload failed: Disk full");
    let effects = app.update(edit(EditMsg::Retry(id)));
    assert!(matches!(effects[..], [Effect::RetryEdit(got)] if got == id));

    // The edit ends: its line goes.
    app.update(edit(EditMsg::Event(EditEvent::Closed { id })));
    assert!(app.edits.is_empty());
}

/// Review Focus 2 in the UI: a change on the server holds the upload and offers
/// "Overwrite / Keep remote / Save a copy" inline (no dialog); the choice runs once.
#[tokio::test]
async fn remote_changed_choice_inline() {
    let a = srv_site(Protocol::Ssh);
    let fake = Arc::new(FakeBackend::new().with_site(a.clone()));
    let mut events = fake.subscribe_edits();
    let (mut app, session, id) = editing(&fake, &a).await;
    let remote = fake.remote_fs(&a.id);
    let later = SystemTime::UNIX_EPOCH + Duration::from_secs(5_000);
    remote.add_file("/srv/a.txt", b"theirs", later);

    fake.save_edit(session, id, b"mine");
    let changed = next_event(&mut events).await;
    assert_eq!(changed, EditEvent::RemoteChanged { id });
    app.update(edit(EditMsg::Event(changed)));
    assert_eq!(
        the_edit(&app).status,
        EditStatus::Conflict {
            busy: false,
            error: None,
            recovered: None
        }
    );
    assert_eq!(
        edit_line(the_edit(&app)),
        "a.txt: Changed on the server since you opened it"
    );
    assert!(!app.dialogs.is_open(), "inline, never a dialog");
    assert!(app.form.is_none());
    assert_eq!(remote.read_file("/srv/a.txt").unwrap(), b"theirs");

    // "Keep remote": one request, the buttons off meanwhile.
    let effects = app.update(edit(EditMsg::Resolve {
        id,
        choice: EditChoice::KeepRemote,
    }));
    assert!(matches!(
        effects[..],
        [Effect::ResolveEdit {
            id: got,
            choice: EditChoice::KeepRemote
        }] if got == id
    ));
    assert!(matches!(
        the_edit(&app).status,
        EditStatus::Conflict { busy: true, .. }
    ));
    assert!(
        app.update(edit(EditMsg::Resolve {
            id,
            choice: EditChoice::Overwrite
        }))
        .is_empty()
    );
    let result = fake.resolve_edit(id, EditChoice::KeepRemote).await;
    let replaced = next_event(&mut events).await;
    app.update(edit(EditMsg::Event(replaced)));
    app.update(edit(EditMsg::Done { id, result }));
    let EditStatus::Replaced {
        recovered: Some(copy),
    } = &the_edit(&app).status
    else {
        panic!("{:?}", the_edit(&app).status);
    };
    assert_eq!(
        std::fs::read(copy).unwrap(),
        b"mine",
        "the user's version is safe"
    );
    assert_eq!(remote.read_file("/srv/a.txt").unwrap(), b"theirs");

    // A choice that fails keeps the choice, with the reason.
    app.update(edit(EditMsg::Event(EditEvent::RemoteChanged { id })));
    app.update(edit(EditMsg::Resolve {
        id,
        choice: EditChoice::Overwrite,
    }));
    app.update(edit(EditMsg::Done {
        id,
        result: Err(Error::new(ErrorKind::Network, "Connection lost")),
    }));
    let EditStatus::Conflict {
        busy: false,
        error: Some(error),
        ..
    } = &the_edit(&app).status
    else {
        panic!("{:?}", the_edit(&app).status);
    };
    assert_eq!(error.message, "Connection lost");
}

/// Review Focus 4: the last tab of a session with edits asks inline before closing
/// (another tab of the session closes as usual). "Upload pending changes and close" sends
/// the unsent change first; the tab and the session close only after. An upload that
/// cannot finish keeps the question, and "Close and keep local copies" keeps the files.
#[tokio::test]
async fn closing_last_tab_with_edits_asks() {
    assert_eq!(FLUSH_LIMIT, Duration::from_secs(10));
    let a = srv_site(Protocol::Ssh);
    let fake = Arc::new(FakeBackend::new().with_site(a.clone()));
    let backend: Arc<dyn BackendApi> = fake.clone();
    let (mut app, session, id) = editing(&fake, &a).await;
    let first = app.tabs[0].id;
    app.update(Message::NewTab {
        tab: first,
        kind: TabKind::Files,
    });
    assert_eq!(app.tabs.len(), 2);
    let second = app.tabs[1].id;
    let local = save_unseen(&fake, id, b"unsent");

    // Not the last tab: it closes, no question.
    let effects = app.update(Message::CloseTab(second));
    assert!(!effects.iter().any(|e| matches!(e, Effect::CloseSession(_))));
    assert_eq!(app.tabs.len(), 1);
    assert!(app.close_asks.is_empty());

    // The last one: the backend is asked what is unsent; nothing closes.
    let effects = app.update(Message::CloseTab(first));
    let [Effect::CheckEdits { purpose, sessions }] = &effects[..] else {
        panic!("{effects:?}");
    };
    assert_eq!(
        (purpose.clone(), sessions.clone()),
        (EditsFor::Tab(first), vec![session])
    );
    assert_eq!(app.tabs.len(), 1);
    assert!(
        app.update(Message::CloseTab(first)).is_empty(),
        "asked once"
    );
    let checked = effects::check_edits(backend.clone(), purpose.clone(), sessions.clone()).await;
    assert!(app.update(checked).is_empty());
    let question = &app.close_asks[&first];
    assert_eq!(
        question.files,
        Some(vec![QuestionFile {
            remote: rpath("/srv/a.txt"),
            unsent: true,
            conflict: false,
        }])
    );
    assert_eq!(question.kept_in, Some(fake.recovered_dir()));

    // Cancel: the tab stays, so does the edit.
    assert!(
        app.update(edit(EditMsg::CloseTab {
            tab: first,
            answer: CloseAnswer::Cancel
        }))
        .is_empty()
    );
    assert!(app.close_asks.is_empty());
    assert_eq!(app.tabs.len(), 1);

    // Asked again, the upload can't finish (the connection is down): the question stays
    // with what is left; nothing closes.
    let effects = app.update(Message::CloseTab(first));
    let [Effect::CheckEdits { purpose, sessions }] = &effects[..] else {
        panic!("{effects:?}");
    };
    app.update(effects::check_edits(backend.clone(), purpose.clone(), sessions.clone()).await);
    fake.drop_connection(session, Error::new(ErrorKind::Network, "Connection lost"));
    let effects = app.update(edit(EditMsg::CloseTab {
        tab: first,
        answer: CloseAnswer::Upload,
    }));
    let [
        Effect::SettleEdits {
            purpose,
            sessions,
            how: Settle::Upload,
        },
    ] = &effects[..]
    else {
        panic!("{effects:?}");
    };
    assert_eq!(app.close_asks[&first].busy, Some(CloseAnswer::Upload));
    let settled = effects::settle_edits(
        backend.clone(),
        purpose.clone(),
        sessions.clone(),
        Settle::Upload,
        Duration::from_millis(200),
    )
    .await;
    let effects = app.update(settled);
    assert!(!effects.iter().any(|e| matches!(e, Effect::CloseSession(_))));
    assert_eq!(app.tabs.len(), 1);
    let question = &app.close_asks[&first];
    assert_eq!(question.busy, None);
    assert_eq!(question.not_uploaded, 1);
    assert_eq!(fake.calls(), vec![Call::FlushEdits(session)]);

    // Back: "Upload pending changes and close" sends it, then the tab and the session go.
    fake.reconnect(session).await.unwrap();
    let effects = app.update(edit(EditMsg::CloseTab {
        tab: first,
        answer: CloseAnswer::Upload,
    }));
    let [
        Effect::SettleEdits {
            purpose,
            sessions,
            how,
        },
    ] = &effects[..]
    else {
        panic!("{effects:?}");
    };
    let settled = effects::settle_edits(
        backend.clone(),
        purpose.clone(),
        sessions.clone(),
        *how,
        FLUSH_LIMIT,
    )
    .await;
    // Uploaded before anything closed.
    assert_eq!(
        fake.remote_fs(&a.id).read_file("/srv/a.txt").unwrap(),
        b"unsent"
    );
    assert!(!fake.calls().contains(&Call::Close(session)));
    let effects = app.update(settled);
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::CloseSession(s) if *s == session)),
        "{effects:?}"
    );
    assert!(app.tabs.is_empty());
    assert!(app.close_asks.is_empty());
    // All sent: the edit directory goes with the edits.
    assert!(!local.exists());
    assert_eq!(effects::close_session(backend.clone(), session).await, None);
}

/// Review Focus 4: closing the window with an unsent edit asks in the center *before*
/// the Plan 5 flow (save, engine, sessions); "Upload and quit" sends it, then that flow
/// runs. Nothing unsent: the usual flow at once. "Cancel" keeps the window.
#[tokio::test]
async fn window_close_with_pending_edits_asks_before_saving() {
    let a = srv_site(Protocol::Ssh);
    let ws = workspace("Shop", &[&a.id]);
    let fake = Arc::new(
        FakeBackend::new()
            .with_site(a.clone())
            .with_workspace(ws.clone()),
    );
    let backend: Arc<dyn BackendApi> = fake.clone();
    let (mut app, session, id) = editing(&fake, &a).await;
    app.open_workspace = Some(ws.id.clone());
    save_unseen(&fake, id, b"unsent");
    let window = iced::window::Id::unique();

    let effects = app.update(Message::CloseRequested(window));
    let [Effect::CheckEdits { purpose, sessions }] = &effects[..] else {
        panic!("no save, no shutdown before the question: {effects:?}");
    };
    assert_eq!(purpose.clone(), EditsFor::Quit(window));
    assert!(!app.closing, "the window stays open");
    let checked = effects::check_edits(backend.clone(), purpose.clone(), sessions.clone()).await;
    assert!(app.update(checked).is_empty());
    assert!(matches!(app.form, Some(Form::QuitEdits)), "in the center");
    let quit = app.center_ask.as_ref().unwrap();
    assert_eq!(
        quit.question.files.as_deref(),
        Some(
            &[QuestionFile {
                remote: rpath("/srv/a.txt"),
                unsent: true,
                conflict: false,
            }][..]
        )
    );
    // Asked again while the question shows: still the question.
    assert!(app.update(Message::CloseRequested(window)).is_empty());
    assert!(!app.closing);

    // "Cancel": the window stays, nothing was saved or closed.
    app.update(edit(EditMsg::Center(CloseAnswer::Cancel)));
    assert!(app.center_ask.is_none());
    assert!(app.form.is_none());
    assert!(!app.closing);
    assert!(fake.calls().is_empty());

    // "Upload and quit": the upload, then the Plan 5 flow.
    let effects = app.update(Message::CloseRequested(window));
    let [Effect::CheckEdits { purpose, sessions }] = &effects[..] else {
        panic!("{effects:?}");
    };
    app.update(effects::check_edits(backend.clone(), purpose.clone(), sessions.clone()).await);
    let effects = app.update(edit(EditMsg::Center(CloseAnswer::Upload)));
    let [
        Effect::SettleEdits {
            purpose,
            sessions,
            how,
        },
    ] = &effects[..]
    else {
        panic!("{effects:?}");
    };
    assert_eq!(*how, Settle::Upload);
    assert!(!app.closing);
    let settled = effects::settle_edits(
        backend.clone(),
        purpose.clone(),
        sessions.clone(),
        *how,
        FLUSH_LIMIT,
    )
    .await;
    // (The tab shows again: its lists' scroll offsets are restored.)
    let effects = without_panes(app.update(settled));
    let [Effect::Shutdown { save, window: w }] = &effects[..] else {
        panic!("{effects:?}");
    };
    assert_eq!(*w, window);
    assert!(app.closing);
    assert!(app.form.is_none());
    // While closing, the edit buttons do nothing.
    assert!(
        app.update(edit(EditMsg::Retry(id))).is_empty(),
        "input is ignored while closing"
    );
    let outcome = effects::shutdown(backend.clone(), save.clone(), effects::SHUTDOWN_LIMIT).await;
    assert_eq!(outcome, effects::ShutdownOutcome::Done);
    assert_eq!(
        fake.calls(),
        vec![
            Call::FlushEdits(session),
            Call::SaveTabs(ws.id.clone()),
            Call::Shutdown,
            Call::Close(session),
        ],
        "the edits are settled before the save"
    );
    assert_eq!(
        fake.remote_fs(&a.id).read_file("/srv/a.txt").unwrap(),
        b"unsent"
    );

    // Active edits all on the server: no question, the usual flow.
    let fake = Arc::new(FakeBackend::new().with_site(a.clone()));
    let backend: Arc<dyn BackendApi> = fake.clone();
    let (mut app, _, _) = editing(&fake, &a).await;
    let effects = app.update(Message::CloseRequested(window));
    let [Effect::CheckEdits { purpose, sessions }] = &effects[..] else {
        panic!("{effects:?}");
    };
    let effects =
        app.update(effects::check_edits(backend, purpose.clone(), sessions.clone()).await);
    assert!(
        matches!(effects[..], [Effect::Shutdown { .. }]),
        "{effects:?}"
    );
    assert!(app.closing);
    assert!(app.form.is_none());
}

/// Review Focus 4: "Quit and keep local copies" says where the copies go, saves the
/// unsent change as a recovered copy and deletes nothing, before or after the shutdown.
#[tokio::test]
async fn keep_local_copies_never_deletes() {
    let a = srv_site(Protocol::Ssh);
    let fake = Arc::new(FakeBackend::new().with_site(a.clone()));
    let backend: Arc<dyn BackendApi> = fake.clone();
    let (mut app, session, id) = editing(&fake, &a).await;
    let local = save_unseen(&fake, id, b"unsent");
    let window = iced::window::Id::unique();

    let effects = app.update(Message::CloseRequested(window));
    let [Effect::CheckEdits { purpose, sessions }] = &effects[..] else {
        panic!("{effects:?}");
    };
    app.update(effects::check_edits(backend.clone(), purpose.clone(), sessions.clone()).await);
    let kept_in = fake.recovered_dir();
    assert_eq!(
        app.center_ask.as_ref().unwrap().question.kept_in.as_ref(),
        Some(&kept_in),
        "the question says where the copies go"
    );

    let effects = app.update(edit(EditMsg::Center(CloseAnswer::Keep)));
    let [
        Effect::SettleEdits {
            purpose,
            sessions,
            how,
        },
    ] = &effects[..]
    else {
        panic!("{effects:?}");
    };
    assert_eq!(*how, Settle::Keep);
    let settled = effects::settle_edits(
        backend.clone(),
        purpose.clone(),
        sessions.clone(),
        *how,
        FLUSH_LIMIT,
    )
    .await;
    let Message::Edit(EditMsg::Settled {
        result: Ok(result), ..
    }) = &settled
    else {
        panic!("{settled:?}");
    };
    assert_eq!(result.kept.len(), 1);
    let copy = result.kept[0].recovered.clone();
    assert!(copy.starts_with(&kept_in));
    let effects = app.update(settled);
    let shutdown = effects
        .iter()
        .find_map(|e| match e {
            Effect::Shutdown { save, .. } => Some(save.clone()),
            _ => None,
        })
        .expect("the close flow goes on");
    assert!(app.closing);
    let notice = app.kept_notice.clone().expect("the kept copies are said");
    assert!(
        !notice.closed_unasked,
        "the user chose: nothing more to say"
    );
    assert_eq!((notice.count, notice.dir), (1, kept_in.clone()));
    assert_eq!(fake.calls(), vec![Call::KeepEdits(session)]);
    assert_eq!(std::fs::read(&local).unwrap(), b"unsent", "nothing deleted");
    assert_eq!(std::fs::read(&copy).unwrap(), b"unsent");
    assert_eq!(
        fake.remote_fs(&a.id).read_file("/srv/a.txt").unwrap(),
        b"v1",
        "nothing uploaded"
    );

    effects::shutdown(backend, shutdown, effects::SHUTDOWN_LIMIT).await;
    assert_eq!(
        std::fs::read(&local).unwrap(),
        b"unsent",
        "the shutdown deletes nothing either"
    );
    assert_eq!(std::fs::read(&copy).unwrap(), b"unsent");
}

/// `editor_command`: `system` by default; a typed command applies and is saved; a blank
/// one is refused inline (the last valid one stays); the file keeps it and the keys this
/// version does not know.
#[tokio::test]
async fn editor_setting_roundtrip() {
    use crate::settings::{self as file, EDITOR_DEFAULT};

    let mut app = App::new();
    app.form = Some(Form::Settings);
    assert_eq!(app.settings.editor_command, "system");
    assert_eq!(EDITOR_DEFAULT, "system");

    let effects = app.update(Message::Settings(settings::SettingsMsg::EditorCommand(
        "code --wait".into(),
    )));
    let [Effect::SaveSettings(saved)] = &effects[..] else {
        panic!("{effects:?}");
    };
    assert_eq!(saved.editor_command, "code --wait");
    app.update(Message::SettingsSaved(Ok(())));

    // Blank: refused inline, nothing saved, the command in force stays.
    let effects = app.update(Message::Settings(settings::SettingsMsg::EditorCommand(
        "   ".into(),
    )));
    assert!(effects.is_empty());
    assert_eq!(app.editor_input.as_deref(), Some("   "));
    assert_eq!(app.settings.editor_command, "code --wait");
    // Leaving the page forgets the blank field.
    app.update(Message::Settings(settings::SettingsMsg::Close));
    assert_eq!(app.editor_input, None);

    // The file: written and read back, unknown keys kept; blank in the file is the default.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(file::FILE_NAME);
    std::fs::write(&path, "future_key = 1\neditor_command = \"  \"\n").unwrap();
    let loaded = file::load(&path);
    assert_eq!(loaded.error, None);
    assert_eq!(loaded.settings.editor_command, "system");
    file::save(&path, saved).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("editor_command = \"code --wait\""), "{text}");
    assert!(text.contains("future_key = 1"), "{text}");
    assert_eq!(file::load(&path).settings, *saved);
    // A file from before the setting: the default.
    assert_eq!(
        file::parse("workers_per_site = 2").unwrap().editor_command,
        "system"
    );
    // A file saved by 1.0 with its default, `xdg-open`, keeps it (still a valid command).
    assert_eq!(
        file::parse("editor_command = \"xdg-open\"")
            .unwrap()
            .editor_command,
        "xdg-open"
    );
}

/// Every file under `dir`, recursively (the recovered copies).
fn files_under(dir: &std::path::Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return found;
    };
    for entry in entries {
        let path = entry.unwrap().path();
        if path.is_dir() {
            found.extend(files_under(&path));
        } else {
            found.push(path);
        }
    }
    found
}

/// Fix round 1 (Important 4, 5): a session closed without the question (here as the
/// site's deletion or a reload would) keeps its unsent edit as a recovered copy and
/// deletes nothing; the notice says how many and the recovered root, stays until
/// dismissed, and an upload's toast does not replace it.
#[tokio::test]
async fn close_session_keeps_unsent_edits_and_says_where() {
    let a = srv_site(Protocol::Ssh);
    let fake = Arc::new(FakeBackend::new().with_site(a.clone()));
    let backend: Arc<dyn BackendApi> = fake.clone();
    let (mut app, session, id) = editing(&fake, &a).await;
    let local = save_unseen(&fake, id, b"unsent");

    let (kept, kept_in) = effects::close_session(backend.clone(), session)
        .await
        .expect("the unsent edit is kept");
    assert_eq!(kept_in, fake.recovered_dir());
    assert_eq!(kept.len(), 1);
    assert_eq!(std::fs::read(&kept[0].recovered).unwrap(), b"unsent");
    assert_eq!(std::fs::read(&local).unwrap(), b"unsent", "nothing deleted");
    assert_eq!(files_under(&kept_in), vec![kept[0].recovered.clone()]);
    // Final review: the edits are settled (an upload under way finishes, the rest is
    // kept) before the session closes, never cut by the close.
    assert_eq!(
        fake.calls(),
        vec![Call::KeepEdits(session), Call::Close(session)]
    );

    app.update(edit(EditMsg::Kept {
        kept: kept.clone(),
        kept_in: kept_in.clone(),
    }));
    let notice = app.kept_notice.clone().unwrap();
    assert_eq!(
        (notice.count, &notice.dir),
        (1, &kept_in),
        "the root, not a file's folder"
    );
    // An upload's toast leaves it alone; a second close adds to it.
    app.update(edit(EditMsg::Event(EditEvent::Uploaded {
        id,
        at: SystemTime::now(),
    })));
    app.update(edit(EditMsg::Kept {
        kept,
        kept_in: kept_in.clone(),
    }));
    assert_eq!(app.kept_notice.as_ref().map(|n| n.count), Some(2));
    assert!(app.toast.is_some());
    app.update(edit(EditMsg::DismissKept));
    assert_eq!(app.kept_notice, None);
}

/// Fix round 1 (Important 3, 5): "Close workspace" with an unsent edit asks in the
/// center first (an open site form steps aside and comes back); "Close and keep local
/// copies" keeps it and only then closes the tabs and sessions; nothing is deleted.
#[tokio::test]
async fn close_workspace_with_edits_asks() {
    let a = srv_site(Protocol::Ssh);
    let ws = workspace("Shop", &[&a.id]);
    let fake = Arc::new(
        FakeBackend::new()
            .with_site(a.clone())
            .with_workspace(ws.clone()),
    );
    let backend: Arc<dyn BackendApi> = fake.clone();
    let (mut app, session, id) = editing(&fake, &a).await;
    app.open_workspace = Some(ws.id.clone());
    let local = save_unseen(&fake, id, b"unsent");
    app.update(Message::EditSite(a.id.clone()));
    assert!(matches!(app.form, Some(Form::Site(_))));

    let effects = app.update(Message::CloseWorkspace(ws.id.clone()));
    let [Effect::CheckEdits { purpose, sessions }] = &effects[..] else {
        panic!("{effects:?}");
    };
    assert_eq!(*purpose, EditsFor::Workspace(ws.id.clone()));
    assert_eq!(app.tabs.len(), 1, "nothing closes yet");
    app.update(effects::check_edits(backend.clone(), purpose.clone(), sessions.clone()).await);
    assert!(matches!(app.form, Some(Form::QuitEdits)));
    assert_eq!(
        app.center_ask.as_ref().unwrap().question.files,
        Some(vec![QuestionFile {
            remote: rpath("/srv/a.txt"),
            unsent: true,
            conflict: false,
        }])
    );
    // Cancel: the site form is back, the workspace stays open.
    app.update(edit(EditMsg::Center(CloseAnswer::Cancel)));
    assert!(matches!(app.form, Some(Form::Site(_))), "{:?}", app.form);
    assert_eq!(app.open_workspace, Some(ws.id.clone()));

    let effects = app.update(Message::CloseWorkspace(ws.id.clone()));
    let [Effect::CheckEdits { purpose, sessions }] = &effects[..] else {
        panic!("{effects:?}");
    };
    app.update(effects::check_edits(backend.clone(), purpose.clone(), sessions.clone()).await);
    let effects = app.update(edit(EditMsg::Center(CloseAnswer::Keep)));
    let [
        Effect::SettleEdits {
            purpose,
            sessions,
            how,
        },
    ] = &effects[..]
    else {
        panic!("{effects:?}");
    };
    assert_eq!(*how, Settle::Keep);
    let settled = effects::settle_edits(
        backend.clone(),
        purpose.clone(),
        sessions.clone(),
        *how,
        FLUSH_LIMIT,
    )
    .await;
    let effects = app.update(settled);
    assert!(effects.iter().any(|e| matches!(e, Effect::SaveTabs { .. })));
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::CloseSession(s) if *s == session))
    );
    assert!(app.tabs.is_empty());
    assert_eq!(app.open_workspace, None);
    assert!(
        matches!(app.form, Some(Form::Site(_))),
        "the form comes back"
    );
    let copies = files_under(&fake.recovered_dir());
    assert_eq!(copies.len(), 1);
    assert_eq!(std::fs::read(&copies[0]).unwrap(), b"unsent");
    assert_eq!(std::fs::read(&local).unwrap(), b"unsent", "nothing deleted");
    assert_eq!(app.kept_notice.as_ref().map(|n| n.count), Some(1));
}

/// Fix round 1 (Important 3, 5): deleting a site whose files are open for editing says
/// so in the confirmation; the deletion's session close keeps the unsent edit as a
/// recovered copy and deletes nothing.
#[tokio::test]
async fn deleting_a_site_with_edits_keeps_them() {
    let a = srv_site(Protocol::Ssh);
    let fake = Arc::new(FakeBackend::new().with_site(a.clone()));
    let backend: Arc<dyn BackendApi> = fake.clone();
    let (mut app, session, id) = editing(&fake, &a).await;
    let local = save_unseen(&fake, id, b"unsent");
    assert_eq!(
        app.site_delete_note(&a.id),
        Some(strings::delete_site_edits(1))
    );
    assert_eq!(app.site_delete_note(&SiteId::new()), None);

    let effects = app.update(Message::FormDone {
        op: ProfileOp::DeleteSite(a.id.clone()),
        result: Ok(()),
    });
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::CloseSession(s) if *s == session)),
        "{effects:?}"
    );
    let (kept, _) = effects::close_session(backend, session).await.unwrap();
    assert_eq!(std::fs::read(&kept[0].recovered).unwrap(), b"unsent");
    assert_eq!(std::fs::read(&local).unwrap(), b"unsent", "nothing deleted");
}

/// Fix round 1 (Important 2, 5): the question bypassed (a second close request while
/// it is checked), the shutdown keeps the unsent edit; a forced exit keeps it too, before
/// any shutdown. Nothing is deleted either way.
#[tokio::test]
async fn bypassed_question_or_forced_exit_still_keeps_unsent_edits() {
    let a = srv_site(Protocol::Ssh);
    let fake = Arc::new(FakeBackend::new().with_site(a.clone()));
    let backend: Arc<dyn BackendApi> = fake.clone();
    let (mut app, _, id) = editing(&fake, &a).await;
    let local = save_unseen(&fake, id, b"unsent");
    let window = iced::window::Id::unique();
    let effects = app.update(Message::CloseRequested(window));
    assert!(matches!(effects[..], [Effect::CheckEdits { .. }]));
    let effects = without_panes(app.update(Message::CloseRequested(window)));
    let [Effect::Shutdown { save, .. }] = &effects[..] else {
        panic!("the user insists: {effects:?}");
    };
    effects::shutdown(backend, save.clone(), effects::SHUTDOWN_LIMIT).await;
    let copies = files_under(&fake.recovered_dir());
    assert_eq!(copies.len(), 1, "{copies:?}");
    assert_eq!(std::fs::read(&copies[0]).unwrap(), b"unsent");
    assert_eq!(std::fs::read(&local).unwrap(), b"unsent");

    // Forced exit: closing already, the window is asked to close again.
    let fake = Arc::new(FakeBackend::new().with_site(a.clone()));
    let backend: Arc<dyn BackendApi> = fake.clone();
    let (mut app, _, id) = editing(&fake, &a).await;
    let local = save_unseen(&fake, id, b"unsent too");
    app.closing = true;
    let effects = app.update(Message::CloseRequested(window));
    assert!(matches!(effects[..], [Effect::ExitWindow(w)] if w == window));
    // What `ExitWindow` runs before closing the window.
    let kept = effects::preserve_before_exit(backend, effects::PRESERVE_LIMIT).await;
    assert_eq!(kept.len(), 1);
    assert_eq!(std::fs::read(&kept[0].recovered).unwrap(), b"unsent too");
    assert_eq!(std::fs::read(&local).unwrap(), b"unsent too");
}

/// Fix round 1 (minors): nothing unsent, closing the last tab asks nothing; while
/// "Upload" runs, "Stop and keep local copies" switches to keeping and the upload's late
/// result is ignored.
#[tokio::test]
async fn tab_question_skips_when_sent_and_stop_keeps() {
    let a = srv_site(Protocol::Ssh);
    let fake = Arc::new(FakeBackend::new().with_site(a.clone()));
    let backend: Arc<dyn BackendApi> = fake.clone();
    let (mut app, session, id) = editing(&fake, &a).await;
    let tab = app.tabs[0].id;

    let effects = app.update(Message::CloseTab(tab));
    let [Effect::CheckEdits { purpose, sessions }] = &effects[..] else {
        panic!("{effects:?}");
    };
    // An unsent save after all: the question.
    let local = save_unseen(&fake, id, b"unsent");
    app.update(effects::check_edits(backend.clone(), purpose.clone(), sessions.clone()).await);
    assert!(app.close_asks[&tab].files.is_some());

    let effects = app.update(edit(EditMsg::CloseTab {
        tab,
        answer: CloseAnswer::Upload,
    }));
    assert!(matches!(
        effects[..],
        [Effect::SettleEdits {
            how: Settle::Upload,
            ..
        }]
    ));
    let effects = app.update(edit(EditMsg::CloseTab {
        tab,
        answer: CloseAnswer::Keep,
    }));
    assert!(
        matches!(
            effects[..],
            [Effect::SettleEdits {
                how: Settle::Keep,
                ..
            }]
        ),
        "stop and keep: {effects:?}"
    );
    assert_eq!(app.close_asks[&tab].busy, Some(CloseAnswer::Keep));
    // The upload's late result changes nothing.
    let stale = app.update(edit(EditMsg::Settled {
        purpose: EditsFor::Tab(tab),
        how: Settle::Upload,
        result: Ok(crate::app::edit::Settled {
            unsent: fake.edits_pending(session).await,
            ..Default::default()
        }),
    }));
    assert!(stale.is_empty());
    assert_eq!(app.tabs.len(), 1);
    let settled = effects::settle_edits(
        backend.clone(),
        EditsFor::Tab(tab),
        vec![session],
        Settle::Keep,
        FLUSH_LIMIT,
    )
    .await;
    let effects = app.update(settled);
    assert!(effects.iter().any(|e| matches!(e, Effect::CloseSession(_))));
    assert!(app.tabs.is_empty());
    assert!(local.exists());

    // Nothing unsent: no question, the tab closes once the backend says so.
    let fake = Arc::new(FakeBackend::new().with_site(a.clone()));
    let backend: Arc<dyn BackendApi> = fake.clone();
    let (mut app, _, _) = editing(&fake, &a).await;
    let tab = app.tabs[0].id;
    let effects = app.update(Message::CloseTab(tab));
    let [Effect::CheckEdits { purpose, sessions }] = &effects[..] else {
        panic!("{effects:?}");
    };
    let effects =
        app.update(effects::check_edits(backend, purpose.clone(), sessions.clone()).await);
    assert!(effects.iter().any(|e| matches!(e, Effect::CloseSession(_))));
    assert!(app.tabs.is_empty());
    assert!(app.close_asks.is_empty());
}

/// Fix round 1 (minor): with several entries selected, Edit refuses with the reason
/// (one file at a time).
#[tokio::test]
async fn edit_needs_one_selected_file() {
    let a = srv_site(Protocol::Ftp);
    let fake = Arc::new(FakeBackend::new().with_site(a.clone()));
    let remote = fake.remote_fs(&a.id);
    remote.add_file("/srv/a.txt", b"a", SystemTime::UNIX_EPOCH);
    remote.add_file("/srv/b.txt", b"b", SystemTime::UNIX_EPOCH);
    let mut app = remote_focused(&fake, &a).await;
    click(&mut app, "a.txt");
    app.update(Message::Modifiers(Mods {
        ctrl: true,
        shift: false,
    }));
    click(&mut app, "b.txt");
    assert_eq!(
        app.edit_target(0, Side::Remote),
        Some(Err(strings::edit_one_file()))
    );
    assert!(start_effects(&app.update(key(FileKey::Edit))).is_empty());
    assert_eq!(
        app.tabs[0].remote().note.as_deref(),
        Some(strings::edit_one_file())
    );
}

/// Fix round 2 (Important 1, 2): a copy kept while its upload was under way is said in
/// the persistent notice ("Server copy of … may be incomplete"); the upload text promises
/// no bound the code does not keep.
#[test]
fn kept_during_an_upload_is_flagged_and_the_text_is_honest() {
    use crate::backend::KeptEdit;
    let mut app = App::new();
    let dir = PathBuf::from("/data/hoprs/recovered");
    app.update(edit(EditMsg::Kept {
        kept: vec![
            KeptEdit {
                id: EditId(1),
                remote: rpath("/srv/a.txt"),
                recovered: dir.join("s/srv/a.txt.local-1"),
                remote_may_be_truncated: false,
            },
            KeptEdit {
                id: EditId(2),
                remote: rpath("/srv/b.txt"),
                recovered: dir.join("s/srv/b.txt.local-1"),
                remote_may_be_truncated: true,
            },
        ],
        kept_in: dir.clone(),
    }));
    let notice = app.kept_notice.clone().unwrap();
    assert_eq!(notice.count, 2);
    assert_eq!(notice.incomplete, vec![rpath("/srv/b.txt")]);
    assert_eq!(
        strings::uploading_edits(),
        "Uploading… (Stop to keep local copies instead)"
    );
}

/// Fix round 2 (minors): the window's question is never replaced by a workspace's (the
/// workspace request is dropped, the window's shows again); a window close during a busy
/// workspace question waits for it and is asked afterwards; forms parked by two
/// questions both come back.
#[tokio::test]
async fn center_questions_never_overwrite_and_forms_stack() {
    let a = srv_site(Protocol::Ssh);
    let ws = workspace("Shop", &[&a.id]);
    let fake = Arc::new(
        FakeBackend::new()
            .with_site(a.clone())
            .with_workspace(ws.clone()),
    );
    let backend: Arc<dyn BackendApi> = fake.clone();
    let (mut app, _, id) = editing(&fake, &a).await;
    app.open_workspace = Some(ws.id.clone());
    save_unseen(&fake, id, b"unsent");
    let window = iced::window::Id::unique();

    // The window's question first; "Close workspace" then changes nothing.
    let effects = app.update(Message::CloseRequested(window));
    let [Effect::CheckEdits { purpose, sessions }] = &effects[..] else {
        panic!("{effects:?}");
    };
    app.update(effects::check_edits(backend.clone(), purpose.clone(), sessions.clone()).await);
    assert!(
        app.update(Message::CloseWorkspace(ws.id.clone()))
            .is_empty()
    );
    assert_eq!(
        app.center_ask.as_ref().map(|ask| ask.purpose.clone()),
        Some(EditsFor::Quit(window))
    );
    assert!(matches!(app.form, Some(Form::QuitEdits)));
    app.update(edit(EditMsg::Center(CloseAnswer::Cancel)));

    // A busy workspace question: the window waits, then asks.
    app.update(Message::EditSite(a.id.clone()));
    let effects = app.update(Message::CloseWorkspace(ws.id.clone()));
    let [Effect::CheckEdits { purpose, sessions }] = &effects[..] else {
        panic!("{effects:?}");
    };
    app.update(effects::check_edits(backend.clone(), purpose.clone(), sessions.clone()).await);
    assert_eq!(app.parked_forms.len(), 1, "the site form is parked");
    let effects = app.update(edit(EditMsg::Center(CloseAnswer::Upload)));
    assert!(matches!(effects[..], [Effect::SettleEdits { .. }]));
    assert!(app.update(Message::CloseRequested(window)).is_empty());
    assert_eq!(app.queued_quit, Some(window));
    assert!(matches!(
        app.center_ask.as_ref().map(|ask| &ask.purpose),
        Some(EditsFor::Workspace(_))
    ));
    // The workspace's upload ends: it closes, its form comes back, then the window asks
    // (nothing unsent any more: the usual close flow after its check).
    let settled = effects::settle_edits(
        backend.clone(),
        EditsFor::Workspace(ws.id.clone()),
        sessions.clone(),
        Settle::Upload,
        FLUSH_LIMIT,
    )
    .await;
    let effects = app.update(settled);
    assert!(app.tabs.is_empty());
    assert_eq!(app.queued_quit, None);
    assert!(matches!(app.form, Some(Form::Site(_))), "{:?}", app.form);
    assert!(app.parked_forms.is_empty());
    // The queued window close runs now: its check finds nothing unsent (the workspace's
    // upload sent it), so the usual close flow follows.
    let check = without_panes(effects)
        .into_iter()
        .find_map(|e| match e {
            Effect::CheckEdits { purpose, sessions } => Some((purpose, sessions)),
            _ => None,
        })
        .expect("the window close that waited runs");
    assert_eq!(check.0, EditsFor::Quit(window));
    let effects =
        without_panes(app.update(effects::check_edits(backend.clone(), check.0, check.1).await));
    assert!(
        matches!(effects[..], [Effect::Shutdown { .. }]),
        "{effects:?}"
    );

    // Two parks: both forms come back, in order.
    let mut app = App::new();
    app.form = Some(Form::Settings);
    app.park_form_for_tests();
    app.form = Some(Form::Import);
    app.park_form_for_tests();
    assert_eq!(app.parked_forms.len(), 2);
    app.unpark_form_for_tests();
    assert!(matches!(app.form, Some(Form::Import)));
    app.form = None;
    app.unpark_form_for_tests();
    assert!(matches!(app.form, Some(Form::Settings)));
}

fn session_state(app: &mut App, session: SessionId, state: hoprs_core::ConnectionState) {
    app.update(Message::SessionState {
        session,
        state,
        auth: None,
    });
}

fn reconnects(effects: &[Effect]) -> Vec<SessionId> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::Reconnect { session } => Some(*session),
            _ => None,
        })
        .collect()
}

fn pending(id: EditId) -> Message {
    edit(EditMsg::Event(EditEvent::Pending {
        id,
        remote_may_be_truncated: false,
    }))
}

/// Final review: a save made in the editor while the session is lost to the network (the
/// transport gave up, say after a laptop sleep) reconnects the session, as a user action
/// on its tab would; once, and never for a session refused for good.
#[tokio::test]
async fn pending_edit_on_lost_session_reconnects() {
    use hoprs_core::ConnectionState;
    let a = srv_site(Protocol::Ssh);
    let fake = Arc::new(FakeBackend::new().with_site(a.clone()));
    let (mut app, session, id) = editing(&fake, &a).await;
    let lost = Error::new(ErrorKind::Network, "Connection lost");
    session_state(&mut app, session, ConnectionState::Disconnected(lost));

    let effects = app.update(pending(id));
    assert_eq!(reconnects(&effects), vec![session], "{effects:?}");
    assert!(matches!(the_edit(&app).status, EditStatus::Waiting { .. }));
    // A reconnection already runs: another save starts none.
    let effects = app.update(pending(id));
    assert!(reconnects(&effects).is_empty(), "{effects:?}");

    // Refused for good: the save waits, nothing reconnects.
    for refused in [
        Error::new(ErrorKind::Auth, "Authentication failed"),
        hoprs_ssh::testing::changed_host_key_error("h", 3),
        Error::new(ErrorKind::Tls, "Certificate not trusted"),
    ] {
        let fake = Arc::new(FakeBackend::new().with_site(a.clone()));
        let (mut app, session, id) = editing(&fake, &a).await;
        session_state(
            &mut app,
            session,
            ConnectionState::Disconnected(refused.clone()),
        );
        let effects = app.update(pending(id));
        assert!(reconnects(&effects).is_empty(), "{refused:?}: {effects:?}");
        assert!(matches!(the_edit(&app).status, EditStatus::Waiting { .. }));
    }

    // Connected: a pending save (the backend is still trying) reconnects nothing.
    let fake = Arc::new(FakeBackend::new().with_site(a.clone()));
    let (mut app, _, id) = editing(&fake, &a).await;
    assert!(reconnects(&app.update(pending(id))).is_empty());
}

/// Final review: a session closed without the question while an edit was unsent (the
/// site's host changed, or "Close and reconnect") keeps it as a recovered copy, and the
/// notice says that further saves in the editor won't be uploaded and where the copy is.
#[tokio::test]
async fn silent_close_kept_notice_says_saves_stop() {
    // The site's endpoint changed under a connected session.
    let a = srv_site(Protocol::Ssh);
    let fake = Arc::new(FakeBackend::new().with_site(a.clone()));
    let backend: Arc<dyn BackendApi> = fake.clone();
    let (mut app, session, id) = editing(&fake, &a).await;
    save_unseen(&fake, id, b"unsent");
    let mut moved = a.clone();
    moved.host = "elsewhere".into();
    let effects = app.update(profiles_loaded(
        &app,
        Ok(ProfileSnapshot {
            sites: vec![moved],
            workspaces: Vec::new(),
            unreadable: Vec::new(),
        }),
    ));
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::CloseSession(s) if *s == session)),
        "{effects:?}"
    );
    let (kept, kept_in) = effects::close_session(backend.clone(), session)
        .await
        .expect("the unsent edit is kept");
    app.update(edit(EditMsg::Kept {
        kept: kept.clone(),
        kept_in: kept_in.clone(),
    }));
    let notice = app.kept_notice.clone().unwrap();
    assert!(notice.closed_unasked);
    assert_eq!(notice.dir, kept_in);
    assert_eq!(std::fs::read(&kept[0].recovered).unwrap(), b"unsent");

    // "Close and reconnect" on a session whose host key changed.
    let fake = Arc::new(FakeBackend::new().with_site(a.clone()));
    let backend: Arc<dyn BackendApi> = fake.clone();
    let (mut app, session, id) = editing(&fake, &a).await;
    save_unseen(&fake, id, b"unsent 2");
    app.update(Message::SessionState {
        session,
        state: hoprs_core::ConnectionState::Disconnected(
            hoprs_ssh::testing::pinned_key_changed_error(&a.host),
        ),
        auth: None,
    });
    let effects = app.update(Message::RestartSession(app.tabs[0].id));
    let Ok([Effect::RestartSite { close, site }]) = <[Effect; 1]>::try_from(effects) else {
        panic!("one RestartSite effect");
    };
    let messages: Vec<Message> = effects::restart_site(backend, close, site).collect().await;
    assert!(
        matches!(&messages[0], Message::Edit(EditMsg::Kept { .. })),
        "kept before the new connection: {messages:?}"
    );
    for message in messages {
        app.update(message);
    }
    let notice = app.kept_notice.clone().expect("the kept copy is said");
    assert!(notice.closed_unasked);
    assert_eq!(notice.count, 1);
    assert_eq!(fake.calls().first(), Some(&Call::KeepEdits(session)));
    assert!(fake.closed().contains(&session));
    let copies = files_under(&notice.dir);
    assert_eq!(copies.len(), 1, "{copies:?}");
    assert_eq!(std::fs::read(&copies[0]).unwrap(), b"unsent 2");
    // The text: saves stop, the copy is where the notice points.
    assert_eq!(
        strings::kept_saves_stopped(),
        "The session closed: further saves in the editor won't be uploaded. Your changes are in the recovered copy; open the file again to go on editing."
    );
}

/// The folder compare settings: the messages update state and each emits the save
/// effect; the typed ignore text is cleaned for the setting and kept as typed in the field.
#[tokio::test]
async fn compare_settings_messages_save() {
    let mut app = App::new();
    assert!(app.settings.compare_auto);

    let effects = app.update(Message::Settings(settings::SettingsMsg::CompareAuto(false)));
    let [Effect::SaveSettings(saved)] = &effects[..] else {
        panic!("{effects:?}");
    };
    assert!(!saved.compare_auto);
    assert!(!app.settings.compare_auto);
    app.update(Message::SettingsSaved(Ok(())));
    // Unchanged: nothing to save.
    assert!(
        app.update(Message::Settings(settings::SettingsMsg::CompareAuto(false)))
            .is_empty()
    );

    app.form = Some(Form::Settings);
    let effects = app.update(Message::Settings(settings::SettingsMsg::CompareIgnore(
        " a, ,*.log ".into(),
    )));
    let [Effect::SaveSettings(saved)] = &effects[..] else {
        panic!("{effects:?}");
    };
    assert_eq!(saved.compare_ignore, vec!["a", "*.log"]);
    assert_eq!(app.compare_ignore_input.as_deref(), Some(" a, ,*.log "));
    app.update(Message::SettingsSaved(Ok(())));

    app.update(Message::Settings(settings::SettingsMsg::Close));
    assert_eq!(app.compare_ignore_input, None);
}
