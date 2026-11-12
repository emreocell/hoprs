//! A file tab: the local and the remote pane side by side (mockup v2 `.panes`), each with
//! its breadcrumb, the column headers and the virtual list of entries, or the pane's
//! loading, empty, error or not-connected state.

use hoprs_core::{EntryKind, Error, RemotePath};
use iced::advanced::widget::Operation;
use iced::advanced::widget::operation::{Focusable, Outcome};
use iced::event::Status;
use iced::keyboard::{self, Key, key::Named};
use iced::widget::{Id, button, column, container, mouse_area, row, rule, scrollable, space, text};
use iced::{Alignment, Element, Event, Length, Padding, Rectangle, window};

use crate::app::actions::{EditKind, PaneAction};
use crate::app::files::{Pane, PaneStatus, Row, SortKey};
use crate::app::{App, FileKey, Message, Mods, PaneMsg, Tab, TabId, TabState};
use crate::backend::Side;
use crate::ui::theme::{self, palette};
use crate::ui::virtual_list::{self, ROW_HEIGHT};
use crate::ui::{file_actions, format, strings};

/// The path bar (mockup `.path`: 38 px).
const PATH_HEIGHT: f32 = 38.0;
/// Fixed columns; the name takes the rest.
const SIZE_WIDTH: f32 = 96.0;
const MODIFIED_WIDTH: f32 = 128.0;
const PERMISSIONS_WIDTH: f32 = 124.0;
/// Rows of the loading skeleton.
const SKELETON_ROWS: usize = 8;

/// One breadcrumb segment and the folder it opens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Crumb {
    pub label: String,
    pub target: RemotePath,
}

/// The breadcrumb of `path`: `/` then each folder; under `home` (local side) it starts
/// with `~` instead of the folders above home.
pub fn breadcrumb_segments(path: &RemotePath, home: Option<&RemotePath>) -> Vec<Crumb> {
    let home = home.filter(|home| is_within(path, home));
    let (mut crumbs, rest) = match home {
        Some(home) => (
            vec![Crumb {
                label: strings::HOME_DIR.to_owned(),
                target: home.clone(),
            }],
            &path.as_str()[home.as_str().len()..],
        ),
        None => (
            vec![Crumb {
                label: strings::PATH_SEPARATOR.to_owned(),
                target: RemotePath::root(),
            }],
            path.as_str(),
        ),
    };
    let mut target = crumbs[0].target.clone();
    for name in rest.split('/').filter(|name| !name.is_empty()) {
        target = target.join(name);
        crumbs.push(Crumb {
            label: name.to_owned(),
            target: target.clone(),
        });
    }
    crumbs
}

/// The id of a pane's scrollable, for [`crate::app::Effect::ScrollPane`].
pub fn scroll_id(tab: TabId, side: Side) -> Id {
    let side = match side {
        Side::Local => "local",
        Side::Remote => "remote",
    };
    Id::from(format!("files-{}-{side}", tab.0))
}

/// The id of a pane's inline name editor, for [`crate::app::Effect::FocusEdit`].
pub fn edit_id(tab: TabId, side: Side) -> Id {
    let side = match side {
        Side::Local => "local",
        Side::Remote => "remote",
    };
    Id::from(format!("edit-{}-{side}", tab.0))
}

/// The keyboard for the panes: the modifiers always (clicks read them), the keys only
/// when no widget took them. Escape comes through even when a text field took it (to let
/// go of the focus), since it also closes the pane's editor. A text field that has the
/// focus but ignores a key (arrows, Tab, F2, F7…) is caught later, by
/// [`crate::app::Effect::KeyUnlessTyping`].
pub fn key_message(event: Event, status: Status, _: window::Id) -> Option<Message> {
    let Event::Keyboard(event) = event else {
        return None;
    };
    match event {
        keyboard::Event::ModifiersChanged(modifiers) => Some(Message::Modifiers(Mods {
            ctrl: modifiers.command(),
            shift: modifiers.shift(),
        })),
        keyboard::Event::KeyPressed {
            key: Key::Named(named),
            modifiers,
            ..
        } => {
            // Ctrl+Enter sends the selection to the other pane.
            let key = match file_key(named)? {
                FileKey::Enter if modifiers.command() => FileKey::Transfer,
                key => key,
            };
            if status == Status::Captured && key != FileKey::Escape {
                return None;
            }
            Some(Message::KeyPressed {
                key,
                shift: modifiers.shift(),
            })
        }
        _ => None,
    }
}

/// The pane key a named key stands for.
fn file_key(named: Named) -> Option<FileKey> {
    Some(match named {
        Named::ArrowUp => FileKey::Up,
        Named::ArrowDown => FileKey::Down,
        Named::Home => FileKey::Home,
        Named::End => FileKey::End,
        Named::Enter => FileKey::Enter,
        Named::Backspace => FileKey::Back,
        Named::Tab => FileKey::SwitchPane,
        Named::F7 => FileKey::NewFolder,
        Named::F2 => FileKey::Rename,
        Named::Delete => FileKey::Delete,
        Named::Escape => FileKey::Escape,
        _ => return None,
    })
}

/// A widget operation that finds out whether any text field has the focus.
pub fn text_focused() -> impl Operation<bool> {
    struct TextFocused(bool);

    impl Operation<bool> for TextFocused {
        fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<bool>)) {
            operate(self);
        }

        fn focusable(&mut self, _id: Option<&Id>, _bounds: Rectangle, state: &mut dyn Focusable) {
            self.0 |= state.is_focused();
        }

        fn finish(&self) -> Outcome<bool> {
            Outcome::Some(self.0)
        }
    }

    TextFocused(false)
}

/// The tab's two panes.
pub fn view<'a>(app: &'a App, tab: &'a Tab) -> Element<'a, Message> {
    let site_name = app
        .site(&tab.site)
        .map_or(strings::SITE_MISSING, |s| s.name.as_str());
    let this_year = format::current_year();
    row![
        pane(app, tab, Side::Local, strings::LOCAL, this_year),
        rule::vertical(theme::HAIRLINE).style(theme::rule_1px),
        pane(app, tab, Side::Remote, site_name, this_year),
    ]
    .height(Length::Fill)
    .into()
}

fn pane<'a>(
    app: &'a App,
    tab: &'a Tab,
    side: Side,
    place: &'a str,
    this_year: i32,
) -> Element<'a, Message> {
    let pane = tab.pane(side);
    let msg = move |msg: PaneMsg| Message::Pane {
        tab: tab.id,
        side,
        msg,
    };
    let home = match side {
        Side::Local => app.home.as_ref(),
        Side::Remote => None,
    };
    let focused = tab.focus == side;
    let body: Element<'a, Message> = match tab.pane_status(side) {
        PaneStatus::Loading => skeleton(side),
        PaneStatus::Full => list(tab.id, side, pane, this_year, focused),
        PaneStatus::Empty => column![
            // `..` stays reachable from an empty folder.
            pane.has_up()
                .then(|| file_row(tab.id, side, pane, 0, this_year, focused)),
            centered(text(strings::EMPTY_FOLDER).color(palette().muted).into()),
        ]
        .into(),
        PaneStatus::Failed(error) => failed(error, msg(PaneMsg::Retry)),
        PaneStatus::NotConnected => not_connected(app, tab),
    };
    let mut content = column![
        // The focused pane (the one the keyboard drives) has the accent line on top.
        container(space())
            .width(Length::Fill)
            .height(theme::HAIRLINE)
            .style(theme::pane_focus(focused)),
        path_bar(place, pane, home, msg),
        hrule(),
        file_actions::toolbar(tab, side),
        hrule(),
    ];
    for bar in file_actions::bars(tab.id, side, pane) {
        content = content.push(bar).push(hrule());
    }
    content = content.push(header(side, pane, msg)).push(hrule());
    if let Some(row) = file_actions::new_folder_row(tab.id, side, pane) {
        content = content.push(row);
    }
    let content = content
        .push(body)
        .width(Length::FillPortion(1))
        .height(Length::Fill);
    // A drag from the other pane (or files from the system, on the remote side) would
    // drop here: a 1 px accent outline.
    let target = app.drop_target() == Some((tab.id, side))
        || (app.os_drag && side == Side::Remote && tab.can_act(Side::Remote));
    let content = container(content)
        .width(Length::FillPortion(1))
        .height(Length::Fill)
        .style(theme::drop_target(target));
    // A press anywhere focuses the pane for the keyboard, without taking the click; the
    // pointer going in and out tells a drag where it would drop.
    mouse_area(content)
        .on_release(msg(PaneMsg::Focus))
        .on_enter(msg(PaneMsg::PointerIn))
        .on_exit(msg(PaneMsg::PointerOut))
        .into()
}

fn hrule<'a>() -> Element<'a, Message> {
    rule::horizontal(theme::HAIRLINE)
        .style(theme::rule_1px)
        .into()
}

fn centered(content: Element<'_, Message>) -> Element<'_, Message> {
    container(content)
        .center(Length::Fill)
        .padding(theme::SPACE_MD)
        .into()
}

/// "Local" or the site's name, then the clickable breadcrumb in mono.
fn path_bar<'a>(
    place: &'a str,
    pane: &'a Pane,
    home: Option<&RemotePath>,
    msg: impl Fn(PaneMsg) -> Message,
) -> Element<'a, Message> {
    let p = palette();
    let mut crumbs = row![].align_y(Alignment::Center);
    for (i, crumb) in breadcrumb_segments(&pane.path, home)
        .into_iter()
        .enumerate()
    {
        // The root "/" is itself the separator before the first folder.
        if i > 0 && !(i == 1 && crumb_is_root(&pane.path, home)) {
            crumbs = crumbs.push(
                text(strings::PATH_SEPARATOR)
                    .font(theme::MONO)
                    .size(theme::TEXT_MONO)
                    .color(p.rule_2),
            );
        }
        crumbs = crumbs.push(
            button(
                text(crumb.label)
                    .font(theme::MONO)
                    .size(theme::TEXT_MONO)
                    .wrapping(text::Wrapping::None),
            )
            .padding(Padding::from([0.0, theme::SPACE_3XS]))
            .style(theme::crumb)
            .on_press(msg(PaneMsg::Navigate(crumb.target))),
        );
    }
    row![
        text(place)
            .size(theme::TEXT_XS)
            .color(p.muted)
            .wrapping(text::Wrapping::None),
        scrollable(crumbs)
            .direction(scrollable::Direction::Horizontal(
                scrollable::Scrollbar::hidden()
            ))
            .anchor_right()
            .width(Length::Fill),
    ]
    .spacing(theme::SPACE_SM)
    .align_y(Alignment::Center)
    .height(PATH_HEIGHT)
    .padding(Padding::ZERO.left(theme::SPACE_MD).right(theme::SPACE_MD))
    .into()
}

/// Whether the breadcrumb starts at the root (not at `~`).
fn crumb_is_root(path: &RemotePath, home: Option<&RemotePath>) -> bool {
    !home.is_some_and(|home| is_within(path, home))
}

/// `path` is `home` or inside it.
fn is_within(path: &RemotePath, home: &RemotePath) -> bool {
    let (path, home) = (path.as_str(), home.as_str());
    home != "/"
        && (path == home
            || path
                .strip_prefix(home)
                .is_some_and(|rest| rest.starts_with('/')))
}

/// Name, Size, Modified and, on the remote side, Permissions; a click sorts.
fn header<'a>(side: Side, pane: &Pane, msg: impl Fn(PaneMsg) -> Message) -> Element<'a, Message> {
    let p = palette();
    let sortable = |label: &'static str, key: SortKey, width: Length, right: bool| {
        let arrow = (pane.sort.key == key).then_some(if pane.sort.descending {
            strings::SORT_DESCENDING
        } else {
            strings::SORT_ASCENDING
        });
        let mut label = row![text(label).font(theme::INTER_MEDIUM).size(theme::TEXT_XS)]
            .spacing(theme::SPACE_2XS);
        if let Some(arrow) = arrow {
            label = label.push(text(arrow).size(theme::TEXT_XS));
        }
        let label = container(label).width(Length::Fill).align_x(if right {
            Alignment::End
        } else {
            Alignment::Start
        });
        button(label)
            .width(width)
            .padding(Padding::from([theme::SPACE_XS, theme::SPACE_MD]))
            .style(theme::header_cell)
            .on_press(msg(PaneMsg::Sort(key)))
    };
    let mut cells = row![
        sortable(strings::COL_NAME, SortKey::Name, Length::Fill, false),
        sortable(
            strings::COL_SIZE,
            SortKey::Size,
            Length::Fixed(SIZE_WIDTH),
            true
        ),
        sortable(
            strings::COL_MODIFIED,
            SortKey::Modified,
            Length::Fixed(MODIFIED_WIDTH),
            false
        ),
    ];
    if side == Side::Remote {
        cells = cells.push(
            container(
                text(strings::COL_PERMISSIONS)
                    .font(theme::INTER_MEDIUM)
                    .size(theme::TEXT_XS)
                    .color(p.muted),
            )
            .width(PERMISSIONS_WIDTH)
            .padding(Padding::from([theme::SPACE_XS, theme::SPACE_MD])),
        );
    }
    cells.into()
}

fn list<'a>(
    tab: TabId,
    side: Side,
    pane: &'a Pane,
    this_year: i32,
    focused: bool,
) -> Element<'a, Message> {
    virtual_list::view(
        scroll_id(tab, side),
        pane.row_count(),
        pane.offset_y,
        move |viewport| Message::Pane {
            tab,
            side,
            msg: PaneMsg::Scrolled {
                offset_y: viewport.absolute_offset().y,
                viewport_h: viewport.bounds().height,
            },
        },
        move |i| file_row(tab, side, pane, i, this_year, focused),
    )
}

/// One 30 px row: click selects, double click enters, hover highlights. The entry being
/// renamed shows the name editor in its name cell.
fn file_row<'a>(
    tab: TabId,
    side: Side,
    pane: &'a Pane,
    index: usize,
    this_year: i32,
    focused: bool,
) -> Element<'a, Message> {
    let p = palette();
    let msg = move |msg: PaneMsg| Message::Pane { tab, side, msg };
    let cell = |content: Element<'a, Message>, width: Length, right: bool| {
        container(content)
            .width(width)
            .height(Length::Fill)
            .padding(Padding::ZERO.left(theme::SPACE_MD).right(theme::SPACE_MD))
            .align_x(if right {
                Alignment::End
            } else {
                Alignment::Start
            })
            .align_y(Alignment::Center)
            .clip(true)
    };
    let data = |value: String| {
        text(value)
            .size(theme::TEXT_SM)
            .color(p.muted)
            .wrapping(text::Wrapping::None)
    };
    let (name, is_dir, size, modified, permissions) = match pane.row(index) {
        None => return space().height(ROW_HEIGHT).into(),
        Some(Row::Up) => (
            strings::PARENT_DIR,
            true,
            String::new(),
            String::new(),
            String::new(),
        ),
        Some(Row::Entry(entry)) => {
            let is_dir = entry.kind == EntryKind::Dir;
            (
                entry.name.as_str(),
                is_dir,
                if is_dir {
                    strings::NO_SIZE.to_owned()
                } else {
                    format::size(entry.size)
                },
                format::modified(entry.modified, this_year),
                format::permissions(entry.kind, entry.mode),
            )
        }
    };
    let up = matches!(pane.row(index), Some(Row::Up));
    let renaming = match (&pane.action, pane.row(index)) {
        (Some(PaneAction::Edit(edit)), Some(Row::Entry(entry))) => {
            matches!(&edit.kind, EditKind::Rename { from } if *from == entry.name).then_some(edit)
        }
        _ => None,
    };
    // Folders: medium weight in ink, with a muted "/" (mockup `.dir::after`).
    let mut name_cell = row![
        text(name)
            .font(if is_dir {
                theme::MONO_MEDIUM
            } else {
                theme::MONO
            })
            .size(theme::TEXT_MONO)
            .wrapping(text::Wrapping::None)
    ];
    if is_dir && !up {
        name_cell = name_cell.push(
            text(strings::PATH_SEPARATOR)
                .font(theme::MONO)
                .size(theme::TEXT_MONO)
                .color(p.muted),
        );
    }
    let size_text = if is_dir {
        data(size)
    } else {
        text(size)
            .size(theme::TEXT_SM)
            .wrapping(text::Wrapping::None)
    };
    let name_cell: Element<'a, Message> = match renaming {
        // The editor fills the name column; its own padding lines the text up.
        Some(edit) => container(file_actions::name_editor(tab, side, edit))
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(Padding::ZERO.left(theme::SPACE_XS).right(theme::SPACE_XS))
            .align_y(Alignment::Center)
            .into(),
        None => cell(name_cell.into(), Length::Fill, false).into(),
    };
    let mut cells = row![
        name_cell,
        cell(size_text.into(), Length::Fixed(SIZE_WIDTH), true),
        cell(data(modified).into(), Length::Fixed(MODIFIED_WIDTH), false),
    ];
    if side == Side::Remote {
        cells = cells.push(cell(
            data(permissions)
                .font(theme::MONO)
                .size(theme::TEXT_MONO)
                .into(),
            Length::Fixed(PERMISSIONS_WIDTH),
            false,
        ));
    }
    let selected = pane.selection.contains(index);
    let hovered = pane.hovered == Some(index);
    let cursor = focused && pane.selection.cursor == Some(index);
    mouse_area(
        container(cells)
            .width(Length::Fill)
            .height(ROW_HEIGHT)
            .style(theme::file_row(selected, hovered, cursor)),
    )
    .on_press(msg(PaneMsg::Click(index)))
    .on_double_click(msg(PaneMsg::Open(index)))
    .on_enter(msg(PaneMsg::Hover(index)))
    .on_exit(msg(PaneMsg::Unhover(index)))
    .into()
}

/// Eight grey rows where the entries will be.
fn skeleton<'a>(side: Side) -> Element<'a, Message> {
    const NAME_WIDTHS: [f32; SKELETON_ROWS] = [136.0, 96.0, 168.0, 120.0, 88.0, 152.0, 112.0, 72.0];
    let bar = |width: f32| {
        container(space())
            .width(width)
            .height(theme::TEXT_SM)
            .style(theme::skeleton)
    };
    let cell = |width: Length, bar_width: f32, right: bool| {
        container(bar(bar_width))
            .width(width)
            .height(Length::Fill)
            .padding(Padding::ZERO.left(theme::SPACE_MD).right(theme::SPACE_MD))
            .align_x(if right {
                Alignment::End
            } else {
                Alignment::Start
            })
            .align_y(Alignment::Center)
    };
    column(NAME_WIDTHS.into_iter().map(|name_width| {
        let mut cells = row![
            cell(Length::Fill, name_width, false),
            cell(Length::Fixed(SIZE_WIDTH), 40.0, true),
            cell(Length::Fixed(MODIFIED_WIDTH), 80.0, false),
        ];
        if side == Side::Remote {
            cells = cells.push(cell(Length::Fixed(PERMISSIONS_WIDTH), 80.0, false));
        }
        cells.height(ROW_HEIGHT).into()
    }))
    .into()
}

/// The listing failed: what broke and Retry.
fn failed(error: &Error, retry: Message) -> Element<'_, Message> {
    let p = palette();
    centered(
        column![
            text(&error.message).color(p.error),
            error.detail.as_ref().map(|detail| {
                text(detail)
                    .font(theme::MONO)
                    .size(theme::TEXT_MONO)
                    .color(p.muted)
            }),
            button(text(strings::RETRY))
                .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_XS]))
                .style(theme::button_outline)
                .on_press(retry),
        ]
        .spacing(theme::SPACE_XS)
        .align_x(Alignment::Center)
        .into(),
    )
}

/// The remote side without a connection: "Not connected", why, and Reconnect (the
/// sidebar's connect path, which retries the failed session) or Edit site.
fn not_connected<'a>(app: &'a App, tab: &'a Tab) -> Element<'a, Message> {
    let p = palette();
    let failure = match &tab.state {
        TabState::Failed(failure) => Some(failure),
        TabState::Connecting | TabState::Connected(_) => None,
    };
    let mut body = column![text(strings::NOT_CONNECTED).font(theme::INTER_SEMIBOLD)]
        .spacing(theme::SPACE_XS)
        .align_x(Alignment::Center);
    if let Some(failure) = failure {
        body = body.push(text(&failure.error.message).color(p.error));
        if let Some(detail) = &failure.error.detail {
            body = body.push(
                text(detail)
                    .font(theme::MONO)
                    .size(theme::TEXT_MONO)
                    .color(p.muted),
            );
        }
        if failure.keyring_unavailable {
            body = body.push(
                text(strings::KEYRING_UNAVAILABLE)
                    .size(theme::TEXT_XS)
                    .color(p.muted),
            );
        }
    }
    if app.site(&tab.site).is_some() {
        body = body.push(
            row![
                button(text(strings::RECONNECT))
                    .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_XS]))
                    .style(theme::button_outline)
                    .on_press(Message::ConnectSite(tab.site.clone())),
                button(text(strings::EDIT_SITE))
                    .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_XS]))
                    .style(theme::button_text)
                    .on_press(Message::EditSite(tab.site.clone())),
            ]
            .spacing(theme::SPACE_XS),
        );
    }
    centered(body.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(s: &str) -> RemotePath {
        RemotePath::new(s).unwrap()
    }

    fn crumbs(p: &str, home: Option<&str>) -> Vec<(String, String)> {
        let home = home.map(path);
        super::breadcrumb_segments(&path(p), home.as_ref())
            .into_iter()
            .map(|c| (c.label, c.target.to_string()))
            .collect()
    }

    fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter()
            .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
            .collect()
    }

    fn press(named: Named) -> Event {
        Event::Keyboard(keyboard::Event::KeyPressed {
            key: Key::Named(named),
            modified_key: Key::Named(named),
            physical_key: keyboard::key::Physical::Unidentified(
                keyboard::key::NativeCode::Unidentified,
            ),
            location: keyboard::Location::Standard,
            modifiers: keyboard::Modifiers::default(),
            text: None,
            repeat: false,
        })
    }

    /// Keys reach the panes only when no widget took them, except Escape; F2, F7 and Del
    /// are the action keys.
    #[test]
    fn keys_taken_by_a_text_field_stay_there() {
        let id = window::Id::unique();
        let pressed = |named, status| match key_message(press(named), status, id) {
            Some(Message::KeyPressed { key, .. }) => Some(key),
            None => None,
            other => panic!("{other:?}"),
        };
        for (named, key) in [
            (Named::F7, FileKey::NewFolder),
            (Named::F2, FileKey::Rename),
            (Named::Delete, FileKey::Delete),
            (Named::ArrowDown, FileKey::Down),
            (Named::Escape, FileKey::Escape),
        ] {
            assert_eq!(pressed(named, Status::Ignored), Some(key), "{named:?}");
        }
        // A text field took them (Delete edits the text, Enter submits, …).
        for named in [Named::Delete, Named::Enter, Named::Home, Named::Backspace] {
            assert_eq!(pressed(named, Status::Captured), None, "{named:?}");
        }
        // Escape always comes through: it also closes the pane's editor.
        assert_eq!(
            pressed(Named::Escape, Status::Captured),
            Some(FileKey::Escape)
        );
        assert_eq!(pressed(Named::F5, Status::Ignored), None);
    }

    #[test]
    fn breadcrumb_segments() {
        // Remote: the root, then each folder, each opening its own path.
        assert_eq!(crumbs("/", None), pairs(&[("/", "/")]));
        assert_eq!(
            crumbs("/var/www/shop/current", None),
            pairs(&[
                ("/", "/"),
                ("var", "/var"),
                ("www", "/var/www"),
                ("shop", "/var/www/shop"),
                ("current", "/var/www/shop/current"),
            ])
        );
        // Local under home: `~` stands for the folders above.
        assert_eq!(
            crumbs("/home/me/work/shop/dist", Some("/home/me")),
            pairs(&[
                ("~", "/home/me"),
                ("work", "/home/me/work"),
                ("shop", "/home/me/work/shop"),
                ("dist", "/home/me/work/shop/dist"),
            ])
        );
        assert_eq!(
            crumbs("/home/me", Some("/home/me")),
            pairs(&[("~", "/home/me")])
        );
        // Outside home, or a sibling that only shares the prefix: from the root.
        assert_eq!(
            crumbs("/home/meg/x", Some("/home/me")),
            pairs(&[
                ("/", "/"),
                ("home", "/home"),
                ("meg", "/home/meg"),
                ("x", "/home/meg/x"),
            ])
        );
        assert_eq!(
            crumbs("/etc", Some("/home/me")),
            pairs(&[("/", "/"), ("etc", "/etc")])
        );
        // A home of "/" changes nothing.
        assert_eq!(
            crumbs("/etc", Some("/")),
            pairs(&[("/", "/"), ("etc", "/etc")])
        );
        // The separator rule: none right after the root, one between folders.
        assert!(crumb_is_root(&path("/etc"), Some(&path("/home/me"))));
        assert!(!crumb_is_root(&path("/home/me/a"), Some(&path("/home/me"))));
    }
}
