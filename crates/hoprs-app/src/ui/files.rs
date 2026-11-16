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
use crate::app::{
    App, FailureAction, FileKey, Message, Mods, PaletteMsg, PaneMsg, SiteConn, Tab, TabId, TabState,
};
use crate::backend::Side;
use crate::ui::fit::{self, Columns, MODIFIED_WIDTH, PERMISSIONS_WIDTH, SIZE_WIDTH};
use crate::ui::icons::{self, Icon};
use crate::ui::theme::{self, palette};
use crate::ui::virtual_list::{self, ROW_HEIGHT};
use crate::ui::widgets;
use crate::ui::{file_actions, format, strings};

/// The path bar (mockup `.path`: 38 px).
const PATH_HEIGHT: f32 = 38.0;
/// Width of a selected row's bar.
const SELECTION_BAR: f32 = 2.0;
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
                label: strings::home_dir().to_owned(),
                target: home.clone(),
            }],
            &path.as_str()[home.as_str().len()..],
        ),
        None => (
            vec![Crumb {
                label: strings::path_separator().to_owned(),
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

/// Which breadcrumb segments fit `max_chars` characters: indices in order, one `None`
/// for the elided middle. A segment costs its label plus one separator, the gap two; the
/// first and last always stay, then segments are added from the end while they fit.
pub fn fit_crumbs(crumbs: &[Crumb], max_chars: usize) -> Vec<Option<usize>> {
    let cost = |i: usize| crumbs[i].label.chars().count() + 1;
    let n = crumbs.len();
    let total: usize = (0..n).map(cost).sum();
    if n <= 2 || total <= max_chars {
        return (0..n).map(Some).collect();
    }
    let mut used = cost(0) + cost(n - 1) + 2;
    let mut from = n - 1;
    while from > 1 && used + cost(from - 1) <= max_chars {
        from -= 1;
        used += cost(from);
    }
    let mut out = vec![Some(0)];
    if from > 1 {
        out.push(None);
    }
    out.extend((from..n).map(Some));
    out
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
        // Ctrl+K (or Ctrl+Shift+K): the palette, unless a widget took it (a focused
        // terminal sends it to the shell; its Ctrl+Shift+K comes through the widget).
        keyboard::Event::KeyPressed {
            key,
            physical_key,
            modifiers,
            ..
        } if modifiers.command()
            && !modifiers.alt()
            && key
                .to_latin(physical_key)
                .is_some_and(|c| c.eq_ignore_ascii_case(&'k')) =>
        {
            (status != Status::Captured).then_some(Message::Palette(PaletteMsg::Open))
        }
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
        Named::F4 => FileKey::Edit,
        Named::Escape => FileKey::Escape,
        Named::Space => FileKey::Space,
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
    // The differences panel sizes its rows to the room the tab has.
    iced::widget::responsive(move |size| content(app, tab, size.height)).into()
}

fn content<'a>(app: &'a App, tab: &'a Tab, area: f32) -> Element<'a, Message> {
    let site_name = app
        .site(&tab.site)
        .map_or(strings::site_missing(), |s| s.name.as_str());
    let this_year = format::current_year();
    let panes = row![
        pane(app, tab, Side::Local, strings::local(), this_year),
        rule::vertical(theme::HAIRLINE).style(theme::rule_1px),
        pane(app, tab, Side::Remote, site_name, this_year),
    ]
    .height(Length::Fill);
    let mut col = column![];
    if let Some(ask) = crate::ui::edits::close_ask(app, tab) {
        col = col.push(ask).push(hrule());
    }
    if let Some(banner) = lost_banner(app, tab) {
        col = col.push(banner).push(hrule());
    }
    if let Some(strip) = crate::ui::edits::strip(app, tab) {
        col = col.push(strip).push(hrule());
    }
    let col = col.push(panes);
    match crate::ui::compare::view(app, tab, area) {
        Some(panel) => col.push(panel).into(),
        None => col.into(),
    }
}

/// What the bar across a connected tab says, if anything.
#[derive(Debug, PartialEq)]
pub enum Banner<'a> {
    /// The connection was lost for good: the reason and "Reconnect".
    Lost(&'a Error),
    /// The session found another host key than the one it verified: the blocking
    /// explanation and its actions, never "Reconnect" (which would always fail).
    HostKeyChanged,
    /// A reconnection the app asked for (a click on "Reconnect", or an action on a lost
    /// session) is running: "Reconnecting…", no error.
    Reconnecting,
}

/// The bar a site's tabs show above their content.
pub fn banner(conn: &SiteConn) -> Option<Banner<'_>> {
    if conn.reconnecting && conn.host_key_change().is_none() {
        return Some(Banner::Reconnecting);
    }
    let error = conn.lost_connection()?;
    Some(if conn.host_key_change().is_some() {
        Banner::HostKeyChanged
    } else {
        Banner::Lost(error)
    })
}

/// The connection of a connected tab was lost for good: a thin bar across the tab with
/// the reason and "Reconnect" (the session's own reconnect), no modal. A changed host key
/// shows its blocking explanation there instead, with "Close and reconnect" (or "Remove
/// the old entry…").
pub(crate) fn lost_banner<'a>(app: &'a App, tab: &'a Tab) -> Option<Element<'a, Message>> {
    let conn = app.conn(&tab.site);
    let error = match banner(conn)? {
        Banner::Lost(error) => error,
        Banner::Reconnecting => {
            return Some(
                container(text(strings::reconnecting_now()).color(palette().muted))
                    .width(Length::Fill)
                    .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_MD]))
                    .style(theme::pane_bar)
                    .into(),
            );
        }
        Banner::HostKeyChanged => {
            return Some(
                container(crate::ui::dialogs::host_key_changed_body(app, tab))
                    .width(Length::Fill)
                    .padding(Padding::from([theme::SPACE_XS, theme::SPACE_MD]))
                    .style(theme::pane_bar)
                    .into(),
            );
        }
    };
    let p = palette();
    let mut reason = row![text(strings::disconnected(&strings::error(error))).color(p.error)]
        .spacing(theme::SPACE_XS)
        .align_y(Alignment::Center);
    if let Some(detail) = &error.detail {
        reason = reason.push(
            text(detail)
                .font(theme::MONO)
                .size(theme::TEXT_MONO)
                .color(p.muted),
        );
    }
    let label = if conn.reconnecting {
        strings::reconnecting_now()
    } else {
        strings::reconnect()
    };
    let reconnect = widgets::outlined(
        button(text(label))
            .padding(Padding::from([theme::SPACE_3XS, theme::SPACE_XS]))
            .on_press_maybe((!conn.reconnecting).then_some(Message::ReconnectSession(tab.id))),
        !conn.reconnecting,
    );
    Some(
        container(
            row![reason, space::horizontal(), reconnect]
                .spacing(theme::SPACE_SM)
                .align_y(Alignment::Center),
        )
        .width(Length::Fill)
        .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_MD]))
        .style(theme::pane_bar)
        .into(),
    )
}

fn pane<'a>(
    app: &'a App,
    tab: &'a Tab,
    side: Side,
    place: &'a str,
    this_year: i32,
) -> Element<'a, Message> {
    let Some(pane) = tab.pane(side) else {
        return space().into();
    };
    let msg = move |msg: PaneMsg| Message::Pane {
        tab: tab.id,
        side,
        msg,
    };
    let home = match side {
        Side::Local => app.home.as_ref(),
        Side::Remote => None,
    };
    let focused = tab.focus() == Some(side);
    let width = fit::pane_width(app.window_width);
    let cols = fit::columns(width, side);
    let conn = app.conn(&tab.site);
    let place_icon = app
        .site(&tab.site)
        .map_or(Icon::HardDrives, crate::ui::sidebar::site_icon);
    let body: Element<'a, Message> = match tab.pane_status(conn, side) {
        PaneStatus::Loading => skeleton(cols),
        PaneStatus::Full => list(tab.id, side, pane, this_year, focused, cols, width),
        PaneStatus::Empty => column![
            // `..` stays reachable from an empty folder.
            pane.has_up()
                .then(|| file_row(tab.id, side, pane, 0, this_year, focused, cols, width)),
            centered(text(strings::empty_folder()).color(palette().muted).into()),
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
        path_bar(place, side, place_icon, pane, home, width, msg),
        hrule(),
        file_actions::toolbar(tab, conn, side, fit::toolbar(width, side)),
        hrule(),
    ];
    for bar in file_actions::bars(tab.id, side, pane) {
        content = content.push(bar).push(hrule());
    }
    content = content.push(header(pane, cols, msg)).push(hrule());
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
        || (app.os_drag && side == Side::Remote && tab.can_act(conn, Side::Remote));
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

/// Where the separators go between the kept crumbs of [`fit_crumbs`]: `(separator
/// before, item)` for each. The root "/" is itself the separator after it, so none is
/// drawn right after it (`/…/src`, `/home`).
fn crumb_layout(kept: &[Option<usize>], root_first: bool) -> Vec<(bool, Option<usize>)> {
    kept.iter()
        .enumerate()
        .map(|(n, entry)| {
            let after_root = root_first && n == 1;
            (n > 0 && !after_root, *entry)
        })
        .collect()
}

/// "Local" or the site's name (with its icon), then the clickable breadcrumb in mono,
/// elided in the middle when the pane is too narrow for it.
fn path_bar<'a>(
    place: &'a str,
    side: Side,
    protocol_icon: Icon,
    pane: &'a Pane,
    home: Option<&RemotePath>,
    width: f32,
    msg: impl Fn(PaneMsg) -> Message,
) -> Element<'a, Message> {
    let p = palette();
    let all = breadcrumb_segments(&pane.path, home);
    // Room for the crumbs: the pane minus its padding, the icon and the place name.
    let place_px = place.chars().count() as f32 * theme::TEXT_XS * fit::INTER_ADVANCE;
    let room = width
        - 2.0 * theme::SPACE_MD
        - icons::ROW
        - place_px
        - 2.0 * theme::SPACE_SM
        - theme::SPACE_2XS;
    let max_chars = (room / (theme::TEXT_MONO * fit::MONO_ADVANCE)).max(0.0) as usize;
    let kept = fit_crumbs(&all, max_chars);
    let root_first = crumb_is_root(&pane.path, home);
    let mut crumbs = row![].align_y(Alignment::Center);
    let separator = || {
        text(strings::path_separator())
            .font(theme::MONO)
            .size(theme::TEXT_MONO)
            .color(p.rule_2)
    };
    for (sep, entry) in crumb_layout(&kept, root_first) {
        if sep {
            crumbs = crumbs.push(separator());
        }
        let Some(i) = entry else {
            // The elided middle: "…" with the whole path on hover.
            crumbs = crumbs.push(widgets::tip(
                text(strings::ellipsis())
                    .font(theme::MONO)
                    .size(theme::TEXT_MONO)
                    .color(p.muted),
                pane.path.as_str(),
                None,
            ));
            continue;
        };
        let crumb = &all[i];
        crumbs = crumbs.push(
            button(
                text(crumb.label.clone())
                    .font(theme::MONO)
                    .size(theme::TEXT_MONO)
                    .wrapping(text::Wrapping::None),
            )
            .padding(Padding::from([0.0, theme::SPACE_3XS]))
            .style(theme::crumb)
            .on_press(msg(PaneMsg::Navigate(crumb.target.clone()))),
        );
    }
    let place_icon = match side {
        Side::Local => Icon::Desktop,
        Side::Remote => protocol_icon,
    };
    row![
        icons::icon(place_icon, icons::ROW).color(p.muted),
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
    .spacing(theme::SPACE_2XS)
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
fn header<'a>(
    pane: &Pane,
    cols: Columns,
    msg: impl Fn(PaneMsg) -> Message,
) -> Element<'a, Message> {
    let p = palette();
    let sortable = |label: &'static str, key: SortKey, width: Length, right: bool| {
        let arrow = (pane.sort.key == key).then_some(if pane.sort.descending {
            strings::sort_descending()
        } else {
            strings::sort_ascending()
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
            .padding(Padding {
                left: if key == SortKey::Name {
                    // In line with the names: past the selection bar and the kind icon.
                    theme::SPACE_MD + SELECTION_BAR + icons::ROW + theme::SPACE_XS
                } else {
                    theme::SPACE_MD
                },
                ..Padding::from([theme::SPACE_XS, theme::SPACE_MD])
            })
            .style(theme::header_cell)
            .on_press(msg(PaneMsg::Sort(key)))
    };
    let mut cells = row![
        sortable(strings::col_name(), SortKey::Name, Length::Fill, false),
        sortable(
            strings::col_size(),
            SortKey::Size,
            Length::Fixed(SIZE_WIDTH),
            true
        ),
    ];
    if cols.modified {
        cells = cells.push(sortable(
            strings::col_modified(),
            SortKey::Modified,
            Length::Fixed(MODIFIED_WIDTH),
            false,
        ));
    }
    if cols.permissions {
        cells = cells.push(
            container(
                text(strings::col_permissions())
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
    cols: Columns,
    width: f32,
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
        move |i| file_row(tab, side, pane, i, this_year, focused, cols, width),
    )
}

/// One 30 px row: click selects, double click enters, hover highlights. The entry being
/// renamed shows the name editor in its name cell.
#[allow(clippy::too_many_arguments)]
fn file_row<'a>(
    tab: TabId,
    side: Side,
    pane: &'a Pane,
    index: usize,
    this_year: i32,
    focused: bool,
    cols: Columns,
    width: f32,
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
            strings::parent_dir(),
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
                    strings::no_size().to_owned()
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
    let selected = pane.selection.contains(index);
    // The kind icon: muted, ink when the row is selected.
    let kind_icon = match pane.row(index) {
        Some(Row::Entry(entry)) => icons::for_entry(&entry.name, entry.kind),
        _ => Icon::ArrowBendLeftUp,
    };
    // Cut to the column with "…" (the folder's "/" keeps its character), the whole name
    // on hover only when cut.
    let mut room = fit::name_width(width, cols);
    if is_dir && !up {
        room -= theme::TEXT_MONO * fit::MONO_ADVANCE;
    }
    let shown = fit::ellipsize(name, room, theme::TEXT_MONO, fit::MONO_ADVANCE);
    let cut = matches!(shown, std::borrow::Cow::Owned(_));
    // Folders: medium weight in ink, with a muted "/" (mockup `.dir::after`).
    let name_text: Element<'a, Message> = text(shown)
        .font(if is_dir {
            theme::MONO_MEDIUM
        } else {
            theme::MONO
        })
        .size(theme::TEXT_MONO)
        .wrapping(text::Wrapping::None)
        .into();
    let name_text = if cut {
        widgets::tip(name_text, name, None)
    } else {
        name_text
    };
    let mut name_cell = row![
        icons::icon(kind_icon, icons::ROW).color(if selected { p.ink } else { p.muted }),
        name_text
    ]
    .spacing(theme::SPACE_XS)
    .align_y(Alignment::Center);
    if is_dir && !up {
        name_cell = name_cell.push(
            text(strings::path_separator())
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
    // The selection bar, or the same room transparent so the text never shifts.
    let bar = container(space())
        .width(SELECTION_BAR)
        .height(Length::Fill)
        .style(move |t: &iced::Theme| {
            if selected {
                theme::selection_bar(focused)(t)
            } else {
                container::Style::default()
            }
        });
    let mut cells = row![
        bar,
        name_cell,
        cell(size_text.into(), Length::Fixed(SIZE_WIDTH), true),
    ];
    if cols.modified {
        cells = cells.push(cell(
            data(modified).into(),
            Length::Fixed(MODIFIED_WIDTH),
            false,
        ));
    }
    if cols.permissions {
        cells = cells.push(cell(
            data(permissions)
                .font(theme::MONO)
                .size(theme::TEXT_MONO)
                .into(),
            Length::Fixed(PERMISSIONS_WIDTH),
            false,
        ));
    }
    let hovered = pane.hovered == Some(index);
    let cursor = focused && pane.selection.cursor == Some(index);
    mouse_area(
        container(cells)
            .width(Length::Fill)
            .height(ROW_HEIGHT)
            .style(theme::file_row(selected, hovered, cursor, false)),
    )
    .on_press(msg(PaneMsg::Click(index)))
    .on_double_click(msg(PaneMsg::Open(index)))
    .on_enter(msg(PaneMsg::Hover(index)))
    .on_exit(msg(PaneMsg::Unhover(index)))
    .into()
}

/// Eight grey rows where the entries will be.
fn skeleton<'a>(cols: Columns) -> Element<'a, Message> {
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
        ];
        if cols.modified {
            cells = cells.push(cell(Length::Fixed(MODIFIED_WIDTH), 80.0, false));
        }
        if cols.permissions {
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
            text(strings::error(error)).color(p.error),
            error.detail.as_ref().map(|detail| {
                text(detail)
                    .font(theme::MONO)
                    .size(theme::TEXT_MONO)
                    .color(p.muted)
            }),
            button(text(strings::retry()))
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
pub(crate) fn not_connected<'a>(app: &'a App, tab: &'a Tab) -> Element<'a, Message> {
    let conn = app.conn(&tab.site);
    if conn.host_key_change().is_some() {
        return crate::ui::dialogs::host_key_changed(app, tab);
    }
    let p = palette();
    let failure = match &conn.state {
        TabState::Failed(failure) => Some(failure),
        TabState::Connecting | TabState::Connected(_) => None,
    };
    let mut body = column![text(strings::not_connected()).font(theme::INTER_SEMIBOLD)]
        .spacing(theme::SPACE_XS)
        .align_x(Alignment::Center);
    if let Some(failure) = failure {
        body = body.push(text(strings::error(&failure.error)).color(p.error));
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
                text(strings::keyring_unavailable())
                    .size(theme::TEXT_XS)
                    .color(p.muted),
            );
        }
    }
    let mut actions = row![].spacing(theme::SPACE_XS);
    for action in app.failure_actions(tab) {
        let (label, style, on_press): (_, fn(&iced::Theme, _) -> _, _) = match action {
            FailureAction::Reconnect => (
                strings::reconnect(),
                theme::button_outline,
                Message::ConnectSite(tab.site.clone()),
            ),
            FailureAction::EditSite => (
                strings::edit_site(),
                theme::button_text,
                Message::EditSite(tab.site.clone()),
            ),
            // Drawn by the changed host key view above.
            FailureAction::RemoveOldEntry | FailureAction::CloseAndReconnect => continue,
        };
        actions = actions.push(
            button(text(label))
                .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_XS]))
                .style(style)
                .on_press(on_press),
        );
    }
    body = body.push(actions);
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

    #[test]
    fn fit_crumbs_keeps_ends() {
        let labels = [
            "/",
            "home",
            "me",
            "projects",
            "web-app",
            "src",
            "components",
        ];
        let crumbs: Vec<Crumb> = labels
            .iter()
            .map(|l| Crumb {
                label: (*l).to_owned(),
                target: RemotePath::root(),
            })
            .collect();
        assert_eq!(fit_crumbs(&crumbs, 20), [Some(0), None, Some(5), Some(6)]);
        let all = fit_crumbs(&crumbs, 60);
        assert_eq!(all.len(), 7);
        assert!(all.iter().all(Option::is_some));
        assert_eq!(fit_crumbs(&crumbs[..2], 1), [Some(0), Some(1)]);
    }

    #[test]
    fn crumb_layout_has_no_double_slash_after_root() {
        // `/…/src`: the root, the gap, then a separator only before src.
        assert_eq!(
            crumb_layout(&[Some(0), None, Some(5)], true),
            [(false, Some(0)), (false, None), (true, Some(5))]
        );
        // `/home/me`.
        assert_eq!(
            crumb_layout(&[Some(0), Some(1), Some(2)], true),
            [(false, Some(0)), (false, Some(1)), (true, Some(2))]
        );
        // From `~`: separators everywhere but first.
        assert_eq!(
            crumb_layout(&[Some(0), Some(1)], false),
            [(false, Some(0)), (true, Some(1))]
        );
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

    /// Ctrl+K opens the palette unless a widget took it: a focused terminal captures every
    /// key (Ctrl+K is the shell's there); a text field lets it through. "Ctrl" is iced's
    /// `COMMAND`: Cmd on macOS, as `key_message` matches `modifiers.command()`.
    #[test]
    fn ctrl_k_opens_the_palette_unless_captured() {
        let id = window::Id::unique();
        let press = |c: &str, modifiers| {
            Event::Keyboard(keyboard::Event::KeyPressed {
                key: Key::Character(c.into()),
                modified_key: Key::Character(c.into()),
                physical_key: keyboard::key::Physical::Unidentified(
                    keyboard::key::NativeCode::Unidentified,
                ),
                location: keyboard::Location::Standard,
                modifiers,
                text: None,
                repeat: false,
            })
        };
        let opens = |m: Option<Message>| matches!(m, Some(Message::Palette(PaletteMsg::Open)));
        let ctrl = keyboard::Modifiers::COMMAND;
        let ctrl_shift = keyboard::Modifiers::COMMAND | keyboard::Modifiers::SHIFT;
        assert!(opens(key_message(press("k", ctrl), Status::Ignored, id)));
        assert!(opens(key_message(
            press("K", ctrl_shift),
            Status::Ignored,
            id
        )));
        assert_eq!(
            key_message(press("k", ctrl), Status::Captured, id).map(|m| format!("{m:?}")),
            None
        );
        // Without Ctrl (typing a k), or with Alt, nothing.
        let alt = keyboard::Modifiers::COMMAND | keyboard::Modifiers::ALT;
        for (c, modifiers) in [
            ("k", keyboard::Modifiers::default()),
            ("k", alt),
            ("j", ctrl),
        ] {
            assert!(
                key_message(press(c, modifiers), Status::Ignored, id).is_none(),
                "{c} {modifiers:?}"
            );
        }
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
