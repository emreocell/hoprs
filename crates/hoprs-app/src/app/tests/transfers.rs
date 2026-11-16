//! Task 8: transfers from the panes (buttons, Ctrl+Enter, drag, system drop) and the queue
//! panel driven by the engine's events (Review Focus 2: lag and stale events).

use std::path::PathBuf;

use super::*;

use hoprs_core::Error;
use hoprs_transfer::{
    ConflictInfo, ConflictPolicy, Direction, Event, EventKind, ProgressUpdate, Resolution,
    ResolveAction, TransferId, TransferItem, TransferSpec, TransferState,
};

use crate::app::queue::{QueueRow, RowStatus};
use crate::backend::{ControlOp, TransferSnapshot};

fn spec(site: &SiteId, direction: Direction, local: &str, remote: &str) -> TransferSpec {
    TransferSpec {
        site: site.clone(),
        direction,
        local: rpath(local),
        remote: rpath(remote),
    }
}

pub(super) fn item(id: i64, site: &SiteId, state: TransferState) -> TransferItem {
    TransferItem {
        id: TransferId(id),
        parent: None,
        spec: spec(
            site,
            Direction::Upload,
            &format!("/home/me/f{id}"),
            &format!("/srv/f{id}"),
        ),
        is_dir: Some(false),
        size: Some(1000),
        bytes_done: 0,
        state,
    }
}

fn event(seq: u64, kind: EventKind) -> Event {
    Event { seq, kind }
}

fn changed(seq: u64, id: i64, state: TransferState) -> Event {
    event(
        seq,
        EventKind::StateChanged {
            id: TransferId(id),
            state,
        },
    )
}

fn snapshot(seq: u64, items: Vec<TransferItem>) -> TransferSnapshot {
    TransferSnapshot {
        seq,
        items,
        workers: 4,
    }
}

fn failed(message: &str) -> TransferState {
    TransferState::Failed(Error::new(ErrorKind::Permission, message))
}

fn conflict() -> TransferState {
    TransferState::AwaitingDecision(ConflictInfo {
        src_size: 10,
        dst_size: 4,
        src_modified: None,
        dst_modified: None,
    })
}

/// The single snapshot request among `effects`.
fn snapshot_request(effects: &[Effect]) -> RequestId {
    match effects {
        [Effect::SnapshotQueue(request)] => *request,
        other => panic!("expected one SnapshotQueue, got {other:?}"),
    }
}

/// Starts the queue the way the subscription does (a resync), on `items` at `seq`.
pub(super) fn synced(app: &mut App, seq: u64, items: Vec<TransferItem>) {
    let request = snapshot_request(&app.update(Message::QueueResync));
    assert!(
        app.update(Message::QueueSnapshot {
            request,
            snapshot: snapshot(seq, items),
        })
        .is_empty()
    );
    assert!(!app.queue.resyncing());
}

fn state_of(app: &App, id: i64) -> &TransferState {
    &app.queue
        .row(TransferId(id))
        .expect("the row exists")
        .item
        .state
}

fn ids(rows: &[&QueueRow]) -> Vec<i64> {
    rows.iter().map(|r| r.item.id.0).collect()
}

/// The specs of the single `Enqueue` among `effects`.
fn enqueued(effects: &[Effect]) -> Vec<TransferSpec> {
    let specs: Vec<_> = effects
        .iter()
        .filter_map(|e| match e {
            Effect::Enqueue(specs) => Some(specs.clone()),
            _ => None,
        })
        .collect();
    match <[Vec<TransferSpec>; 1]>::try_from(specs) {
        Ok([specs]) => specs,
        Err(all) => panic!("expected one Enqueue, got {all:?}"),
    }
}

fn no_enqueue(effects: &[Effect]) -> bool {
    !effects.iter().any(|e| matches!(e, Effect::Enqueue(_)))
}

/// Runs the queue effects against `fake` as `effects::run` does and returns their
/// messages.
async fn run_queue(fake: &Arc<FakeBackend>, effects: Vec<Effect>) -> Vec<Message> {
    let backend: Arc<dyn BackendApi> = fake.clone();
    let mut messages = Vec::new();
    for effect in effects {
        messages.push(match effect {
            Effect::Enqueue(specs) => {
                Message::Enqueued(effects::enqueue_all(backend.clone(), specs).await)
            }
            Effect::Control { id, op } => Message::ControlDone {
                id,
                result: backend.control(id, op).await,
            },
            Effect::SnapshotQueue(request) => Message::QueueSnapshot {
                request,
                snapshot: backend.transfer_snapshot().await,
            },
            Effect::ClearFinished => Message::Cleared(backend.clear_finished().await),
            other => panic!("not a queue effect: {other:?}"),
        });
    }
    messages
}

/// A connected tab: local `/home/me` holds `a.txt`, `b.txt` and `dir/`; remote `/srv`
/// holds `r.txt` and `sub/`.
async fn transfer_tab() -> (Arc<FakeBackend>, App, Site, TabId) {
    let a = srv_site();
    let fake = Arc::new(FakeBackend::new().with_site(a.clone()));
    let fake = &fake;
    fake.remote_fs(&a.id)
        .add_file("/srv/r.txt", b"r", SystemTime::UNIX_EPOCH);
    fake.remote_fs(&a.id).add_dir("/srv/sub");
    let local = fake.local_fs();
    local.add_file("/home/me/a.txt", b"a", SystemTime::UNIX_EPOCH);
    local.add_file("/home/me/b.txt", b"b", SystemTime::UNIX_EPOCH);
    local.add_dir("/home/me/dir");
    let app = connected_tab(fake, &a).await;
    assert!(app.can_act(0, Side::Remote) && app.can_act(0, Side::Local));
    let tab = app.tabs[0].id;
    (fake.clone(), app, a, tab)
}

fn srv_site() -> Site {
    let mut a = site("a", "h", "u", Protocol::Ftp);
    a.remote_dir = Some("/srv".into());
    a
}

/// The row of the entry called `name` in the first tab's pane.
fn row_of(app: &App, side: Side, name: &str) -> usize {
    let pane = app.tabs[0].pane(side).unwrap();
    usize::from(pane.has_up())
        + pane
            .entries
            .iter()
            .position(|e| e.name == name)
            .unwrap_or_else(|| panic!("no {name}"))
}

/// Upload / Download send the selection to the other pane's folder, folders as they are;
/// Ctrl+Enter does the same from the focused pane. The backend gets each spec.
#[tokio::test]
async fn enqueue_selection_to_other_pane() {
    let (fake, mut app, a, tab) = transfer_tab().await;

    // Nothing selected: the button does nothing.
    assert!(!app.tabs[0].can_transfer(app.tab_conn(0), Side::Local));
    assert!(
        app.update(pane_msg(tab, Side::Local, PaneMsg::Transfer))
            .is_empty()
    );

    // a.txt and dir/ (Ctrl+click), then Upload: into the remote folder.
    let a_row = row_of(&app, Side::Local, "a.txt");
    let dir_row = row_of(&app, Side::Local, "dir");
    app.update(pane_msg(tab, Side::Local, PaneMsg::Click(a_row)));
    app.update(Message::Modifiers(Mods {
        ctrl: true,
        shift: false,
    }));
    app.update(pane_msg(tab, Side::Local, PaneMsg::Click(dir_row)));
    app.update(Message::Modifiers(Mods::default()));
    assert!(app.tabs[0].can_transfer(app.tab_conn(0), Side::Local));
    let effects = app.update(pane_msg(tab, Side::Local, PaneMsg::Transfer));
    let specs = enqueued(&effects);
    assert_eq!(
        specs,
        vec![
            spec(&a.id, Direction::Upload, "/home/me/dir", "/srv/dir"),
            spec(&a.id, Direction::Upload, "/home/me/a.txt", "/srv/a.txt"),
        ]
    );
    for message in run_queue(&fake, effects).await {
        assert!(app.update(message).is_empty());
    }
    assert_eq!(fake.enqueued(), specs);
    assert!(app.queue.error.is_none());

    // The remote pane: r.txt, Ctrl+Enter: a download into the local folder.
    app.update(Message::Key {
        key: FileKey::SwitchPane,
        shift: false,
    });
    let r_row = row_of(&app, Side::Remote, "r.txt");
    app.update(pane_msg(tab, Side::Remote, PaneMsg::Click(r_row)));
    let effects = app.update(Message::Key {
        key: FileKey::Transfer,
        shift: false,
    });
    assert_eq!(
        enqueued(&effects),
        vec![spec(
            &a.id,
            Direction::Download,
            "/home/me/r.txt",
            "/srv/r.txt"
        )]
    );

    // The `..` row alone is no selection.
    app.update(pane_msg(tab, Side::Remote, PaneMsg::Click(0)));
    assert!(
        app.update(pane_msg(tab, Side::Remote, PaneMsg::Transfer))
            .is_empty()
    );

    // A tab whose remote side is not connected sends nothing.
    app.conns.get_mut(&app.tabs[0].site).unwrap().state = TabState::Connecting;
    app.update(pane_msg(tab, Side::Local, PaneMsg::Click(a_row)));
    assert!(!app.tabs[0].can_transfer(app.tab_conn(0), Side::Local));
    assert!(
        app.update(pane_msg(tab, Side::Local, PaneMsg::Transfer))
            .is_empty()
    );
}

/// A file dropped from the system file manager goes to the remote folder of the tab in
/// view; with no connected tab in view the queue says why, inline.
#[tokio::test]
async fn drop_from_os_targets_remote_dir() {
    let fake = Arc::new(FakeBackend::new());

    // No tab yet. The queue opens to show why.
    let mut app = loaded(&fake).await;
    app.update(Message::ToggleQueue);
    assert!(!app.queue_open);
    let effects = app.update(Message::FileDropped(PathBuf::from("/tmp/x/photo.jpg")));
    assert!(effects.is_empty());
    assert_eq!(
        app.queue.error.as_ref().map(|e| e.message.as_str()),
        Some("Open a connected site to drop files into it")
    );
    assert!(app.queue_open);
    app.update(Message::Queue(QueueMsg::DismissError));
    assert!(app.queue.error.is_none());

    let (fake, mut app, a, tab) = transfer_tab().await;
    assert!(app.update(Message::FilesHovered(true)).is_empty());
    assert!(app.os_drag);
    let effects = app.update(Message::FileDropped(PathBuf::from(
        "/home/me/Downloads/photo.jpg",
    )));
    assert!(!app.os_drag);
    assert_eq!(
        enqueued(&effects),
        vec![spec(
            &a.id,
            Direction::Upload,
            "/home/me/Downloads/photo.jpg",
            "/srv/photo.jpg"
        )]
    );

    // The remote pane moved to /srv/sub: drops follow it. A folder goes as it is.
    let effects = app.update(pane_msg(
        tab,
        Side::Remote,
        PaneMsg::Navigate(rpath("/srv/sub")),
    ));
    settle(&mut app, &fake, run_listings(&fake, &effects).await).await;
    let effects = app.update(Message::FileDropped(PathBuf::from("/home/me/dir")));
    assert_eq!(
        enqueued(&effects),
        vec![spec(
            &a.id,
            Direction::Upload,
            "/home/me/dir",
            "/srv/sub/dir"
        )]
    );

    // The remote folder is busy (still listing): a different message than "no site".
    let effects = app.update(pane_msg(tab, Side::Remote, PaneMsg::Retry));
    assert!(!effects.is_empty(), "a listing is in flight");
    let effects = app.update(Message::FileDropped(PathBuf::from("/home/me/a.txt")));
    assert!(no_enqueue(&effects));
    assert_eq!(
        app.queue.error.as_ref().map(|e| e.message.as_str()),
        Some("The remote folder is busy or not listed yet; drop again in a moment")
    );
    app.update(Message::Queue(QueueMsg::DismissError));

    // A form in the center: no tab in view.
    app.update(Message::NewSite);
    let effects = app.update(Message::FileDropped(PathBuf::from("/home/me/a.txt")));
    assert!(no_enqueue(&effects));
    assert!(app.queue.error.is_some());
}

/// A press on a row then the button up over the other pane enqueues the selection there;
/// released over its own pane (a plain click) or outside the panes, nothing.
#[tokio::test]
async fn drag_between_panes_enqueues() {
    let (_fake, mut app, a, tab) = transfer_tab().await;
    let b_row = row_of(&app, Side::Local, "b.txt");

    // A plain click: press and release in the same pane.
    app.update(pane_msg(tab, Side::Local, PaneMsg::PointerIn));
    app.update(pane_msg(tab, Side::Local, PaneMsg::Click(b_row)));
    assert_eq!(app.drag, Some((tab, Side::Local)));
    assert_eq!(app.drop_target(), None);
    assert!(no_enqueue(&app.update(Message::DragEnd)));
    assert_eq!(app.drag, None);

    // Press on b.txt, move to the remote pane (the enter may come before the exit), up.
    app.update(pane_msg(tab, Side::Local, PaneMsg::Click(b_row)));
    app.update(pane_msg(tab, Side::Remote, PaneMsg::PointerIn));
    app.update(pane_msg(tab, Side::Local, PaneMsg::PointerOut));
    assert_eq!(
        app.drop_target(),
        Some((tab, Side::Remote)),
        "the remote pane shows it is the target"
    );
    let effects = app.update(Message::DragEnd);
    assert_eq!(
        enqueued(&effects),
        vec![spec(
            &a.id,
            Direction::Upload,
            "/home/me/b.txt",
            "/srv/b.txt"
        )]
    );
    assert_eq!((app.drag, app.drop_target()), (None, None));
    // A second release (no drag) does nothing.
    assert!(app.update(Message::DragEnd).is_empty());

    // From the remote pane to the local one: a download.
    let r_row = row_of(&app, Side::Remote, "r.txt");
    app.update(pane_msg(tab, Side::Remote, PaneMsg::Click(r_row)));
    app.update(pane_msg(tab, Side::Remote, PaneMsg::PointerOut));
    app.update(pane_msg(tab, Side::Local, PaneMsg::PointerIn));
    assert_eq!(
        enqueued(&app.update(Message::DragEnd)),
        vec![spec(
            &a.id,
            Direction::Download,
            "/home/me/r.txt",
            "/srv/r.txt"
        )]
    );

    // The pointer leaves the window mid-drag (its release may never come back): the drag
    // ends, and a later release elsewhere drops nothing.
    app.update(pane_msg(tab, Side::Local, PaneMsg::Click(b_row)));
    app.update(pane_msg(tab, Side::Remote, PaneMsg::PointerIn));
    assert!(app.update(Message::DragCancel).is_empty());
    assert_eq!((app.drag, app.pointer), (None, None));
    app.update(pane_msg(tab, Side::Remote, PaneMsg::PointerIn));
    assert!(no_enqueue(&app.update(Message::DragEnd)));
    app.update(pane_msg(tab, Side::Remote, PaneMsg::PointerOut));

    // The window events behind it: any button up ends a drag, leaving the window cancels.
    use iced::{Event as IcedEvent, mouse};
    let id = iced::window::Id::unique();
    let status = iced::event::Status::Ignored;
    let map = |e| crate::ui::queue::pointer_message(e, status, id);
    assert!(matches!(
        map(IcedEvent::Mouse(mouse::Event::ButtonReleased(
            mouse::Button::Right
        ))),
        Some(Message::DragEnd)
    ));
    assert!(matches!(
        map(IcedEvent::Mouse(mouse::Event::ButtonReleased(
            mouse::Button::Left
        ))),
        Some(Message::DragEnd)
    ));
    assert!(matches!(
        map(IcedEvent::Mouse(mouse::Event::CursorLeft)),
        Some(Message::DragCancel)
    ));

    // Released outside both panes (over the queue): nothing.
    app.update(pane_msg(tab, Side::Local, PaneMsg::Click(b_row)));
    app.update(pane_msg(tab, Side::Local, PaneMsg::PointerOut));
    assert_eq!(app.pointer, None);
    assert!(no_enqueue(&app.update(Message::DragEnd)));
}

/// Active (running, paused, awaiting a decision), Queued, Failed, Done (done, skipped,
/// cancelled): each row in one tab, the counts in the tab labels.
#[test]
fn queue_tabs_partition_states() {
    let mut app = App::new();
    let s = SiteId::new();
    assert!(app.queue.resyncing(), "waits for its first snapshot");
    synced(
        &mut app,
        9,
        vec![
            item(1, &s, TransferState::Active),
            item(2, &s, TransferState::Queued),
            item(3, &s, TransferState::Paused),
            item(4, &s, conflict()),
            item(5, &s, failed("Permission denied")),
            item(6, &s, TransferState::Done),
            item(7, &s, TransferState::Skipped("exists".into())),
            item(8, &s, TransferState::Cancelled),
        ],
    );
    assert_eq!(app.queue.workers, 4);
    assert_eq!(app.queue.counts(), [3, 1, 1, 3]);
    assert_eq!(ids(&app.queue.rows_in(QueueTab::Active)), [1, 3, 4]);
    assert_eq!(ids(&app.queue.rows_in(QueueTab::Queued)), [2]);
    assert_eq!(ids(&app.queue.rows_in(QueueTab::Failed)), [5]);
    assert_eq!(ids(&app.queue.rows_in(QueueTab::Done)), [6, 7, 8]);
    assert_eq!(strings::queue_tab(strings::queue_done(), 3), "Done 3");

    // Each state's buttons.
    let actions = |id| app.queue.actions(app.queue.row(TransferId(id)).unwrap());
    assert_eq!(actions(1), [RowAction::Pause, RowAction::Cancel]);
    assert_eq!(actions(2), [RowAction::Pause, RowAction::Cancel]);
    assert_eq!(actions(3), [RowAction::Resume, RowAction::Cancel]);
    assert_eq!(actions(8), [RowAction::Retry]);
    assert!(actions(6).is_empty() && actions(7).is_empty());

    // An item moving on changes tab; the tab buttons pick what shows.
    assert!(
        app.update(Message::TransferEvent(changed(
            10,
            2,
            TransferState::Active
        )))
        .is_empty()
    );
    assert_eq!(app.queue.counts(), [4, 0, 1, 3]);
    assert_eq!(app.queue.tab, QueueTab::Active);
    app.queue.offset_y = 60.0;
    assert!(
        app.update(Message::Queue(QueueMsg::SelectTab(QueueTab::Done)))
            .is_empty()
    );
    assert_eq!((app.queue.tab, app.queue.offset_y), (QueueTab::Done, 0.0));

    // Pause on a running row asks the engine; a button the row doesn't have does nothing.
    assert!(matches!(
        &app.update(Message::Queue(QueueMsg::Action(
            TransferId(1),
            RowAction::Pause
        )))[..],
        [Effect::Control {
            id: TransferId(1),
            op: ControlOp::Pause
        }]
    ));
    assert!(
        app.update(Message::Queue(QueueMsg::Action(
            TransferId(6),
            RowAction::Retry
        )))
        .is_empty()
    );
    assert!(
        app.update(Message::Queue(QueueMsg::Action(
            TransferId(99),
            RowAction::Pause
        )))
        .is_empty()
    );
}

/// Review Focus 2. The receiver lags (events lost): the stream says so, the queue asks a
/// new snapshot, buffers what comes meanwhile and, once the snapshot is in, drops every
/// event at or below its `seq`. A stale `StateChanged` (older than the snapshot, or
/// delivered out of order later) never overwrites the newer state; an old snapshot
/// answering late is ignored; a gap in `seq` starts another resync.
#[tokio::test]
async fn lagged_triggers_resnapshot_and_drops_stale() {
    let fake = Arc::new(FakeBackend::new());
    let s = SiteId::new();
    let mut app = App::new();
    let mut feed = Box::pin(effects::transfer_feed(fake.as_ref()));
    let mut next = async || {
        tokio::time::timeout(Duration::from_secs(5), feed.next())
            .await
            .expect("a message")
            .expect("the stream is open")
    };

    // Start: the subscription asks for a snapshot after subscribing.
    let first = next().await;
    assert!(matches!(first, Message::QueueResync));
    let effects = app.update(first);
    let r1 = snapshot_request(&effects);
    for message in run_queue(&fake, effects).await {
        assert!(app.update(message).is_empty());
    }
    assert_eq!(app.queue.len(), 0);
    assert!(!app.queue.resyncing());

    // Live: added, then running.
    fake.send_transfer_event(event(
        1,
        EventKind::Added(item(1, &s, TransferState::Queued)),
    ));
    fake.send_transfer_event(changed(2, 1, TransferState::Active));
    for _ in 0..2 {
        let message = next().await;
        assert!(app.update(message).is_empty());
    }
    assert_eq!(state_of(&app, 1), &TransferState::Active);

    // 20 events while the UI reads none: the 16-deep receiver lags and loses 3..=6.
    // seq 21: paused; seq 22: running again.
    for seq in 3..=22 {
        let e = match seq {
            21 => changed(seq, 1, TransferState::Paused),
            22 => changed(seq, 1, TransferState::Active),
            _ => event(
                seq,
                EventKind::Progress(vec![ProgressUpdate {
                    id: TransferId(1),
                    bytes_done: seq * 10,
                    total: Some(1000),
                    bytes_per_sec: 100,
                }]),
            ),
        };
        fake.send_transfer_event(e);
    }
    // By the time the snapshot is taken the engine went on: 23 (last progress), 24 (done).
    let mut done = item(1, &s, TransferState::Done);
    done.bytes_done = 1000;
    fake.set_queue(24, vec![done]);

    let lagged = next().await;
    assert!(
        matches!(lagged, Message::QueueResync),
        "a lagging receiver asks for a resync, got {lagged:?}"
    );
    let effects = app.update(lagged);
    let r2 = snapshot_request(&effects);
    assert_ne!(r1, r2);
    assert!(app.queue.resyncing());

    // The 16 events left (7..=22) arrive before the snapshot: buffered, not applied.
    for seq in 7..=22 {
        let message = next().await;
        assert!(matches!(&message, Message::TransferEvent(e) if e.seq == seq));
        assert!(app.update(message).is_empty());
    }
    assert_eq!(
        state_of(&app, 1),
        &TransferState::Active,
        "nothing applied while resyncing"
    );

    // The snapshot (seq 24) lands: every buffered event is below it, so dropped; the
    // stale "Active" of seq 22 does not overwrite "Done".
    for message in run_queue(&fake, effects).await {
        assert!(app.update(message).is_empty());
    }
    assert!(!app.queue.resyncing());
    assert_eq!(state_of(&app, 1), &TransferState::Done);
    assert_eq!(app.queue.row(TransferId(1)).unwrap().item.bytes_done, 1000);

    // 23 and 24 reach the UI after the snapshot that holds them: dropped too.
    fake.send_transfer_event(event(
        23,
        EventKind::Progress(vec![ProgressUpdate {
            id: TransferId(1),
            bytes_done: 1000,
            total: Some(1000),
            bytes_per_sec: 100,
        }]),
    ));
    fake.send_transfer_event(changed(24, 1, TransferState::Done));
    for _ in 23..=24 {
        let message = next().await;
        assert!(app.update(message).is_empty());
    }
    assert_eq!(state_of(&app, 1), &TransferState::Done);
    assert_eq!(app.queue.total_speed(), 0);

    // An old snapshot answering late (its request superseded) changes nothing.
    assert!(
        app.update(Message::QueueSnapshot {
            request: r1,
            snapshot: snapshot(5, vec![item(1, &s, TransferState::Queued)]),
        })
        .is_empty()
    );
    assert_eq!(state_of(&app, 1), &TransferState::Done);

    // A stale StateChanged delivered out of order: dropped.
    assert!(
        app.update(Message::TransferEvent(changed(
            15,
            1,
            TransferState::Active
        )))
        .is_empty()
    );
    assert_eq!(state_of(&app, 1), &TransferState::Done);

    // The next event applies; one after a gap (26 missing) resyncs instead.
    assert!(
        app.update(Message::TransferEvent(event(
            25,
            EventKind::Added(item(2, &s, TransferState::Queued))
        )))
        .is_empty()
    );
    assert_eq!(state_of(&app, 2), &TransferState::Queued);
    let effects = app.update(Message::TransferEvent(changed(
        27,
        2,
        TransferState::Active,
    )));
    let r3 = snapshot_request(&effects);
    assert_eq!(state_of(&app, 2), &TransferState::Queued);

    // A snapshot older than what is applied (a stopped engine answers `(0, [])`) is
    // dropped, and not asked again in a loop.
    assert!(
        app.update(Message::QueueSnapshot {
            request: r3,
            snapshot: snapshot(0, Vec::new()),
        })
        .is_empty()
    );
    assert_eq!(state_of(&app, 2), &TransferState::Queued);
    assert!(app.queue.resyncing());
}

/// A conflict shows inline in its row (no modal): Overwrite / Resume / Skip and its own
/// "Apply to all" box, which goes with the answer.
#[tokio::test]
async fn conflict_inline_apply_to_all() {
    let fake = Arc::new(FakeBackend::new());
    let s = SiteId::new();
    let mut app = loaded(&fake).await;
    synced(
        &mut app,
        3,
        vec![item(1, &s, conflict()), item(2, &s, conflict())],
    );
    let row = app.queue.row(TransferId(1)).unwrap();
    assert_eq!(app.queue.tab_of(row), QueueTab::Active);
    assert_eq!(
        app.queue.actions(row),
        [
            RowAction::Resolve(ResolveAction::Overwrite),
            RowAction::Resolve(ResolveAction::Resume),
            RowAction::Resolve(ResolveAction::Skip),
        ]
    );

    // Tick "Apply to all" on row 1, then Overwrite.
    assert!(
        app.update(Message::Queue(QueueMsg::ToggleApplyAll(TransferId(1))))
            .is_empty()
    );
    assert!(app.queue.apply_all(TransferId(1)));
    assert!(
        !app.queue.apply_all(TransferId(2)),
        "each row has its own box"
    );
    let effects = app.update(Message::Queue(QueueMsg::Action(
        TransferId(1),
        RowAction::Resolve(ResolveAction::Overwrite),
    )));
    let overwrite_all = ControlOp::Resolve(Resolution {
        action: ResolveAction::Overwrite,
        apply_to_all: true,
    });
    assert!(matches!(
        &effects[..],
        [Effect::Control { id: TransferId(1), op }] if *op == overwrite_all
    ));
    assert!(app.form.is_none(), "no modal");
    // The label says the answer lasts for the session (the engine makes it its policy).
    assert_eq!(strings::apply_to_all(), "Apply to all (this session)");
    assert_eq!(
        app.queue.session_policy(),
        None,
        "not before the engine accepts it"
    );
    for message in run_queue(&fake, effects).await {
        assert!(app.update(message).is_empty());
    }
    assert_eq!(fake.controls(), vec![(TransferId(1), overwrite_all)]);
    // Accepted: the queue remembers the session's policy and its head says so.
    assert_eq!(app.queue.session_policy(), Some(ConflictPolicy::Overwrite));
    assert_eq!(
        strings::session_policy(ConflictPolicy::Overwrite),
        "Conflicts: overwrite (this session)"
    );

    // A refused "to all" answer leaves the session's policy as it was.
    app.update(Message::Queue(QueueMsg::ToggleApplyAll(TransferId(2))));
    let effects = app.update(Message::Queue(QueueMsg::Action(
        TransferId(2),
        RowAction::Resolve(ResolveAction::Resume),
    )));
    assert_eq!(effects.len(), 1);
    app.update(Message::ControlDone {
        id: TransferId(2),
        result: Err(Error::new(
            ErrorKind::Conflict,
            "Transfer is not awaiting a decision",
        )),
    });
    assert_eq!(app.queue.session_policy(), Some(ConflictPolicy::Overwrite));
    app.update(Message::Queue(QueueMsg::ToggleApplyAll(TransferId(2))));

    // Row 2 alone: Skip, not to all.
    let effects = app.update(Message::Queue(QueueMsg::Action(
        TransferId(2),
        RowAction::Resolve(ResolveAction::Skip),
    )));
    assert!(matches!(
        &effects[..],
        [Effect::Control {
            id: TransferId(2),
            op: ControlOp::Resolve(Resolution {
                action: ResolveAction::Skip,
                apply_to_all: false,
            })
        }]
    ));

    // The engine answers: the rows move on and the box goes with the conflict.
    app.update(Message::TransferEvent(changed(4, 1, TransferState::Queued)));
    app.update(Message::TransferEvent(changed(
        5,
        2,
        TransferState::Skipped("skipped by user".into()),
    )));
    assert!(!app.queue.apply_all(TransferId(1)));
    assert_eq!(app.queue.counts(), [0, 1, 0, 1]);
}

/// A failed row shows its error and Retry / Dismiss. Retry asks the engine (its refusal
/// shows in the row); Dismiss moves the row to Done without touching the engine, and Retry
/// still works from there.
#[tokio::test]
async fn failed_row_retry_skip() {
    let fake = Arc::new(FakeBackend::new());
    let s = SiteId::new();
    let mut app = loaded(&fake).await;
    synced(
        &mut app,
        2,
        vec![
            item(1, &s, failed("Permission denied: /var/www")),
            item(2, &s, failed("Disk full")),
        ],
    );
    assert_eq!(ids(&app.queue.rows_in(QueueTab::Failed)), [1, 2]);
    let row = app.queue.row(TransferId(1)).unwrap();
    assert_eq!(
        app.queue.actions(row),
        [RowAction::Retry, RowAction::Dismiss]
    );

    // Retry: to the engine. Its refusal stays in the row until the row changes.
    let effects = app.update(Message::Queue(QueueMsg::Action(
        TransferId(1),
        RowAction::Retry,
    )));
    assert!(matches!(
        &effects[..],
        [Effect::Control {
            id: TransferId(1),
            op: ControlOp::Retry
        }]
    ));
    for message in run_queue(&fake, effects).await {
        assert!(app.update(message).is_empty());
    }
    assert_eq!(fake.controls(), vec![(TransferId(1), ControlOp::Retry)]);
    app.update(Message::ControlDone {
        id: TransferId(1),
        result: Err(Error::new(ErrorKind::Conflict, "Transfer is not failed")),
    });
    assert_eq!(
        app.queue
            .row_error(TransferId(1))
            .map(|e| e.message.as_str()),
        Some("Transfer is not failed")
    );
    app.update(Message::TransferEvent(changed(3, 1, TransferState::Queued)));
    assert!(app.queue.row_error(TransferId(1)).is_none());
    assert_eq!(
        app.queue.tab_of(app.queue.row(TransferId(1)).unwrap()),
        QueueTab::Queued
    );

    // Dismiss: out of Failed, into Done, only Retry left; the engine is not asked.
    assert!(
        app.update(Message::Queue(QueueMsg::Action(
            TransferId(2),
            RowAction::Dismiss
        )))
        .is_empty()
    );
    assert_eq!(fake.controls().len(), 1);
    assert_eq!(app.queue.counts(), [0, 1, 0, 1]);
    let row = app.queue.row(TransferId(2)).unwrap();
    assert_eq!(app.queue.tab_of(row), QueueTab::Done);
    assert_eq!(app.queue.actions(row), [RowAction::Retry]);
    assert_eq!(app.queue.status(row), RowStatus::Dismissed);
    // A dismissed row keeps its place across a resync.
    let request = snapshot_request(&app.update(Message::QueueResync));
    app.update(Message::QueueSnapshot {
        request,
        snapshot: snapshot(
            3,
            vec![
                item(1, &s, TransferState::Queued),
                item(2, &s, failed("Disk full")),
            ],
        ),
    });
    assert_eq!(app.queue.counts(), [0, 1, 0, 1]);

    // Retry from Done: to the engine, and the row is no longer dismissed.
    assert!(matches!(
        &app.update(Message::Queue(QueueMsg::Action(
            TransferId(2),
            RowAction::Retry
        )))[..],
        [Effect::Control {
            id: TransferId(2),
            op: ControlOp::Retry
        }]
    ));
    let row = app.queue.row(TransferId(2)).unwrap();
    assert_eq!(app.queue.tab_of(row), QueueTab::Failed);
}

/// The status column of a row: a running folder reads "Transferring…" (never "Starting…"
/// for its whole run); a file of unknown size shows its bytes and speed; "Starting…" only
/// before any byte or progress; a known size gets the bar.
#[test]
fn row_status_labels() {
    let mut app = App::new();
    let s = SiteId::new();
    let mut folder = item(1, &s, TransferState::Active);
    folder.is_dir = Some(true);
    folder.size = None;
    let mut unknown = item(2, &s, TransferState::Active);
    unknown.size = None;
    let fresh = item(3, &s, TransferState::Active);
    let mut resumed = item(4, &s, TransferState::Active);
    resumed.bytes_done = 250;
    synced(
        &mut app,
        1,
        vec![
            folder,
            unknown,
            fresh,
            resumed,
            item(5, &s, TransferState::Queued),
            item(6, &s, failed("boom")),
            item(7, &s, TransferState::Skipped("exists".into())),
        ],
    );
    fn status(app: &App, id: i64) -> RowStatus<'_> {
        app.queue.status(app.queue.row(TransferId(id)).unwrap())
    }
    assert_eq!(status(&app, 1), RowStatus::Transferring);
    assert_eq!(status(&app, 2), RowStatus::Starting);
    assert_eq!(status(&app, 3), RowStatus::Starting);
    // Bytes already there (a resumed copy): the bar at once.
    assert_eq!(
        status(&app, 4),
        RowStatus::Progress {
            fraction: 0.25,
            bytes_per_sec: 0
        }
    );
    assert_eq!(status(&app, 5), RowStatus::Waiting);
    assert!(matches!(status(&app, 6), RowStatus::Error(e) if e.message == "boom"));
    assert_eq!(status(&app, 7), RowStatus::Skipped("exists"));

    let update = |id, bytes_done, total, bytes_per_sec| ProgressUpdate {
        id: TransferId(id),
        bytes_done,
        total,
        bytes_per_sec,
    };
    app.update(Message::TransferEvent(event(
        2,
        EventKind::Progress(vec![
            update(1, 0, None, 0),
            update(2, 4096, None, 2048),
            update(3, 0, Some(1000), 0),
        ]),
    )));
    // The folder stays "Transferring…"; the unknown size shows bytes and speed.
    assert_eq!(status(&app, 1), RowStatus::Transferring);
    assert_eq!(
        status(&app, 2),
        RowStatus::Bytes {
            done: 4096,
            bytes_per_sec: 2048
        }
    );
    assert_eq!(strings::bytes_and_speed(4096, 2048), "4.0 KB, 2.0 KB/s");
    // A progress came (even at 0 bytes): the bar, not "Starting…".
    assert_eq!(
        status(&app, 3),
        RowStatus::Progress {
            fraction: 0.0,
            bytes_per_sec: 0
        }
    );
    // A refused control wins over the state.
    app.update(Message::ControlDone {
        id: TransferId(3),
        result: Err(Error::new(ErrorKind::Conflict, "nope")),
    });
    assert!(matches!(status(&app, 3), RowStatus::Error(e) if e.message == "nope"));
}

/// The total is the sum of the running rows' speeds; a row that stops counts no more.
#[test]
fn total_speed_sums_active() {
    let mut app = App::new();
    let s = SiteId::new();
    synced(
        &mut app,
        1,
        vec![
            item(1, &s, TransferState::Active),
            item(2, &s, TransferState::Active),
            item(3, &s, TransferState::Paused),
        ],
    );
    assert_eq!(app.queue.total_speed(), 0);
    let update = |id, bytes_per_sec| ProgressUpdate {
        id: TransferId(id),
        bytes_done: 500,
        total: Some(1000),
        bytes_per_sec,
    };
    app.update(Message::TransferEvent(event(
        2,
        EventKind::Progress(vec![update(1, 1000), update(2, 2500), update(3, 400)]),
    )));
    assert_eq!(app.queue.total_speed(), 3500);
    let row = app.queue.row(TransferId(2)).unwrap();
    assert_eq!((row.bytes_per_sec, row.fraction()), (2500, Some(0.5)));
    assert_eq!(app.queue.row(TransferId(3)).unwrap().bytes_per_sec, 0);
    assert_eq!(strings::speed(app.queue.total_speed()), "3.4 KB/s");
    assert_eq!(
        strings::total_workers(app.queue.workers),
        " total, 4 workers"
    );
    assert_eq!(strings::percent(0.519), "51%");

    // The final progress, then Done: that row stops counting.
    app.update(Message::TransferEvent(event(
        3,
        EventKind::Progress(vec![ProgressUpdate {
            bytes_done: 1000,
            ..update(2, 3000)
        }]),
    )));
    assert_eq!(app.queue.total_speed(), 4000);
    app.update(Message::TransferEvent(changed(4, 2, TransferState::Done)));
    assert_eq!(app.queue.total_speed(), 1000);
    assert_eq!(app.queue.row(TransferId(2)).unwrap().fraction(), Some(1.0));
}

/// "Clear finished" calls the engine, which emits no event: a new snapshot (same `seq`,
/// accepted) makes the finished rows go. A failure shows inline.
#[tokio::test]
async fn clear_finished_removes_rows() {
    let fake = Arc::new(FakeBackend::new());
    let s = SiteId::new();
    let mut app = loaded(&fake).await;
    let items = vec![
        item(1, &s, TransferState::Done),
        item(2, &s, TransferState::Active),
        item(3, &s, failed("boom")),
        item(4, &s, TransferState::Cancelled),
    ];
    fake.set_queue(5, items.clone());
    synced(&mut app, 5, items);
    assert!(app.queue.has_finished());
    assert_eq!(app.queue.len(), 4);

    let effects = app.update(Message::Queue(QueueMsg::ClearFinished));
    assert!(matches!(effects[..], [Effect::ClearFinished]));
    let cleared = run_queue(&fake, effects).await;
    assert!(matches!(cleared[..], [Message::Cleared(Ok(3))]));
    assert_eq!(fake.clears(), 1);
    let mut effects = Vec::new();
    for message in cleared {
        effects.extend(app.update(message));
    }
    snapshot_request(&effects);
    for message in run_queue(&fake, effects).await {
        assert!(app.update(message).is_empty());
    }
    assert_eq!(app.queue.len(), 1);
    assert_eq!(ids(&app.queue.rows_in(QueueTab::Active)), [2]);
    assert!(!app.queue.has_finished());

    // The engine refused: the error shows above the rows, nothing else changes.
    assert!(
        app.update(Message::Cleared(Err(Error::new(
            ErrorKind::Io,
            "Transfer engine stopped"
        ))))
        .is_empty()
    );
    assert_eq!(
        app.queue.error.as_ref().map(|e| e.message.as_str()),
        Some("Transfer engine stopped")
    );
    assert_eq!(app.queue.len(), 1);
}

/// A lost connection pauses the site's transfers: its tab says so until one runs again.
#[test]
fn site_paused_until_a_transfer_runs() {
    let mut app = App::new();
    let s = SiteId::new();
    synced(&mut app, 0, vec![item(1, &s, TransferState::Queued)]);
    app.update(Message::TransferEvent(event(
        1,
        EventKind::SitePaused {
            site: s.clone(),
            error: Error::new(ErrorKind::Network, "Connection reset"),
        },
    )));
    assert!(app.queue.site_paused(&s).is_some());
    app.update(Message::TransferEvent(changed(2, 1, TransferState::Active)));
    assert!(app.queue.site_paused(&s).is_none());

    // Paused again; a resync whose snapshot shows the site running clears it, while a
    // site with nothing running stays paused (the snapshot doesn't list paused sites).
    let other = SiteId::new();
    for (seq, site) in [(3, &s), (4, &other)] {
        app.update(Message::TransferEvent(event(
            seq,
            EventKind::SitePaused {
                site: site.clone(),
                error: Error::new(ErrorKind::Network, "Connection reset"),
            },
        )));
    }
    let request = snapshot_request(&app.update(Message::QueueResync));
    app.update(Message::QueueSnapshot {
        request,
        snapshot: snapshot(
            4,
            vec![
                item(1, &s, TransferState::Active),
                item(2, &other, TransferState::Queued),
            ],
        ),
    });
    assert!(app.queue.site_paused(&s).is_none());
    assert!(app.queue.site_paused(&other).is_some());
}

/// End to end against the Docker SSH server: the real backend and engine, the
/// subscription's stream, the app's queue. A file uploaded from the local pane shows up
/// in the queue, runs and lands on the server; "Clear finished" removes it.
#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d --build --wait"]
async fn upload_through_the_queue_end_to_end() {
    use crate::backend::{
        Backend, BackendConfig, BrokerEvent, PromptAnswer, SessionConfig, broker::PromptKind,
    };

    let dir = tempfile::tempdir().unwrap();
    let config = BackendConfig {
        profile_dir: dir.path().join("profiles"),
        queue_path: None,
        sessions: SessionConfig {
            known_hosts: Some(dir.path().join("known_hosts")),
            ..SessionConfig::default()
        },
        backup_params: hoprs_vault::ExportParams::default(),
        edit_dir: dir.path().join("edit"),
        recovered_dir: dir.path().join("recovered"),
    };
    let backend = Backend::open(config, Arc::new(hoprs_vault::MemoryStore::new())).unwrap();
    let backend: Arc<dyn BackendApi> = Arc::new(backend);
    // Trust the test server's host key.
    let mut prompts = backend.prompt_events().unwrap();
    let answers = tokio::spawn({
        let backend = backend.clone();
        async move {
            while let Some(event) = prompts.recv().await {
                if let BrokerEvent::Request(r) = event {
                    let yes = matches!(r.kind, PromptKind::HostKey { .. });
                    let answer = if yes {
                        PromptAnswer::Decision(true)
                    } else {
                        PromptAnswer::Cancel
                    };
                    backend.answer_prompt(r.id, answer).await;
                }
            }
        }
    });

    let mut site = Site::new("docker ssh", "127.0.0.1", "alice", Protocol::Ssh);
    site.port = hoprs_ssh::testing::SSH_PORT;
    site.ssh.use_agent = false;
    site.ssh.key_file = Some(hoprs_ssh::testing::fixture("id_ed25519"));
    backend.save_site(site.clone()).await.unwrap();
    let connected = backend.connect(site.id.clone()).await.unwrap();
    let work = format!(
        "/home/alice/t-queue-{}",
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    backend
        .mkdir(Some(connected.session), Side::Remote, rpath(&work))
        .await
        .unwrap();
    let local = tempfile::tempdir().unwrap();
    std::fs::write(local.path().join("hello.txt"), b"hello queue").unwrap();
    let local_file = crate::app::files::local_path(&local.path().join("hello.txt")).unwrap();

    // The app follows the engine through the subscription's stream.
    let mut app = App::new();
    let mut feed = Box::pin(effects::transfer_feed(backend.as_ref()));
    let run = async |app: &mut App, effects: Vec<Effect>| {
        let mut messages = Vec::new();
        for effect in effects {
            messages.push(match effect {
                Effect::Enqueue(specs) => {
                    Message::Enqueued(effects::enqueue_all(backend.clone(), specs).await)
                }
                Effect::SnapshotQueue(request) => Message::QueueSnapshot {
                    request,
                    snapshot: backend.transfer_snapshot().await,
                },
                Effect::ClearFinished => Message::Cleared(backend.clear_finished().await),
                other => panic!("unexpected {other:?}"),
            });
        }
        let mut next = Vec::new();
        for message in messages {
            next.extend(app.update(message));
        }
        next
    };
    let first = feed.next().await.unwrap();
    let effects = app.update(first);
    assert!(run(&mut app, effects).await.is_empty());
    assert_eq!(app.queue.workers, 4);

    let effects = vec![Effect::Enqueue(vec![TransferSpec {
        site: site.id.clone(),
        direction: Direction::Upload,
        local: local_file,
        remote: rpath(&format!("{work}/hello.txt")),
    }])];
    assert!(run(&mut app, effects).await.is_empty());
    assert!(app.queue.error.is_none());

    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    while app.queue.rows_in(QueueTab::Done).is_empty() {
        let message = tokio::time::timeout_at(deadline, feed.next())
            .await
            .expect("the upload finishes")
            .unwrap();
        let effects = app.update(message);
        assert!(run(&mut app, effects).await.is_empty());
    }
    let done = app.queue.rows_in(QueueTab::Done);
    assert_eq!(done[0].item.state, TransferState::Done);
    assert_eq!(done[0].item.bytes_done, 11);
    let listed = backend
        .list(Some(connected.session), Side::Remote, rpath(&work))
        .await
        .unwrap();
    assert!(listed.iter().any(|e| e.name == "hello.txt" && e.size == 11));

    let effects = app.update(Message::Queue(QueueMsg::ClearFinished));
    let effects = run(&mut app, effects).await;
    assert!(run(&mut app, effects).await.is_empty());
    assert_eq!(app.queue.len(), 0);

    backend.close(connected.session).await;
    answers.abort();
}

/// A queue database that could not be opened: the backend runs on an in-memory queue,
/// and the queue panel says so (transfers of this run won't survive a restart).
#[tokio::test]
async fn unavailable_queue_is_shown_in_the_panel() {
    let cause = Error::new(hoprs_core::ErrorKind::Io, "Cannot open the queue database")
        .with_detail("file is not a database");
    let fake = Arc::new(FakeBackend::new().with_queue_error(cause.clone()));
    let (mut app, boot) = App::boot();
    assert!(
        boot.iter().any(|e| matches!(e, Effect::LoadQueueStatus)),
        "{boot:?}"
    );
    let backend: Arc<dyn BackendApi> = fake.clone();
    app.update(Message::QueueStatus(backend.queue_error()));
    let error = app.queue.error.as_ref().expect("the panel shows it");
    assert_eq!(error.message, strings::queue_unavailable(&cause.message));
    assert!(
        error.message.contains("won't survive a restart"),
        "{}",
        error.message
    );
    assert_eq!(error.detail, cause.detail);

    // A queue that opened: nothing to say.
    let mut app = App::new();
    let fine: Arc<dyn BackendApi> = Arc::new(FakeBackend::new());
    app.update(Message::QueueStatus(fine.queue_error()));
    assert_eq!(app.queue.error, None);
}

/// A deleted site's transfers can never run again: its unfinished items are cancelled
/// through the usual control path (a folder takes its pending children with it), its
/// finished ones and other sites' are left alone.
#[tokio::test]
async fn deleting_a_site_cancels_its_transfers() {
    let s = srv_site();
    let t = site("t", "k", "u", Protocol::Ssh);
    let fake = Arc::new(FakeBackend::new().with_site(s.clone()).with_site(t.clone()));
    let mut app = loaded(&fake).await;
    let mut folder = item(5, &s.id, TransferState::Queued);
    folder.is_dir = Some(true);
    let mut child = item(6, &s.id, TransferState::Queued);
    child.parent = Some(TransferId(5));
    let mut done_folder = item(7, &s.id, TransferState::Done);
    done_folder.is_dir = Some(true);
    let mut orphan = item(8, &s.id, TransferState::Paused);
    orphan.parent = Some(TransferId(7));
    synced(
        &mut app,
        1,
        vec![
            item(1, &s.id, TransferState::Queued),
            item(2, &s.id, TransferState::Paused),
            item(3, &s.id, TransferState::Done),
            item(4, &t.id, TransferState::Active),
            folder,
            child,
            done_folder,
            orphan,
            item(9, &s.id, conflict()),
            item(10, &s.id, TransferState::Active),
        ],
    );

    // A failed delete cancels nothing.
    let effects = app.update(Message::FormDone {
        op: ProfileOp::DeleteSite(s.id.clone()),
        result: Err(Error::new(ErrorKind::Io, "disk full")),
    });
    assert!(effects.is_empty(), "{effects:?}");

    let effects = app.update(Message::FormDone {
        op: ProfileOp::DeleteSite(s.id.clone()),
        result: Ok(()),
    });
    let cancelled: Vec<i64> = effects
        .iter()
        .filter_map(|e| match e {
            Effect::Control {
                id,
                op: ControlOp::Cancel,
            } => Some(id.0),
            _ => None,
        })
        .collect();
    assert_eq!(cancelled, [1, 2, 5, 8, 9, 10], "{effects:?}");
}

/// Review Focus 3: a transfer started on a session lost for a network reason is queued at
/// once (the engine holds it until the site is back) and the session reconnects.
#[tokio::test]
async fn transfer_on_dropped_session_reconnects() {
    let (fake, mut app, _a, tab) = transfer_tab().await;
    let session = app.tab_conn(0).session().unwrap();
    let lost = Error::new(ErrorKind::Network, "Connection lost");
    fake.drop_connection(session, lost.clone());
    app.update(Message::SessionState {
        session,
        state: hoprs_core::ConnectionState::Disconnected(lost),
        auth: None,
    });
    let a_row = row_of(&app, Side::Local, "a.txt");
    app.update(pane_msg(tab, Side::Local, PaneMsg::Click(a_row)));
    let effects = app.update(pane_msg(tab, Side::Local, PaneMsg::Transfer));
    assert_eq!(enqueued(&effects).len(), 1, "{effects:?}");
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::Reconnect { session: s } if *s == session)),
        "{effects:?}"
    );
    assert!(app.tab_conn(0).reconnecting);
}

/// Task 5 fix round 1: a file operation held for a reconnection is dropped if its tab
/// closed meanwhile (never run behind the user's back).
#[tokio::test]
async fn held_file_op_of_a_closed_tab_is_dropped() {
    // An SSH site with a terminal and a files tab: the terminal keeps the site's
    // connection when the files tab closes.
    let mut a = site("a", "h", "u", Protocol::Ssh);
    a.ssh.open_as = hoprs_core::OpenAs::Both;
    a.remote_dir = Some("/srv".into());
    let fake = Arc::new(FakeBackend::new().with_site(a.clone()));
    fake.remote_fs(&a.id).add_dir("/srv/sub");
    fake.local_fs().add_dir("/home/me/dir");
    let mut app = connected_tab(&fake, &a).await;
    let index = app.tabs.iter().position(|t| t.is_files()).unwrap();
    assert_eq!(app.tabs.len(), 2);
    let tab = app.tabs[index].id;
    assert!(app.can_act(index, Side::Remote));
    let session = app.tab_conn(index).session().unwrap();
    let lost = Error::new(ErrorKind::Network, "Connection lost");
    fake.drop_connection(session, lost.clone());
    app.update(Message::SessionState {
        session,
        state: hoprs_core::ConnectionState::Disconnected(lost),
        auth: None,
    });
    app.update(pane_msg(tab, Side::Remote, PaneMsg::NewFolder));
    let effects = app.update(pane_msg(tab, Side::Remote, PaneMsg::EditCommit));
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::Reconnect { session: s } if *s == session)),
        "{effects:?}"
    );
    assert!(
        !effects.iter().any(|e| matches!(e, Effect::FileOp { .. })),
        "held: {effects:?}"
    );

    app.update(Message::CloseTab(tab));
    assert_eq!(app.tabs.len(), 1, "the terminal stays");
    let effects = app.update(Message::Reconnected {
        session,
        result: Ok(()),
    });
    assert!(
        !effects.iter().any(|e| matches!(e, Effect::FileOp { .. })),
        "{effects:?}"
    );
}
