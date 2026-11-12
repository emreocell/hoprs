//! Task 7: the file actions of a pane (new folder, rename, delete, permissions) and
//! the Task 6 carry-overs they rely on (re-list offset, keys while editing).

use super::*;

use crate::app::actions::{BusyKind, Edit, EditKind, FileAction, PaneAction, Permissions, octal};
use crate::backend::DeleteTarget;
use hoprs_core::EntryKind;
use hoprs_remote::RemoteFs;

/// The file operations among `effects`.
fn file_ops(effects: &[Effect]) -> Vec<(Side, Option<SessionId>, FileAction)> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::FileOp {
                side,
                session,
                action,
                ..
            } => Some((*side, *session, action.clone())),
            _ => None,
        })
        .collect()
}

fn key(key: FileKey) -> Message {
    Message::Key { key, shift: false }
}

fn ctrl(app: &mut App, on: bool) {
    app.update(Message::Modifiers(Mods {
        ctrl: on,
        shift: false,
    }));
}

/// The row of the entry called `name` in a pane.
fn row_of(app: &App, side: Side, name: &str) -> usize {
    let pane = app.tabs[0].pane(side);
    usize::from(pane.has_up())
        + pane
            .entries
            .iter()
            .position(|e| e.name == name)
            .unwrap_or_else(|| panic!("no {name} in {:?}", pane.entries))
}

fn selected_names(app: &App, side: Side) -> Vec<String> {
    app.tabs[0]
        .pane(side)
        .selected_entries()
        .into_iter()
        .map(|e| e.name.clone())
        .collect()
}

/// A connected tab with the remote pane focused (`/srv`).
async fn remote_focused(fake: &Arc<FakeBackend>, a: &Site) -> (App, TabId) {
    let mut app = connected_tab(fake, a).await;
    let tab = app.tabs[0].id;
    app.update(key(FileKey::SwitchPane));
    assert_eq!(app.tabs[0].focus, Side::Remote);
    (app, tab)
}

fn srv_site() -> Site {
    let mut a = site("a", "h", "u", Protocol::Ftp);
    a.remote_dir = Some("/srv".into());
    a
}

/// F7 opens the inline name row; Enter creates the folder on the pane's side; the folder
/// is listed again with the new folder selected. A taken name is refused inline.
#[tokio::test]
async fn mkdir_then_refresh() {
    let a = srv_site();
    let fake = Arc::new(FakeBackend::new().with_site(a.clone()));
    fake.remote_fs(&a.id)
        .add_file("/srv/a.txt", b"a", SystemTime::UNIX_EPOCH);
    fake.local_fs().add_dir("/home/me");
    let (mut app, tab) = remote_focused(&fake, &a).await;

    let effects = app.update(key(FileKey::NewFolder));
    assert!(
        matches!(
            effects[..],
            [Effect::FocusEdit {
                side: Side::Remote,
                select_end: 10,
                ..
            }]
        ),
        "the editor takes the focus with \"New folder\" selected: {effects:?}"
    );
    assert_eq!(
        app.tabs[0].remote.action,
        Some(PaneAction::Edit(Edit {
            kind: EditKind::NewFolder,
            value: "New folder".into(),
            error: None,
        }))
    );

    // A taken name stays in the editor with the reason.
    app.update(pane_msg(
        tab,
        Side::Remote,
        PaneMsg::EditChanged("a.txt".into()),
    ));
    assert!(app.update(key(FileKey::Enter)).is_empty());
    assert!(matches!(
        &app.tabs[0].remote.action,
        Some(PaneAction::Edit(Edit { error: Some(e), .. })) if *e == strings::NAME_TAKEN
    ));
    // Typing clears it; Enter creates the folder on the session.
    app.update(pane_msg(
        tab,
        Side::Remote,
        PaneMsg::EditChanged("build".into()),
    ));
    assert!(matches!(
        &app.tabs[0].remote.action,
        Some(PaneAction::Edit(Edit { error: None, .. }))
    ));
    let create = app.update(key(FileKey::Enter));
    let ops = file_ops(&create);
    assert_eq!(ops.len(), 1);
    assert!(ops[0].1.is_some(), "the remote side works on the session");
    assert_eq!(ops[0].2, FileAction::Mkdir(rpath("/srv/build")));
    assert_eq!(app.tabs[0].remote.action, None);
    assert_eq!(
        app.tabs[0].remote.busy.as_ref().map(|b| b.kind),
        Some(BusyKind::NewFolder)
    );
    // Nothing else starts while it runs.
    assert!(app.update(key(FileKey::NewFolder)).is_empty());
    assert_eq!(app.tabs[0].remote.action, None);

    // Done: the folder is listed again, with the new folder selected.
    let done = run_file_ops(&fake, &create).await;
    assert!(matches!(
        &done[..],
        [Message::FileOpDone { result: Ok(()), .. }]
    ));
    let refresh = app.update(done.into_iter().next().unwrap());
    assert_eq!(listings(&refresh).len(), 1);
    assert_eq!(listings(&refresh)[0].3, "/srv");
    assert_eq!(app.tabs[0].remote.busy, None);
    settle(&mut app, &fake, run_listings(&fake, &refresh).await).await;
    assert_eq!(entry_names(&app, 0, Side::Remote), ["build", "a.txt"]);
    assert_eq!(selected_names(&app, Side::Remote), ["build"]);
    let made = fake
        .remote_fs(&a.id)
        .stat(&rpath("/srv/build"))
        .await
        .unwrap();
    assert_eq!(made.kind, EntryKind::Dir);

    // The local side works too, without a session (toolbar button this time).
    let start = app.update(pane_msg(tab, Side::Local, PaneMsg::NewFolder));
    assert!(matches!(
        start[..],
        [Effect::FocusEdit {
            side: Side::Local,
            ..
        }]
    ));
    assert_eq!(app.tabs[0].focus, Side::Local);
    let create = app.update(pane_msg(tab, Side::Local, PaneMsg::EditCommit));
    assert_eq!(
        file_ops(&create),
        [(
            Side::Local,
            None,
            FileAction::Mkdir(rpath("/home/me/New folder"))
        )]
    );
    settle(&mut app, &fake, run_file_ops(&fake, &create).await).await;
    assert_eq!(entry_names(&app, 0, Side::Local), ["New folder"]);
}

/// F2 edits the name in its row; Escape leaves it as it was, Enter renames.
#[tokio::test]
async fn rename_inline_commits_on_enter_cancels_on_escape() {
    let a = srv_site();
    let fake = Arc::new(FakeBackend::new().with_site(a.clone()));
    let remote = fake.remote_fs(&a.id);
    remote.add_file("/srv/report.pdf", b"r", SystemTime::UNIX_EPOCH);
    remote.add_file("/srv/b.txt", b"b", SystemTime::UNIX_EPOCH);
    fake.local_fs().add_dir("/home/me");
    let (mut app, tab) = remote_focused(&fake, &a).await;

    // Nothing selected: nothing to rename.
    assert!(app.update(key(FileKey::Rename)).is_empty());
    assert_eq!(app.tabs[0].remote.action, None);

    let row = row_of(&app, Side::Remote, "report.pdf");
    app.update(pane_msg(tab, Side::Remote, PaneMsg::Click(row)));
    let start = app.update(key(FileKey::Rename));
    assert!(
        matches!(
            start[..],
            [Effect::FocusEdit {
                side: Side::Remote,
                select_end: 6,
                ..
            }]
        ),
        "the stem is preselected: {start:?}"
    );
    let editing = Some(PaneAction::Edit(Edit {
        kind: EditKind::Rename {
            from: "report.pdf".into(),
        },
        value: "report.pdf".into(),
        error: None,
    }));
    assert_eq!(app.tabs[0].remote.action, editing);

    // Escape: back as it was, nothing sent.
    app.update(pane_msg(
        tab,
        Side::Remote,
        PaneMsg::EditChanged("final.pdf".into()),
    ));
    assert!(app.update(key(FileKey::Escape)).is_empty());
    assert_eq!(app.tabs[0].remote.action, None);
    assert_eq!(entry_names(&app, 0, Side::Remote), ["b.txt", "report.pdf"]);
    assert!(remote.read_file("/srv/report.pdf").is_some());

    // Again: the editor starts from the real name; Enter renames.
    app.update(key(FileKey::Rename));
    assert_eq!(app.tabs[0].remote.action, editing);
    app.update(pane_msg(
        tab,
        Side::Remote,
        PaneMsg::EditChanged("final.pdf".into()),
    ));
    let commit = app.update(key(FileKey::Enter));
    assert_eq!(
        file_ops(&commit)[0].2,
        FileAction::Rename {
            from: rpath("/srv/report.pdf"),
            to: rpath("/srv/final.pdf"),
        }
    );
    settle(&mut app, &fake, run_file_ops(&fake, &commit).await).await;
    assert_eq!(entry_names(&app, 0, Side::Remote), ["b.txt", "final.pdf"]);
    assert_eq!(selected_names(&app, Side::Remote), ["final.pdf"]);
    assert!(remote.read_file("/srv/report.pdf").is_none());
    assert_eq!(
        remote.read_file("/srv/final.pdf").as_deref(),
        Some(&b"r"[..])
    );

    // The editor's Enter (on_submit) commits too; the same name just closes it.
    app.update(key(FileKey::Rename));
    assert!(
        app.update(pane_msg(tab, Side::Remote, PaneMsg::EditCommit))
            .is_empty()
    );
    assert_eq!(app.tabs[0].remote.action, None);
    // A name another entry has is refused inline.
    app.update(key(FileKey::Rename));
    app.update(pane_msg(
        tab,
        Side::Remote,
        PaneMsg::EditChanged("b.txt".into()),
    ));
    assert!(app.update(key(FileKey::Enter)).is_empty());
    assert!(matches!(
        &app.tabs[0].remote.action,
        Some(PaneAction::Edit(Edit { error: Some(e), .. })) if *e == strings::NAME_TAKEN
    ));
}

/// Del asks first, listing the names; only "Delete" deletes. Folders go with everything
/// in them (links are removed, not followed), with a running count.
#[tokio::test]
async fn delete_requires_confirmation() {
    let a = srv_site();
    let fake = Arc::new(FakeBackend::new().with_site(a.clone()));
    let remote = fake.remote_fs(&a.id);
    let t = SystemTime::UNIX_EPOCH;
    remote.add_file("/srv/tree/sub/x.txt", b"x", t);
    remote.add_file("/srv/tree/y.txt", b"y", t);
    remote.add_dir("/srv/empty");
    remote.add_file("/srv/f.txt", b"f", t);
    remote.add_file("/srv/keep.txt", b"k", t);
    remote.add_file("/elsewhere/k.txt", b"k", t);
    remote.add_symlink("/srv/link", "/elsewhere");
    fake.local_fs().add_dir("/home/me");
    let (mut app, tab) = remote_focused(&fake, &a).await;

    // Only ".." selected: nothing to delete.
    app.update(pane_msg(tab, Side::Remote, PaneMsg::Click(0)));
    assert!(app.update(key(FileKey::Delete)).is_empty());
    assert_eq!(app.tabs[0].remote.action, None);

    for (i, name) in ["tree", "empty", "f.txt", "link"].into_iter().enumerate() {
        ctrl(&mut app, i > 0);
        let row = row_of(&app, Side::Remote, name);
        app.update(pane_msg(tab, Side::Remote, PaneMsg::Click(row)));
    }
    ctrl(&mut app, false);
    // The plain click on "tree" replaced the "..": rows are in list order.
    assert_eq!(
        selected_names(&app, Side::Remote),
        ["empty", "tree", "f.txt", "link"]
    );

    // Del only asks, with the names.
    let ask = app.update(key(FileKey::Delete));
    assert!(file_ops(&ask).is_empty(), "nothing is deleted yet");
    let Some(PaneAction::ConfirmDelete(asked)) = &app.tabs[0].remote.action else {
        panic!("{:?}", app.tabs[0].remote.action);
    };
    let asked: Vec<&str> = asked.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(asked, ["empty", "tree", "f.txt", "link"]);
    assert_eq!(strings::delete_question(&asked), "Delete 4 items?");
    // Escape and Keep both leave everything in place.
    assert!(app.update(key(FileKey::Escape)).is_empty());
    assert_eq!(app.tabs[0].remote.action, None);
    app.update(pane_msg(tab, Side::Remote, PaneMsg::Delete));
    assert!(
        app.update(pane_msg(tab, Side::Remote, PaneMsg::CancelAction))
            .is_empty()
    );
    assert_eq!(app.tabs[0].remote.action, None);
    assert!(remote.read_file("/srv/f.txt").is_some());

    // Confirmed: one operation, not the transfer queue.
    app.update(key(FileKey::Delete));
    let go = app.update(pane_msg(tab, Side::Remote, PaneMsg::ConfirmDelete));
    let ops = file_ops(&go);
    assert_eq!(ops.len(), 1);
    let target = |path: &str, kind| DeleteTarget {
        path: rpath(path),
        kind,
    };
    assert_eq!(
        ops[0].2,
        FileAction::Delete(vec![
            target("/srv/empty", EntryKind::Dir),
            target("/srv/tree", EntryKind::Dir),
            target("/srv/f.txt", EntryKind::File),
            target("/srv/link", EntryKind::Symlink),
        ])
    );
    assert!(fake.enqueued().is_empty());
    assert_eq!(
        app.tabs[0].remote.busy.as_ref().map(|b| (b.kind, b.done)),
        Some((BusyKind::Delete, 0))
    );

    // 7 items go (empty/ 1; x.txt, sub/, y.txt, tree/ 4; f.txt; link): the throttled
    // count reports the final 7, then the result comes last.
    let messages = run_file_ops(&fake, &go).await;
    let (last, progress) = messages.split_last().unwrap();
    assert!(matches!(last, Message::FileOpDone { result: Ok(()), .. }));
    assert_eq!(progress.len(), 1, "throttled: {progress:?}");
    for message in progress.iter().cloned() {
        assert!(app.update(message).is_empty());
    }
    assert_eq!(app.tabs[0].remote.busy.as_ref().map(|b| b.done), Some(7));
    assert_eq!(strings::deleting(7), "Deleting… 7 items");
    let refresh = app.update(last.clone());
    assert_eq!(listings(&refresh).len(), 1);
    settle(&mut app, &fake, run_listings(&fake, &refresh).await).await;
    assert_eq!(entry_names(&app, 0, Side::Remote), ["keep.txt"]);
    assert_eq!(
        remote.read_file("/elsewhere/k.txt").as_deref(),
        Some(&b"k"[..])
    );
    assert_eq!(app.tabs[0].remote.busy, None);
    assert_eq!(app.tabs[0].remote.action_error, None);
}

/// The rwx grid and the octal value follow each other; Apply runs chmod with the mode.
#[tokio::test]
async fn chmod_dialog_octal_roundtrip() {
    // The dialog alone.
    let mut d = Permissions::new(vec!["run.sh".into()], 0o100_755);
    assert_eq!(
        (d.mode, d.octal.as_str()),
        (0o755, "755"),
        "file type bits dropped"
    );
    d.toggle(0o020);
    assert_eq!((d.mode, d.octal.as_str()), (0o775, "775"));
    d.set_octal("640".into());
    assert_eq!(d.mode, 0o640);
    assert!(d.has(0o400) && d.has(0o200) && d.has(0o040));
    assert!(!d.has(0o100) && !d.has(0o020) && !d.has(0o004));
    d.set_octal("6x".into());
    assert!(d.invalid);
    assert_eq!((d.mode, d.octal.as_str()), (0o640, "6x"), "kept as typed");
    d.toggle(0o004);
    assert!(!d.invalid);
    assert_eq!((d.mode, d.octal.as_str()), (0o644, "644"));
    d.set_octal("4755".into());
    assert_eq!(d.mode, 0o4755);
    d.toggle(0o002);
    assert_eq!((d.mode, d.octal.as_str()), (0o4757, "4757"));
    assert_eq!(octal(0o4757), "4757");

    // In the app: remote only, from the entry's mode, applied with chmod.
    let a = srv_site();
    let fake = Arc::new(FakeBackend::new().with_site(a.clone()));
    let remote = fake.remote_fs(&a.id);
    remote.add_file("/srv/run.sh", b"#!", SystemTime::UNIX_EPOCH);
    fake.local_fs()
        .add_file("/home/me/l.txt", b"l", SystemTime::UNIX_EPOCH);
    let (mut app, tab) = remote_focused(&fake, &a).await;
    app.update(pane_msg(tab, Side::Local, PaneMsg::Click(1)));
    assert!(
        app.update(pane_msg(tab, Side::Local, PaneMsg::Permissions))
            .is_empty()
    );
    assert_eq!(
        app.tabs[0].local.action, None,
        "no permissions on the local side"
    );

    let row = row_of(&app, Side::Remote, "run.sh");
    app.update(pane_msg(tab, Side::Remote, PaneMsg::Click(row)));
    app.update(pane_msg(tab, Side::Remote, PaneMsg::Permissions));
    let dialog = |app: &App| match &app.tabs[0].remote.action {
        Some(PaneAction::Permissions(d)) => d.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(dialog(&app).mode, 0o644, "from the listed mode");
    assert_eq!(dialog(&app).names, ["run.sh"]);
    app.update(pane_msg(
        tab,
        Side::Remote,
        PaneMsg::PermissionToggled(0o100),
    ));
    assert_eq!(dialog(&app).octal, "744");
    // An invalid value can't be applied.
    app.update(pane_msg(
        tab,
        Side::Remote,
        PaneMsg::PermissionOctal("99".into()),
    ));
    assert!(
        app.update(pane_msg(tab, Side::Remote, PaneMsg::ApplyPermissions))
            .is_empty()
    );
    app.update(pane_msg(
        tab,
        Side::Remote,
        PaneMsg::PermissionOctal("750".into()),
    ));
    assert!(dialog(&app).has(0o010) && !dialog(&app).has(0o020));
    let apply = app.update(pane_msg(tab, Side::Remote, PaneMsg::ApplyPermissions));
    assert_eq!(
        file_ops(&apply)[0].2,
        FileAction::Chmod {
            paths: vec![rpath("/srv/run.sh")],
            mode: 0o750,
        }
    );
    assert_eq!(app.tabs[0].remote.action, None);
    settle(&mut app, &fake, run_file_ops(&fake, &apply).await).await;
    assert_eq!(app.tabs[0].remote.entries[0].mode, Some(0o750));
    assert_eq!(selected_names(&app, Side::Remote), Vec::<String>::new());
}

/// A failed action shows in the pane, above the table, with Dismiss: no form, no modal,
/// the pane keeps working.
#[tokio::test]
async fn action_error_is_inline_not_modal() {
    let a = srv_site();
    let fake = Arc::new(FakeBackend::new().with_site(a.clone()));
    let remote = fake.remote_fs(&a.id);
    remote.add_file("/srv/a.txt", b"a", SystemTime::UNIX_EPOCH);
    fake.local_fs().add_dir("/home/me/x");
    remote.deny_writes_under("/srv");
    let (mut app, tab) = remote_focused(&fake, &a).await;

    app.update(key(FileKey::NewFolder));
    let create = app.update(key(FileKey::Enter));
    let done = run_file_ops(&fake, &create).await;
    let refresh = app.update(done.into_iter().next().unwrap());
    let error = app.tabs[0]
        .remote
        .action_error
        .clone()
        .expect("inline error");
    assert_eq!(error.kind, ErrorKind::Permission);
    assert!(app.form.is_none(), "no form, no modal");
    assert!(matches!(tab_state(&app, &a.id), TabState::Connected(_)));
    assert_eq!(app.tabs[0].remote.busy, None);
    // The folder is listed again: the table stays, under the error.
    settle(&mut app, &fake, run_listings(&fake, &refresh).await).await;
    assert_eq!(app.tabs[0].pane_status(Side::Remote), PaneStatus::Full);
    assert_eq!(entry_names(&app, 0, Side::Remote), ["a.txt"]);
    assert!(
        app.tabs[0].remote.action_error.is_some(),
        "kept until dismissed"
    );
    assert_eq!(
        app.tabs[0].local.action_error, None,
        "the other pane is untouched"
    );

    // The pane keeps working; Dismiss (or Escape) clears the bar.
    app.update(pane_msg(tab, Side::Remote, PaneMsg::Click(1)));
    app.update(key(FileKey::Rename));
    app.update(pane_msg(
        tab,
        Side::Remote,
        PaneMsg::EditChanged("b.txt".into()),
    ));
    let rename = app.update(key(FileKey::Enter));
    assert_eq!(
        app.tabs[0].remote.action_error, None,
        "a new action clears the old error"
    );
    settle(&mut app, &fake, run_file_ops(&fake, &rename).await).await;
    assert_eq!(
        app.tabs[0].remote.action_error.as_ref().map(|e| e.kind),
        Some(ErrorKind::Permission)
    );
    app.update(pane_msg(tab, Side::Remote, PaneMsg::DismissError));
    assert_eq!(app.tabs[0].remote.action_error, None);
    // Escape dismisses as well.
    app.tabs[0].remote.action_error = Some(error);
    app.update(key(FileKey::Escape));
    assert_eq!(app.tabs[0].remote.action_error, None);
}

/// Results that come back late: for a closed tab, for an operation the pane no longer
/// waits for, or after the pane moved to another folder (not listed again there).
#[tokio::test]
async fn late_action_results_are_safe() {
    let a = srv_site();
    let fake = Arc::new(FakeBackend::new().with_site(a.clone()));
    let remote = fake.remote_fs(&a.id);
    remote.add_file("/srv/docs/r.txt", b"r", SystemTime::UNIX_EPOCH);
    fake.local_fs().add_dir("/home/me");
    let (mut app, tab) = remote_focused(&fake, &a).await;

    app.update(key(FileKey::NewFolder));
    let create = app.update(key(FileKey::Enter));
    let Some(Effect::FileOp { op, .. }) =
        create.iter().find(|e| matches!(e, Effect::FileOp { .. }))
    else {
        panic!("{create:?}");
    };
    let op = *op;
    // Some other id: ignored, the pane still waits.
    let stray = Message::FileOpDone {
        tab,
        side: Side::Remote,
        op: RequestId(op.0 + 1000),
        result: Ok(()),
    };
    assert!(app.update(stray).is_empty());
    assert!(app.tabs[0].remote.busy.is_some());
    // Same id for the other pane: ignored too.
    let other = Message::FileOpDone {
        tab,
        side: Side::Local,
        op,
        result: Ok(()),
    };
    assert!(app.update(other).is_empty());

    // The user goes into "docs" while it runs; the result lands there safely.
    let docs = row_of(&app, Side::Remote, "docs");
    let into = app.update(pane_msg(tab, Side::Remote, PaneMsg::Open(docs)));
    settle(&mut app, &fake, run_listings(&fake, &into).await).await;
    let done = run_file_ops(&fake, &create).await;
    let effects = app.update(done.into_iter().next().unwrap());
    assert!(
        listings(&effects).is_empty(),
        "docs is not listed again for /srv"
    );
    assert_eq!(app.tabs[0].remote.busy, None, "the pane can act again");
    assert_eq!(app.tabs[0].remote.select_after, None);
    assert_eq!(entry_names(&app, 0, Side::Remote), ["r.txt"]);

    // A closed tab drops its result.
    app.update(key(FileKey::NewFolder));
    let create = app.update(key(FileKey::Enter));
    app.update(Message::CloseTab(tab));
    let done = run_file_ops(&fake, &create).await;
    assert!(app.update(done.into_iter().next().unwrap()).is_empty());
    let progress = Message::FileOpProgress {
        tab,
        side: Side::Remote,
        op,
        done: 3,
    };
    assert!(app.update(progress).is_empty());
}

/// Task 6 carry-over: listing a folder again starts its list from the top, so the
/// remembered offset matches the new scrollable.
#[tokio::test]
async fn relisting_resets_the_scroll_offset() {
    let a = srv_site();
    let fake = Arc::new(FakeBackend::new().with_site(a.clone()));
    fake.remote_fs(&a.id)
        .add_file("/srv/a.txt", b"a", SystemTime::UNIX_EPOCH);
    fake.local_fs().add_dir("/home/me");
    let (mut app, tab) = remote_focused(&fake, &a).await;
    let scrolled = |app: &mut App| {
        app.update(pane_msg(
            tab,
            Side::Remote,
            PaneMsg::Scrolled {
                offset_y: 300.0,
                viewport_h: 600.0,
            },
        ));
        assert_eq!(app.tabs[0].remote.offset_y, 300.0);
    };
    let to_top = |effects: &[Effect]| {
        effects.iter().any(|e| {
            matches!(e, Effect::ScrollPane { side: Side::Remote, offset_y, .. } if *offset_y == 0.0)
        })
    };

    scrolled(&mut app);
    let retry = app.update(pane_msg(tab, Side::Remote, PaneMsg::Retry));
    assert_eq!(app.tabs[0].remote.offset_y, 0.0);
    assert!(to_top(&retry));
    settle(&mut app, &fake, run_listings(&fake, &retry).await).await;

    // After an action too.
    scrolled(&mut app);
    app.update(key(FileKey::NewFolder));
    let create = app.update(key(FileKey::Enter));
    let done = run_file_ops(&fake, &create).await;
    let refresh = app.update(done.into_iter().next().unwrap());
    assert_eq!(app.tabs[0].remote.offset_y, 0.0);
    assert!(to_top(&refresh));
}

/// Task 6 carry-over: while a name is being edited (or the permissions dialog is open),
/// the pane shortcuts don't fire: the keys belong to the text field. Outside it, every
/// key first checks that no text field (the sidebar filter) has the focus.
#[tokio::test]
async fn shortcuts_are_suppressed_while_renaming() {
    let a = srv_site();
    let fake = Arc::new(FakeBackend::new().with_site(a.clone()));
    let remote = fake.remote_fs(&a.id);
    remote.add_file("/srv/a.txt", b"a", SystemTime::UNIX_EPOCH);
    remote.add_file("/srv/docs/x", b"x", SystemTime::UNIX_EPOCH);
    fake.local_fs().add_dir("/home/me");
    let (mut app, tab) = remote_focused(&fake, &a).await;

    // Outside an editor: the key waits for the focus check.
    assert!(matches!(
        app.update(Message::KeyPressed {
            key: FileKey::Down,
            shift: false
        })[..],
        [Effect::KeyUnlessTyping {
            key: FileKey::Down,
            shift: false
        }]
    ));

    let row = row_of(&app, Side::Remote, "a.txt");
    app.update(pane_msg(tab, Side::Remote, PaneMsg::Click(row)));
    app.update(key(FileKey::Rename));
    let before = app.tabs[0].clone();
    for k in [
        FileKey::Up,
        FileKey::Down,
        FileKey::Home,
        FileKey::End,
        FileKey::Back,
        FileKey::SwitchPane,
        FileKey::NewFolder,
        FileKey::Rename,
        FileKey::Delete,
    ] {
        assert!(
            app.update(Message::KeyPressed {
                key: k,
                shift: false
            })
            .is_empty(),
            "{k:?} pressed"
        );
        assert!(app.update(key(k)).is_empty(), "{k:?}");
        assert_eq!(app.tabs[0], before, "{k:?} changed the pane");
    }
    // Escape goes straight through and cancels.
    assert!(
        app.update(Message::KeyPressed {
            key: FileKey::Escape,
            shift: false
        })
        .is_empty()
    );
    assert_eq!(app.tabs[0].remote.action, None);

    // Same with the permissions dialog open.
    app.update(pane_msg(tab, Side::Remote, PaneMsg::Permissions));
    let before = app.tabs[0].clone();
    assert!(
        app.update(Message::KeyPressed {
            key: FileKey::Delete,
            shift: false
        })
        .is_empty()
    );
    assert!(app.update(key(FileKey::Delete)).is_empty());
    assert!(app.update(key(FileKey::Enter)).is_empty());
    assert_eq!(app.tabs[0], before);

    // A form open in the center takes every key.
    app.update(key(FileKey::Escape));
    app.update(Message::NewSite);
    assert!(
        app.update(Message::KeyPressed {
            key: FileKey::Down,
            shift: false
        })
        .is_empty()
    );
}

/// Permissions never go through a link (chmod would change its target): links are left
/// out of the selection, the button is off with only links, and the dialog starts from a
/// real entry's mode, not the link's 777.
#[tokio::test]
async fn permissions_skip_symlinks() {
    let a = srv_site();
    let fake = Arc::new(FakeBackend::new().with_site(a.clone()));
    let remote = fake.remote_fs(&a.id);
    remote.add_file("/srv/run.sh", b"#!", SystemTime::UNIX_EPOCH);
    remote.add_file("/etc/passwd", b"x", SystemTime::UNIX_EPOCH);
    remote.add_symlink("/srv/a-link", "/etc/passwd");
    fake.local_fs().add_dir("/home/me");
    let (mut app, tab) = remote_focused(&fake, &a).await;

    // Only the link: nothing to change, the button is off.
    let link = row_of(&app, Side::Remote, "a-link");
    app.update(pane_msg(tab, Side::Remote, PaneMsg::Click(link)));
    assert!(app.tabs[0].remote.permission_targets().is_empty());
    assert!(
        app.update(pane_msg(tab, Side::Remote, PaneMsg::Permissions))
            .is_empty()
    );
    assert_eq!(app.tabs[0].remote.action, None);

    // Link first, then a file: the dialog is about the file alone, from its mode.
    ctrl(&mut app, true);
    let file = row_of(&app, Side::Remote, "run.sh");
    app.update(pane_msg(tab, Side::Remote, PaneMsg::Click(file)));
    ctrl(&mut app, false);
    assert_eq!(selected_names(&app, Side::Remote), ["a-link", "run.sh"]);
    app.update(pane_msg(tab, Side::Remote, PaneMsg::Permissions));
    let Some(PaneAction::Permissions(dialog)) = app.tabs[0].remote.action.clone() else {
        panic!("{:?}", app.tabs[0].remote.action);
    };
    assert_eq!(dialog.names, ["run.sh"]);
    assert_eq!(dialog.mode, 0o644, "not the link's 777");
    assert!(!dialog.mixed);
    let apply = app.update(pane_msg(tab, Side::Remote, PaneMsg::ApplyPermissions));
    assert_eq!(
        file_ops(&apply)[0].2,
        FileAction::Chmod {
            paths: vec![rpath("/srv/run.sh")],
            mode: 0o644,
        }
    );
}

/// Entries with different modes: the dialog starts from the first and says so.
#[tokio::test]
async fn permissions_mixed_modes_are_flagged() {
    let a = srv_site();
    let fake = Arc::new(FakeBackend::new().with_site(a.clone()));
    let remote = fake.remote_fs(&a.id);
    remote.add_file("/srv/a.txt", b"a", SystemTime::UNIX_EPOCH);
    remote.add_file("/srv/b.txt", b"b", SystemTime::UNIX_EPOCH);
    remote.chmod(&rpath("/srv/b.txt"), 0o600).await.unwrap();
    fake.local_fs().add_dir("/home/me");
    let (mut app, tab) = remote_focused(&fake, &a).await;
    app.update(pane_msg(tab, Side::Remote, PaneMsg::Click(1)));
    app.update(Message::Key {
        key: FileKey::Down,
        shift: true,
    });
    app.update(pane_msg(tab, Side::Remote, PaneMsg::Permissions));
    let Some(PaneAction::Permissions(dialog)) = &app.tabs[0].remote.action else {
        panic!("{:?}", app.tabs[0].remote.action);
    };
    assert_eq!(dialog.names, ["a.txt", "b.txt"]);
    assert_eq!(dialog.mode, 0o644, "the first entry's");
    assert!(dialog.mixed);
}

/// The listing may be stale: renaming onto a name that appeared since is refused by the
/// backend (inline), and the file there is kept.
#[tokio::test]
async fn rename_never_overwrites_an_unlisted_target() {
    let a = srv_site();
    let fake = Arc::new(FakeBackend::new().with_site(a.clone()));
    let remote = fake.remote_fs(&a.id);
    remote.add_file("/srv/a.txt", b"a", SystemTime::UNIX_EPOCH);
    fake.local_fs().add_dir("/home/me");
    let (mut app, tab) = remote_focused(&fake, &a).await;
    // Someone else creates "b.txt" after the listing.
    remote.add_file("/srv/b.txt", b"theirs", SystemTime::UNIX_EPOCH);
    assert_eq!(entry_names(&app, 0, Side::Remote), ["a.txt"]);

    app.update(pane_msg(tab, Side::Remote, PaneMsg::Click(1)));
    app.update(key(FileKey::Rename));
    app.update(pane_msg(
        tab,
        Side::Remote,
        PaneMsg::EditChanged("b.txt".into()),
    ));
    let commit = app.update(key(FileKey::Enter));
    assert_eq!(
        file_ops(&commit).len(),
        1,
        "the cached listing let it through"
    );
    settle(&mut app, &fake, run_file_ops(&fake, &commit).await).await;
    let error = app.tabs[0]
        .remote
        .action_error
        .clone()
        .expect("refused inline");
    assert_eq!(error.kind, ErrorKind::Conflict);
    assert_eq!(
        remote.read_file("/srv/b.txt").as_deref(),
        Some(&b"theirs"[..])
    );
    assert_eq!(remote.read_file("/srv/a.txt").as_deref(), Some(&b"a"[..]));
    assert_eq!(entry_names(&app, 0, Side::Remote), ["a.txt", "b.txt"]);
}

/// F2 on an entry scrolled out of view brings its row (and the editor) back first.
#[tokio::test]
async fn rename_scrolls_its_row_into_view() {
    let a = srv_site();
    let fake = Arc::new(FakeBackend::new().with_site(a.clone()));
    let remote = fake.remote_fs(&a.id);
    for i in 0..20 {
        remote.add_file(&format!("/srv/f{i:02}"), b"", SystemTime::UNIX_EPOCH);
    }
    fake.local_fs().add_dir("/home/me");
    let (mut app, tab) = remote_focused(&fake, &a).await;
    // Rows: 0 "..", 1..=20 f00..f19. Select f19, then scroll back to the top.
    app.update(pane_msg(tab, Side::Remote, PaneMsg::Click(20)));
    app.update(pane_msg(
        tab,
        Side::Remote,
        PaneMsg::Scrolled {
            offset_y: 0.0,
            viewport_h: 90.0,
        },
    ));
    let start = app.update(key(FileKey::Rename));
    // Row 20 ends at 630 px; a 90 px viewport shows it from 540.
    assert!(
        matches!(
            start[..],
            [
                Effect::ScrollPane { side: Side::Remote, offset_y, .. },
                Effect::FocusEdit { side: Side::Remote, .. },
            ] if offset_y == 540.0
        ),
        "{start:?}"
    );
    assert_eq!(app.tabs[0].remote.offset_y, 540.0);
    // Already in view: no scroll.
    app.update(key(FileKey::Escape));
    let again = app.update(key(FileKey::Rename));
    assert!(matches!(again[..], [Effect::FocusEdit { .. }]), "{again:?}");
}

/// A delete that fails half way says how many items it removed first.
#[tokio::test]
async fn partial_delete_reports_its_count() {
    let a = srv_site();
    let fake = Arc::new(FakeBackend::new().with_site(a.clone()));
    let remote = fake.remote_fs(&a.id);
    let t = SystemTime::UNIX_EPOCH;
    for name in ["a", "b", "c"] {
        remote.add_file(&format!("/srv/tree/{name}"), b"", t);
    }
    remote.add_file("/srv/tree/z/locked", b"", t);
    remote.deny_writes_under("/srv/tree/z");
    fake.local_fs().add_dir("/home/me");
    let (mut app, tab) = remote_focused(&fake, &a).await;
    app.update(pane_msg(tab, Side::Remote, PaneMsg::Click(1)));
    app.update(key(FileKey::Delete));
    let go = app.update(pane_msg(tab, Side::Remote, PaneMsg::ConfirmDelete));
    settle(&mut app, &fake, run_file_ops(&fake, &go).await).await;
    let error = app.tabs[0]
        .remote
        .action_error
        .clone()
        .expect("inline error");
    assert_eq!(error.kind, ErrorKind::Permission);
    assert!(
        error.message.starts_with("Deleted 3 items, then: "),
        "{}",
        error.message
    );
    assert_eq!(strings::deleted_then(1, "x"), "Deleted 1 item, then: x");
    assert_eq!(entry_names(&app, 0, Side::Remote), ["tree"], "what is left");
}

/// A case-only rename onto an exact-case sibling the pane never listed is refused (a
/// case-sensitive server would replace it); without one it goes through.
#[tokio::test]
async fn case_only_rename_never_overwrites() {
    let a = srv_site();
    let fake = Arc::new(FakeBackend::new().with_site(a.clone()));
    let remote = fake.remote_fs(&a.id);
    remote.add_file("/srv/a.txt", b"mine", SystemTime::UNIX_EPOCH);
    remote.add_file("/srv/b.txt", b"b", SystemTime::UNIX_EPOCH);
    fake.local_fs().add_dir("/home/me");
    let (mut app, tab) = remote_focused(&fake, &a).await;
    // "A.txt" appears after the listing.
    remote.add_file("/srv/A.txt", b"theirs", SystemTime::UNIX_EPOCH);

    let rename = |app: &mut App, row: usize, to: &str| {
        app.update(pane_msg(tab, Side::Remote, PaneMsg::Click(row)));
        app.update(key(FileKey::Rename));
        app.update(pane_msg(tab, Side::Remote, PaneMsg::EditChanged(to.into())));
        app.update(key(FileKey::Enter))
    };
    let row = row_of(&app, Side::Remote, "a.txt");
    let commit = rename(&mut app, row, "A.txt");
    assert_eq!(
        file_ops(&commit).len(),
        1,
        "the cached listing let it through"
    );
    settle(&mut app, &fake, run_file_ops(&fake, &commit).await).await;
    let error = app.tabs[0]
        .remote
        .action_error
        .clone()
        .expect("refused inline");
    assert_eq!(error.kind, ErrorKind::Conflict);
    assert_eq!(
        remote.read_file("/srv/A.txt").as_deref(),
        Some(&b"theirs"[..])
    );
    assert_eq!(
        remote.read_file("/srv/a.txt").as_deref(),
        Some(&b"mine"[..])
    );

    // No exact-case sibling: "b.txt" → "B.txt" goes through.
    let row = row_of(&app, Side::Remote, "b.txt");
    let commit = rename(&mut app, row, "B.txt");
    settle(&mut app, &fake, run_file_ops(&fake, &commit).await).await;
    assert_eq!(app.tabs[0].remote.action_error, None);
    assert_eq!(remote.read_file("/srv/B.txt").as_deref(), Some(&b"b"[..]));
    assert!(remote.read_file("/srv/b.txt").is_none());
    assert_eq!(selected_names(&app, Side::Remote), ["B.txt"]);
}
