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
    // A disabled item: the menu closes, nothing runs.
    app.update(open());
    let items = app.menu_items(&MenuTarget::SidebarOptions);
    let disabled = items
        .iter()
        .position(|item| !item.enabled)
        .expect("Task 5 wires the sidebar items; until then they are disabled");
    assert!(
        app.update(Message::Menu(MenuMsg::Choose(disabled)))
            .is_empty()
    );
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
