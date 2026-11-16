//! Plan 13 Task 4: the context menu's model. It opens where the pointer last pressed,
//! closes on Escape, a click outside, a resize or a choice, never acts through a missing
//! or disabled item, and never opens over a security dialog.

use iced::{Point, Size};

use super::*;
use crate::app::menu::{MenuMsg, MenuTarget};
use crate::backend::broker::{PromptKind, PromptRequest};
use crate::backend::{BrokerEvent, PromptId};

fn open() -> Message {
    Message::Menu(MenuMsg::Open(MenuTarget::SidebarOptions))
}

fn host_key_prompt(id: u64) -> Message {
    Message::Prompt(BrokerEvent::Request(PromptRequest {
        id: PromptId(id),
        session: SessionId(1),
        site: SiteId::new(),
        key: "h:22".into(),
        kind: PromptKind::HostKey {
            host: "h".into(),
            port: 22,
            key_type: "ssh-ed25519".into(),
            fingerprint: "SHA256:new".into(),
            other_key_types: Vec::new(),
        },
    }))
}

#[test]
fn menu_opens_at_the_last_press() {
    let mut app = App::new();
    assert!(app.menu.is_none());
    assert!(
        app.update(Message::PointerPressed(Point::new(300.0, 40.0)))
            .is_empty()
    );
    assert_eq!(app.last_press, Point::new(300.0, 40.0));
    assert!(app.update(open()).is_empty());
    let menu = app.menu.as_ref().expect("the menu is open");
    assert_eq!(menu.at, Point::new(300.0, 40.0));
    assert_eq!(menu.target, MenuTarget::SidebarOptions);
    assert_eq!(menu.cursor, None);
}

#[test]
fn escape_and_outside_click_close() {
    let mut app = App::new();
    app.update(open());
    assert!(app.menu.is_some());
    // Escape closes the menu, and nothing else sees it.
    assert!(
        app.update(Message::KeyPressed {
            key: FileKey::Escape,
            shift: false,
        })
        .is_empty()
    );
    assert!(app.menu.is_none());
    // A click outside (the overlay's backdrop) closes it too.
    app.update(open());
    assert!(app.menu.is_some());
    assert!(app.update(Message::Menu(MenuMsg::Close)).is_empty());
    assert!(app.menu.is_none());
}

#[test]
fn resize_closes_the_menu() {
    let mut app = App::new();
    assert_eq!(app.window_height, super::INITIAL_WINDOW_HEIGHT);
    app.update(open());
    assert!(
        app.update(Message::WindowResized(Size::new(1000.0, 700.0)))
            .is_empty()
    );
    assert!(app.menu.is_none());
    assert_eq!(app.window_width, 1000.0);
    assert_eq!(app.window_height, 700.0);
}

#[test]
fn choose_disabled_or_missing_item_does_nothing() {
    let mut app = App::new();
    app.update(open());
    assert!(app.update(Message::Menu(MenuMsg::Choose(99))).is_empty());
    assert!(app.menu.is_none());
}

#[test]
fn menu_blocked_behind_dialog() {
    let mut app = App::new();
    app.update(host_key_prompt(1));
    assert!(app.dialogs.is_open());
    // Neither the press nor the menu gets through.
    app.update(Message::PointerPressed(Point::new(50.0, 60.0)));
    assert_eq!(app.last_press, Point::ORIGIN);
    assert!(app.update(open()).is_empty());
    assert!(app.menu.is_none());
}

#[test]
fn a_dialog_closes_an_open_menu() {
    let mut app = App::new();
    app.update(open());
    assert!(app.menu.is_some());
    app.update(host_key_prompt(1));
    assert!(app.dialogs.is_open());
    assert!(app.menu.is_none());
}

#[test]
fn menu_ignored_while_closing() {
    let mut app = App::new();
    app.closing = true;
    assert!(app.update(open()).is_empty());
    assert!(app.menu.is_none());
    app.update(Message::PointerPressed(Point::new(5.0, 5.0)));
    assert_eq!(app.last_press, Point::ORIGIN);
}

// ---- Plan 13 Task 13: the pane menus ----

use crate::app::actions::{Busy, BusyKind, PaneAction};
use crate::app::files::local_fs_path;
use crate::app::menu::{MenuAction, MenuItem};
use crate::backend::ConnectError;

fn right_click(tab: TabId, side: Side, row: Option<usize>) -> Message {
    pane_msg(tab, side, PaneMsg::ContextMenu(row))
}

fn pane_target(tab: TabId, side: Side, row: Option<usize>) -> MenuTarget {
    MenuTarget::Pane { tab, side, row }
}

fn selected(app: &App, side: Side) -> Vec<usize> {
    app.tabs[0]
        .pane(side)
        .unwrap()
        .selection
        .rows
        .iter()
        .copied()
        .collect()
}

/// The open menu's items.
fn open_items(app: &App) -> Vec<MenuItem> {
    app.menu_items(&app.menu.as_ref().expect("a menu is open").target)
}

fn actions(items: &[MenuItem]) -> Vec<MenuAction> {
    items.iter().map(|item| item.action).collect()
}

fn enabled(items: &[MenuItem], action: MenuAction) -> bool {
    items
        .iter()
        .find(|item| item.action == action)
        .unwrap_or_else(|| panic!("no {action:?}"))
        .enabled
}

/// The index of `action` in the open menu.
fn index_of(app: &App, action: MenuAction) -> usize {
    open_items(app)
        .iter()
        .position(|item| item.action == action)
        .unwrap_or_else(|| panic!("no {action:?}"))
}

fn key_pressed(key: FileKey) -> Message {
    Message::KeyPressed { key, shift: false }
}

#[tokio::test]
async fn right_click_unselected_row_selects_it_alone() {
    let (_fake, mut app, _a, tab) = transfer_tab().await;
    let a = row_of(&app, Side::Local, "a.txt");
    let b = row_of(&app, Side::Local, "b.txt");
    app.update(pane_msg(tab, Side::Local, PaneMsg::Click(a)));
    app.tabs[0].set_focus(Side::Remote);
    app.update(Message::PointerPressed(Point::new(120.0, 300.0)));
    assert!(
        app.update(right_click(tab, Side::Local, Some(b)))
            .is_empty()
    );
    assert_eq!(selected(&app, Side::Local), [b]);
    assert_eq!(
        app.tabs[0].focus(),
        Some(Side::Local),
        "the pane gets the focus"
    );
    let menu = app.menu.as_ref().expect("the menu is open");
    assert_eq!(menu.at, Point::new(120.0, 300.0));
    assert_eq!(menu.target, pane_target(tab, Side::Local, Some(b)));
    // The pane's own right press, published after the row's, keeps the row's menu.
    app.update(right_click(tab, Side::Local, None));
    assert_eq!(
        app.menu.as_ref().unwrap().target,
        pane_target(tab, Side::Local, Some(b))
    );
}

#[tokio::test]
async fn right_click_selected_row_keeps_selection() {
    let (_fake, mut app, _a, tab) = transfer_tab().await;
    let a = row_of(&app, Side::Local, "a.txt");
    let b = row_of(&app, Side::Local, "b.txt");
    app.update(pane_msg(tab, Side::Local, PaneMsg::Click(a)));
    app.update(Message::Modifiers(Mods {
        ctrl: true,
        shift: false,
    }));
    app.update(pane_msg(tab, Side::Local, PaneMsg::Click(b)));
    app.update(Message::Modifiers(Mods::default()));
    app.update(right_click(tab, Side::Local, Some(a)));
    assert_eq!(selected(&app, Side::Local), [a, b]);
    // The menu acts on both: Delete asks about two items.
    let delete = index_of(&app, MenuAction::Delete);
    app.update(Message::Menu(MenuMsg::Choose(delete)));
    assert!(app.menu.is_none());
    let Some(PaneAction::ConfirmDelete(targets)) = &app.tabs[0].local().action else {
        panic!("{:?}", app.tabs[0].local().action);
    };
    assert_eq!(targets.len(), 2);
}

#[tokio::test]
async fn local_and_remote_items_and_enabled_state() {
    let (_fake, mut app, a, tab) = transfer_tab().await;
    // A local file.
    let file = row_of(&app, Side::Local, "a.txt");
    app.update(right_click(tab, Side::Local, Some(file)));
    let items = open_items(&app);
    assert_eq!(
        actions(&items),
        [
            MenuAction::Transfer(Side::Local),
            MenuAction::Rename,
            MenuAction::CopyPath,
            MenuAction::Refresh,
            MenuAction::NewFolder,
            MenuAction::Delete,
        ]
    );
    assert!(items.iter().all(|item| item.enabled), "{items:?}");
    let delete = items.last().unwrap();
    assert!(delete.destructive && delete.rule_before);
    assert_eq!(items.iter().filter(|item| item.rule_before).count(), 1);
    assert!(items.iter().all(|item| item.checked.is_none()));
    // A local folder: Open too.
    let dir = row_of(&app, Side::Local, "dir");
    app.update(right_click(tab, Side::Local, Some(dir)));
    assert_eq!(
        actions(&open_items(&app))[..2],
        [MenuAction::Transfer(Side::Local), MenuAction::Open]
    );

    // A remote file: Download, Edit, Rename, Permissions…
    let file = row_of(&app, Side::Remote, "r.txt");
    app.update(right_click(tab, Side::Remote, Some(file)));
    assert_eq!(
        actions(&open_items(&app)),
        [
            MenuAction::Transfer(Side::Remote),
            MenuAction::Edit,
            MenuAction::Rename,
            MenuAction::Permissions,
            MenuAction::CopyPath,
            MenuAction::Refresh,
            MenuAction::NewFolder,
            MenuAction::Delete,
        ]
    );
    assert!(open_items(&app).iter().all(|item| item.enabled));
    // A remote folder: Open, no Edit.
    let sub = row_of(&app, Side::Remote, "sub");
    app.update(right_click(tab, Side::Remote, Some(sub)));
    let items = open_items(&app);
    assert_eq!(
        actions(&items),
        [
            MenuAction::Transfer(Side::Remote),
            MenuAction::Open,
            MenuAction::Rename,
            MenuAction::Permissions,
            MenuAction::CopyPath,
            MenuAction::Refresh,
            MenuAction::NewFolder,
            MenuAction::Delete,
        ]
    );
    assert!(enabled(&items, MenuAction::Open));

    // Not connected: all but Copy path and Refresh are off (shown, not hidden).
    app.conn_mut(&a.id).state = TabState::Failed(ConnectError {
        session: None,
        error: Error::new(ErrorKind::Network, "down"),
        keyring_unavailable: false,
    });
    let items = open_items(&app);
    assert_eq!(items.len(), 8);
    for item in &items {
        let on = matches!(item.action, MenuAction::CopyPath | MenuAction::Refresh);
        assert_eq!(item.enabled, on, "{item:?}");
    }

    // A pane busy with an operation: every action is off.
    app.update(right_click(tab, Side::Local, Some(file - 1)));
    app.tabs[0].local_mut().busy = Some(Busy {
        op: RequestId(999),
        kind: BusyKind::Delete,
        done: 0,
        folder: rpath("/home/me"),
    });
    let items = open_items(&app);
    assert!(!items.is_empty());
    assert!(items.iter().all(|item| !item.enabled), "{items:?}");
}

#[tokio::test]
async fn empty_area_menu() {
    let (_fake, mut app, _a, tab) = transfer_tab().await;
    let a = row_of(&app, Side::Local, "a.txt");
    app.update(pane_msg(tab, Side::Local, PaneMsg::Click(a)));
    app.update(right_click(tab, Side::Local, None));
    // The selection stays as it was.
    assert_eq!(selected(&app, Side::Local), [a]);
    let items = open_items(&app);
    assert_eq!(
        actions(&items),
        [
            MenuAction::NewFolder,
            MenuAction::Refresh,
            MenuAction::Compare
        ]
    );
    assert!(items.iter().all(|item| item.enabled && !item.destructive));
    assert!(items.iter().all(|item| !item.rule_before));
}

/// Each item is the message its toolbar button or key sends. (`App` is not `Clone`: the
/// chosen message itself is compared, then run.)
#[tokio::test]
async fn choosing_sends_the_toolbar_message() {
    let (_fake, mut app, _a, tab) = transfer_tab().await;
    let pane = |side, msg| format!("{:?}", pane_msg(tab, side, msg));
    let cases = [
        (
            Side::Local,
            "a.txt",
            MenuAction::Transfer(Side::Local),
            pane(Side::Local, PaneMsg::Transfer),
        ),
        (
            Side::Local,
            "a.txt",
            MenuAction::Rename,
            pane(Side::Local, PaneMsg::Rename),
        ),
        (
            Side::Local,
            "a.txt",
            MenuAction::NewFolder,
            pane(Side::Local, PaneMsg::NewFolder),
        ),
        (
            Side::Local,
            "a.txt",
            MenuAction::Delete,
            pane(Side::Local, PaneMsg::Delete),
        ),
        (
            Side::Local,
            "a.txt",
            MenuAction::Refresh,
            pane(Side::Local, PaneMsg::Refresh),
        ),
        (
            Side::Local,
            "a.txt",
            MenuAction::CopyPath,
            pane(Side::Local, PaneMsg::CopyPath),
        ),
        (
            Side::Remote,
            "r.txt",
            MenuAction::Transfer(Side::Remote),
            pane(Side::Remote, PaneMsg::Transfer),
        ),
        (
            Side::Remote,
            "r.txt",
            MenuAction::Edit,
            pane(Side::Remote, PaneMsg::Edit),
        ),
        (
            Side::Remote,
            "r.txt",
            MenuAction::Permissions,
            pane(Side::Remote, PaneMsg::Permissions),
        ),
    ];
    for (side, name, action, expected) in cases {
        let row = row_of(&app, side, name);
        app.update(right_click(tab, side, Some(row)));
        let index = index_of(&app, action);
        let chosen = app.menu_command(Message::Menu(MenuMsg::Choose(index)));
        assert_eq!(format!("{chosen:?}"), expected, "{action:?}");
        assert!(app.menu.is_none());
    }
    // Open names the row it was asked on.
    let sub = row_of(&app, Side::Remote, "sub");
    app.update(right_click(tab, Side::Remote, Some(sub)));
    let index = index_of(&app, MenuAction::Open);
    let chosen = app.menu_command(Message::Menu(MenuMsg::Choose(index)));
    assert_eq!(
        format!("{chosen:?}"),
        pane(Side::Remote, PaneMsg::Open(sub))
    );
    // Compare is the toolbar's button.
    app.update(right_click(tab, Side::Remote, None));
    let index = index_of(&app, MenuAction::Compare);
    let chosen = app.menu_command(Message::Menu(MenuMsg::Choose(index)));
    assert_eq!(
        format!("{chosen:?}"),
        format!("{:?}", Message::Compare(CompareMsg::Start { tab }))
    );

    // Run through `update`: the same effects as the button (here Rename's editor).
    let a = row_of(&app, Side::Local, "a.txt");
    app.update(right_click(tab, Side::Local, Some(a)));
    let rename = index_of(&app, MenuAction::Rename);
    let effects = app.update(Message::Menu(MenuMsg::Choose(rename)));
    assert!(
        matches!(app.tabs[0].local().action, Some(PaneAction::Edit(_))),
        "{effects:?}"
    );
}

#[tokio::test]
async fn choose_disabled_pane_item_does_nothing() {
    let (_fake, mut app, _a, tab) = transfer_tab().await;
    // Nothing selected: the `..` row's menu has Rename and Delete off.
    app.update(right_click(tab, Side::Local, Some(0)));
    let items = open_items(&app);
    assert!(!enabled(&items, MenuAction::Delete));
    let delete = index_of(&app, MenuAction::Delete);
    assert!(
        app.update(Message::Menu(MenuMsg::Choose(delete)))
            .is_empty()
    );
    assert!(app.menu.is_none(), "the menu closes");
    assert_eq!(app.tabs[0].local().action, None, "nothing ran");
}

#[tokio::test]
async fn copy_path_one_and_several() {
    let (_fake, mut app, _a, tab) = transfer_tab().await;
    assert_eq!(app.copy_paths(0, Side::Local), None, "nothing selected");
    let a = row_of(&app, Side::Local, "a.txt");
    app.update(right_click(tab, Side::Local, Some(a)));
    let expected = local_fs_path(&rpath("/home/me/a.txt"))
        .display()
        .to_string();
    assert_eq!(app.copy_paths(0, Side::Local).as_deref(), Some(&*expected));
    let copy = index_of(&app, MenuAction::CopyPath);
    let effects = app.update(Message::Menu(MenuMsg::Choose(copy)));
    let [Effect::WriteClipboard(text)] = &effects[..] else {
        panic!("{effects:?}");
    };
    assert_eq!(text.0, expected);

    // Several remote entries: one full path per line, in row order.
    let sub = row_of(&app, Side::Remote, "sub");
    let r = row_of(&app, Side::Remote, "r.txt");
    app.update(pane_msg(tab, Side::Remote, PaneMsg::Click(sub)));
    app.update(Message::Modifiers(Mods {
        ctrl: true,
        shift: false,
    }));
    app.update(pane_msg(tab, Side::Remote, PaneMsg::Click(r)));
    app.update(Message::Modifiers(Mods::default()));
    app.update(right_click(tab, Side::Remote, Some(r)));
    let copy = index_of(&app, MenuAction::CopyPath);
    let effects = app.update(Message::Menu(MenuMsg::Choose(copy)));
    let [Effect::WriteClipboard(text)] = &effects[..] else {
        panic!("{effects:?}");
    };
    assert_eq!(text.0, "/srv/sub\n/srv/r.txt");
}

#[tokio::test]
async fn refresh_relists_the_pane() {
    let (_fake, mut app, _a, tab) = transfer_tab().await;
    let lists = |effects: &[Effect]| -> Vec<(Side, String)> {
        effects
            .iter()
            .filter_map(|e| match e {
                Effect::List {
                    tab: t, side, path, ..
                } if *t == tab => Some((*side, path.as_str().to_owned())),
                _ => None,
            })
            .collect()
    };
    // F5 on the focused pane.
    app.tabs[0].set_focus(Side::Remote);
    let effects = app.update(Message::Key {
        key: FileKey::Refresh,
        shift: false,
    });
    assert_eq!(lists(&effects), [(Side::Remote, "/srv".to_owned())]);
    // The menu's item.
    app.update(right_click(tab, Side::Local, None));
    let refresh = index_of(&app, MenuAction::Refresh);
    let effects = app.update(Message::Menu(MenuMsg::Choose(refresh)));
    assert_eq!(lists(&effects), [(Side::Local, "/home/me".to_owned())]);
}

#[tokio::test]
async fn keyboard_menu_opens_at_the_cursor_row() {
    let (_fake, mut app, _a, tab) = transfer_tab().await;
    app.update(pane_msg(
        tab,
        Side::Local,
        PaneMsg::Scrolled {
            offset_y: 0.0,
            viewport_h: 600.0,
            origin: Point::new(300.0, 200.0),
        },
    ));
    app.update(pane_msg(tab, Side::Local, PaneMsg::Click(2)));
    app.update(Message::Key {
        key: FileKey::Menu,
        shift: false,
    });
    let menu = app.menu.as_ref().expect("the menu is open");
    assert_eq!(menu.at, Point::new(324.0, 290.0));
    assert_eq!(menu.target, pane_target(tab, Side::Local, Some(2)));
    // Scrolled down a row: the anchor follows.
    app.update(Message::Menu(MenuMsg::Close));
    app.update(pane_msg(
        tab,
        Side::Local,
        PaneMsg::Scrolled {
            offset_y: 30.0,
            viewport_h: 600.0,
            origin: Point::new(300.0, 200.0),
        },
    ));
    app.update(Message::Key {
        key: FileKey::Menu,
        shift: false,
    });
    assert_eq!(app.menu.as_ref().unwrap().at, Point::new(324.0, 260.0));
}

#[tokio::test]
async fn arrows_enter_escape_in_menu() {
    let (_fake, mut app, _a, tab) = transfer_tab().await;
    // `..` selected: Transfer, Rename, Copy path, Delete are off; Open, Refresh and New
    // folder are on.
    app.update(right_click(tab, Side::Local, Some(0)));
    let items = open_items(&app);
    let on: Vec<usize> = (0..items.len()).filter(|&i| items[i].enabled).collect();
    assert_eq!(
        on.iter().map(|&i| items[i].action).collect::<Vec<_>>(),
        [MenuAction::Open, MenuAction::Refresh, MenuAction::NewFolder]
    );
    let cursor = |app: &App| app.menu.as_ref().unwrap().cursor;
    // Down from nothing: the first enabled item; the keys reach nothing else.
    assert!(app.update(key_pressed(FileKey::Down)).is_empty());
    assert_eq!(cursor(&app), Some(on[0]));
    app.update(key_pressed(FileKey::Down));
    assert_eq!(cursor(&app), Some(on[1]));
    app.update(key_pressed(FileKey::Down));
    app.update(key_pressed(FileKey::Down));
    assert_eq!(cursor(&app), Some(on[0]), "wraps past the end");
    app.update(key_pressed(FileKey::Up));
    assert_eq!(cursor(&app), Some(on[2]), "wraps past the start");
    assert_eq!(selected(&app, Side::Local), [0], "the pane did not move");
    // Enter chooses it (New folder: its editor opens).
    app.update(key_pressed(FileKey::Enter));
    assert!(app.menu.is_none());
    assert!(matches!(
        app.tabs[0].local().action,
        Some(PaneAction::Edit(_))
    ));
    app.update(pane_msg(tab, Side::Local, PaneMsg::CancelAction));
    // Escape closes, nothing else.
    app.update(right_click(tab, Side::Local, Some(0)));
    assert!(app.update(key_pressed(FileKey::Escape)).is_empty());
    assert!(app.menu.is_none());
    // Up from nothing: the last enabled item.
    app.update(right_click(tab, Side::Local, Some(0)));
    app.update(key_pressed(FileKey::Up));
    assert_eq!(cursor(&app), Some(on[2]));
    // Enter with no item highlighted chooses nothing.
    app.update(Message::Menu(MenuMsg::Close));
    app.update(right_click(tab, Side::Local, Some(0)));
    assert!(app.update(key_pressed(FileKey::Enter)).is_empty());
    assert!(app.tabs[0].local().action.is_none());
    // Another key closes the menu, then acts as usual (F7: the pane's new folder).
    let effects = app.update(key_pressed(FileKey::NewFolder));
    assert!(app.menu.is_none());
    assert!(
        matches!(
            effects[..],
            [Effect::KeyUnlessTyping {
                key: FileKey::NewFolder,
                ..
            }]
        ),
        "{effects:?}"
    );
}

#[tokio::test]
async fn menu_closes_when_pane_relists() {
    let (fake, mut app, _a, tab) = transfer_tab().await;
    app.update(right_click(tab, Side::Local, Some(1)));
    // A listing of the other pane leaves it open.
    let effects = app.update(pane_msg(tab, Side::Remote, PaneMsg::Refresh));
    settle(&mut app, &fake, run_listings(&fake, &effects).await).await;
    assert!(app.menu.is_some());
    // Its own pane's rows change: it closes.
    app.tabs[0].set_focus(Side::Local);
    let effects = app.update(Message::Key {
        key: FileKey::Refresh,
        shift: false,
    });
    assert!(app.menu.is_some());
    settle(&mut app, &fake, run_listings(&fake, &effects).await).await;
    assert!(app.menu.is_none());
}

#[tokio::test]
async fn menu_on_closed_tab_does_nothing() {
    let (_fake, mut app, _a, tab) = transfer_tab().await;
    let gone = TabId(tab.0 + 1000);
    // Opened for a tab that is not there: no items, a choice does nothing.
    app.update(right_click(gone, Side::Local, Some(1)));
    assert!(app.menu.is_none());
    app.menu = Some(crate::app::ContextMenu {
        at: Point::ORIGIN,
        target: pane_target(gone, Side::Local, Some(1)),
        cursor: None,
    });
    assert!(open_items(&app).is_empty());
    assert!(app.update(Message::Menu(MenuMsg::Choose(0))).is_empty());
    assert!(app.menu.is_none());
}

#[tokio::test]
async fn menu_closes_when_the_shown_tab_changes() {
    let (_fake, mut app, _a, tab) = transfer_tab().await;
    // Ctrl+, opens Settings over it.
    app.update(right_click(tab, Side::Local, Some(1)));
    app.update(Message::OpenSettings);
    assert!(app.menu.is_none());
    // Back on the tab, then a form takes the center.
    app.update(Message::SelectTab(0));
    app.update(right_click(tab, Side::Local, Some(1)));
    app.update(Message::NewSite);
    assert!(app.menu.is_none());
    // The tab closes.
    app.update(Message::SelectTab(0));
    app.update(right_click(tab, Side::Local, Some(1)));
    assert!(app.menu.is_some());
    app.update(Message::CloseTab(tab));
    assert!(app.menu.is_none());
}

#[tokio::test]
async fn right_click_behind_dialog_is_ignored() {
    let (_fake, mut app, _a, tab) = transfer_tab().await;
    app.update(host_key_prompt(7));
    assert!(app.dialogs.is_open());
    assert!(
        app.update(right_click(tab, Side::Local, Some(2)))
            .is_empty()
    );
    assert!(app.menu.is_none());
    assert!(selected(&app, Side::Local).is_empty());
    // Nor behind the palette.
    let (_fake, mut app, _a, tab) = transfer_tab().await;
    app.update(Message::Palette(crate::app::PaletteMsg::Open));
    assert!(app.palette.is_some());
    app.update(right_click(tab, Side::Local, Some(2)));
    assert!(app.menu.is_none());
}
