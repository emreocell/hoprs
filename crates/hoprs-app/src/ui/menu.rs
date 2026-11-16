//! The context menu overlay: a small box pinned where the menu was asked for
//! ([`origin`] keeps it inside the window) over a transparent full-window layer that takes
//! every press, right press and scroll outside the box and closes the menu. Items read
//! their icon, label, shortcut (mono, `muted`) and, for check items, a check glyph; a
//! destructive item is in the error colour after a rule; a disabled item is drawn in the
//! `disabled` token and cannot be clicked. No shadow, 1 px outline, 4 px corners.

use iced::widget::{
    button, column, container, mouse_area, opaque, pin, row, rule, space, stack, text,
};
use iced::{Alignment, Color, Element, Length, Padding, Point, Size};

use crate::app::menu::{MenuAction, MenuItem};
use crate::app::{App, MenuMsg, Message};
use crate::backend::Side;
use crate::ui::icons::{self, Icon};
use crate::ui::strings;
use crate::ui::theme::{self, palette};

/// The box's width.
pub const WIDTH: f32 = 224.0;
/// An item's height.
pub const ITEM: f32 = 28.0;
/// A rule between items: 1 px with 4 px above and below.
pub const RULE: f32 = 9.0;
/// The box's inner padding (all sides).
const PADDING: f32 = theme::SPACE_2XS;
/// The least distance between the box and the window's edges.
const EDGE: f32 = 4.0;

fn msg(m: MenuMsg) -> Message {
    Message::Menu(m)
}

/// The item's label.
pub fn label(action: MenuAction) -> &'static str {
    match action {
        MenuAction::ShowWorkspaces => strings::menu_show_workspaces(),
        MenuAction::ShowSites => strings::menu_show_sites(),
        MenuAction::Transfer(Side::Local) => strings::upload(),
        MenuAction::Transfer(Side::Remote) => strings::download(),
        MenuAction::Edit => strings::edit_file(),
        MenuAction::Open => strings::menu_open(),
        MenuAction::Rename => strings::rename(),
        MenuAction::Permissions => strings::permissions(),
        MenuAction::CopyPath => strings::menu_copy_path(),
        MenuAction::Refresh => strings::menu_refresh(),
        MenuAction::NewFolder => strings::new_folder(),
        MenuAction::Delete => strings::delete(),
        MenuAction::Compare => strings::compare_button(),
    }
}

/// The item's icon.
pub fn icon(action: MenuAction) -> Icon {
    match action {
        MenuAction::ShowWorkspaces => Icon::Stack,
        MenuAction::ShowSites => Icon::HardDrives,
        MenuAction::Transfer(Side::Local) => Icon::UploadSimple,
        MenuAction::Transfer(Side::Remote) => Icon::DownloadSimple,
        MenuAction::Edit => Icon::PencilSimpleLine,
        MenuAction::Open => Icon::Folder,
        MenuAction::Rename => Icon::Textbox,
        MenuAction::Permissions => Icon::LockKey,
        MenuAction::CopyPath => Icon::Copy,
        MenuAction::Refresh => Icon::ArrowsClockwise,
        MenuAction::NewFolder => Icon::FolderSimplePlus,
        MenuAction::Delete => Icon::Trash,
        MenuAction::Compare => Icon::GitDiff,
    }
}

/// The item's key, as the rest of the app shows it, if it has one.
pub fn shortcut(action: MenuAction) -> Option<&'static str> {
    match action {
        MenuAction::Transfer(_) => Some(strings::key_transfer()),
        MenuAction::Edit => Some(strings::key_edit()),
        MenuAction::Rename => Some(strings::key_rename()),
        MenuAction::Refresh => Some(strings::key_refresh()),
        MenuAction::NewFolder => Some(strings::key_new_folder()),
        MenuAction::Delete => Some(strings::key_delete()),
        MenuAction::ShowWorkspaces
        | MenuAction::ShowSites
        | MenuAction::Open
        | MenuAction::Permissions
        | MenuAction::CopyPath
        | MenuAction::Compare => None,
    }
}

/// The box's size for `items`.
pub fn size(items: &[MenuItem]) -> Size {
    let rules = items.iter().filter(|item| item.rule_before).count();
    Size::new(
        WIDTH,
        items.len() as f32 * ITEM + rules as f32 * RULE + 2.0 * PADDING,
    )
}

/// Where the box of size `menu` goes for a menu asked for at `at` in a `window`: below
/// and right of `at` when it fits, else above and/or left of it; never closer than 4 px
/// to an edge (the top left wins when the window is smaller than the box).
pub fn origin(at: Point, menu: Size, window: Size) -> Point {
    let axis = |at: f32, len: f32, room: f32| {
        let start = if at + len <= room - EDGE {
            at
        } else {
            at - len
        };
        start.min(room - EDGE - len).max(EDGE)
    };
    Point::new(
        axis(at.x, menu.width, window.width),
        axis(at.y, menu.height, window.height),
    )
}

/// `base` with the open menu over it, if one is open.
///
/// Always a `stack` with `base` first, open or not: iced keeps a widget's state only while
/// its place in the tree keeps its kind, so wrapping `base` only while a menu is open
/// would reset every scroll offset and focus behind it each time one opens or closes.
pub fn overlay<'a>(app: &'a App, base: Element<'a, Message>) -> Element<'a, Message> {
    let Some(menu) = &app.menu else {
        return stack![base].into();
    };
    let items = app.menu_items(&menu.target);
    let window = Size::new(app.window_width, app.window_height);
    let at = origin(menu.at, size(&items), window);

    let mut list = column![];
    for (index, item) in items.iter().enumerate() {
        if item.rule_before {
            list = list.push(
                container(rule::horizontal(theme::HAIRLINE).style(theme::rule_1px))
                    .height(RULE)
                    .center_y(RULE),
            );
        }
        list = list.push(item_row(index, item, menu.cursor == Some(index)));
    }
    let boxed = container(list)
        .width(WIDTH)
        .padding(PADDING)
        .style(theme::dialog_box);

    // A press, right press or scroll anywhere outside the box closes the menu; nothing
    // behind it gets the pointer. The box takes its own presses (a disabled item's too).
    let backdrop = opaque(
        mouse_area(space().width(Length::Fill).height(Length::Fill))
            .on_press(msg(MenuMsg::Close))
            .on_right_press(msg(MenuMsg::Close))
            .on_middle_press(msg(MenuMsg::Close))
            .on_scroll(|_| msg(MenuMsg::Close)),
    );
    let layer = pin(opaque(boxed)).position(at);
    stack![base, backdrop, layer].into()
}

/// One item: icon, label, shortcut, check; chosen with a click when enabled. The pointer
/// over an enabled item highlights it ([`MenuMsg::Hover`]), as the arrows do: the row
/// lights up and its icon goes from `muted` to `ink`.
fn item_row<'a>(index: usize, item: &MenuItem, highlighted: bool) -> Element<'a, Message> {
    let p = palette();
    // `None` lets the label follow the row's hover colour.
    let (ink, glyph): (Option<Color>, Color) = if !item.enabled {
        (Some(p.disabled), p.disabled)
    } else if item.destructive {
        (Some(p.error), p.error)
    } else if highlighted {
        (None, p.ink)
    } else {
        (None, p.muted)
    };
    let mut label = text(label(item.action))
        .size(theme::TEXT_SM)
        .wrapping(text::Wrapping::None);
    if let Some(ink) = ink {
        label = label.color(ink);
    }
    let shortcut = shortcut(item.action).map(|key| {
        text(key)
            .font(theme::MONO)
            .size(theme::TEXT_TAG)
            .color(if item.enabled { p.muted } else { p.disabled })
    });
    // The check column keeps its width when unchecked, so labels line up.
    let check: Element<'a, Message> = match item.checked {
        Some(true) => icons::icon(Icon::Check, icons::ROW).color(glyph).into(),
        _ => space().width(icons::ROW).into(),
    };
    let line = row![
        icons::icon(icon(item.action), icons::ROW).color(glyph),
        label,
        space::horizontal(),
        shortcut,
        check,
    ]
    .spacing(theme::SPACE_XS)
    .align_y(Alignment::Center);
    let button = button(
        container(line)
            .height(Length::Fill)
            .align_y(Alignment::Center),
    )
    .width(Length::Fill)
    .height(ITEM)
    .padding(Padding::ZERO.left(theme::SPACE_XS).right(theme::SPACE_XS))
    .style(if highlighted {
        theme::row_selected
    } else {
        theme::row
    })
    .on_press_maybe(item.enabled.then(|| msg(MenuMsg::Choose(index))));
    if item.enabled {
        mouse_area(button)
            .on_enter(msg(MenuMsg::Hover(index)))
            .into()
    } else {
        button.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::menu::{MenuAction, MenuItem};

    fn item(rule_before: bool) -> MenuItem {
        MenuItem {
            action: MenuAction::ShowWorkspaces,
            enabled: true,
            checked: None,
            destructive: false,
            rule_before,
        }
    }

    fn inside(origin: Point, menu: Size, window: Size) -> bool {
        origin.x >= EDGE
            && origin.y >= EDGE
            && origin.x + menu.width <= window.width - EDGE
            && origin.y + menu.height <= window.height - EDGE
    }

    #[test]
    fn origin_stays_inside_window() {
        let window = Size::new(1280.0, 800.0);
        let menu = Size::new(WIDTH, 3.0 * ITEM);
        // Near the bottom right corner: flipped up and left of the pointer.
        let at = Point::new(1270.0, 790.0);
        let o = origin(at, menu, window);
        assert!(inside(o, menu, window), "{o:?}");
        assert_eq!(o, Point::new(1270.0 - WIDTH, 790.0 - menu.height));
        // Room below and to the right: right at the pointer.
        assert_eq!(
            origin(Point::new(10.0, 10.0), menu, window),
            Point::new(10.0, 10.0)
        );
        // Too close to the top left edge: clamped 4 px in.
        assert_eq!(
            origin(Point::new(1.0, 0.0), menu, window),
            Point::new(EDGE, EDGE)
        );
        // A window smaller than the menu: pinned to the top left edge.
        let tiny = Size::new(100.0, 50.0);
        assert_eq!(
            origin(Point::new(60.0, 30.0), menu, tiny),
            Point::new(EDGE, EDGE)
        );
    }

    #[test]
    fn size_counts_items_and_rules() {
        let pad = 2.0 * PADDING;
        assert_eq!(size(&[]), Size::new(WIDTH, pad));
        assert_eq!(size(&[item(false)]), Size::new(WIDTH, ITEM + pad));
        assert_eq!(
            size(&[item(false), item(false), item(true)]),
            Size::new(WIDTH, 3.0 * ITEM + RULE + pad)
        );
    }

    #[test]
    fn every_action_has_a_label_and_icon() {
        for action in [MenuAction::ShowWorkspaces, MenuAction::ShowSites] {
            assert!(!label(action).is_empty());
            let _ = icon(action);
            assert_eq!(shortcut(action), None);
        }
        // The pane items, with the keys the toolbar shows.
        for (action, key) in [
            (MenuAction::Transfer(Side::Local), Some("Ctrl ↵")),
            (MenuAction::Transfer(Side::Remote), Some("Ctrl ↵")),
            (MenuAction::Edit, Some("F4")),
            (MenuAction::Open, None),
            (MenuAction::Rename, Some("F2")),
            (MenuAction::Permissions, None),
            (MenuAction::CopyPath, None),
            (MenuAction::Refresh, Some("F5")),
            (MenuAction::NewFolder, Some("F7")),
            (MenuAction::Delete, Some("Del")),
            (MenuAction::Compare, None),
        ] {
            assert!(!label(action).is_empty(), "{action:?}");
            let _ = icon(action);
            assert_eq!(shortcut(action), key, "{action:?}");
        }
        assert_ne!(
            label(MenuAction::Transfer(Side::Local)),
            label(MenuAction::Transfer(Side::Remote))
        );
    }
}
