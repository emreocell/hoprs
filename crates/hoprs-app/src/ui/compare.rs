//! The folder compare panel (spec "The panel"), between the file panes and the transfer
//! queue: the head with both roots, the automatic toggle, Collapse and the cross; the
//! groups of differences in a virtual list (a header with select-all, then one 30 px row
//! each: checkbox, relative path in mono, status, sizes); and the foot with the progress or
//! result line and the actions. Nothing here is a modal.
//!
//! What the panel says is built by plain functions ([`head`], [`lines`], [`row_text`],
//! [`footer`]) from the tab's [`CompareState`], so the tests check the texts; [`view`] only
//! draws them.

use hoprs_core::EntryKind;
use hoprs_remote::Status;
use iced::widget::{
    Id, button, checkbox, column, container, mouse_area, row, rule, space, text, tooltip,
};
use iced::{Alignment, Element, Length, Padding};

use crate::app::compare::{CompareState, Group, Phase, Row, RowState};
use crate::app::{App, CompareMsg, Message, Panel, Tab, TabId};
use crate::ui::divider;
use crate::ui::fit;
use crate::ui::icons::{self, Icon};
use crate::ui::layout::QUEUE_HEAD_HEIGHT;
use crate::ui::theme::{self, palette};
use crate::ui::virtual_list::{self, ROW_HEIGHT};
use crate::ui::{format, strings, widgets};

/// The panel's share of the tab area until the user drags it.
pub const DEFAULT_SHARE: f32 = 0.30;
/// The divider, the head and the foot with their rules.
const CHROME_HEIGHT: f32 = divider::HIT + 2.0 * QUEUE_HEAD_HEIGHT + 2.0 * theme::HAIRLINE;
/// The panel's least height: its divider, head and foot, their rules and two rows.
pub const MIN_HEIGHT: f32 = CHROME_HEIGHT + MIN_BODY_HEIGHT;
/// The rows' region's least height.
pub const MIN_BODY_HEIGHT: f32 = 2.0 * ROW_HEIGHT;
const STATUS_WIDTH: f32 = 148.0;
const SIZES_WIDTH: f32 = 128.0;

/// The id of the panel's list, for its scroll offset.
pub fn scroll_id(tab: TabId) -> Id {
    Id::from(format!("compare-{}", tab.0))
}

/// What the head says.
#[derive(Debug, Clone, PartialEq)]
pub struct Head {
    pub title: &'static str,
    pub local: String,
    pub remote: String,
    pub auto: bool,
}

/// The head: the roots the result is about (the panes' folders before a run captured any).
pub fn head(state: &CompareState, auto: bool) -> Head {
    let (local, remote) = state
        .roots
        .as_ref()
        .map(|(l, r)| (l.to_string(), r.to_string()))
        .unwrap_or_default();
    Head {
        title: strings::compare_title(),
        local,
        remote,
        auto,
    }
}

/// `path` for a narrow head: its last folder under `…/`, when there is more above it.
pub fn short_root(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rsplit_once('/') {
        Some((above, last)) if !above.is_empty() => {
            format!("{}/{last}", strings::ellipsis())
        }
        _ => path.to_owned(),
    }
}

/// The icon of a group's header.
pub fn group_icon(group: Group) -> Icon {
    match group {
        Group::OnlyLocal => Icon::Desktop,
        Group::OnlyRemote => Icon::HardDrives,
        Group::Different => Icon::ArrowsLeftRight,
        Group::Unreadable => Icon::WarningCircle,
    }
}

/// One line of the list: a group header or a difference (index into `rows`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Line {
    Header(Group),
    Row(usize),
}

/// The list's lines: each non-empty group's header and its rows.
pub fn lines(state: &CompareState) -> Vec<Line> {
    let mut lines = Vec::with_capacity(state.rows.len() + Group::ORDER.len());
    for group in Group::ORDER {
        let start = lines.len();
        lines.push(Line::Header(group));
        lines.extend(
            state
                .rows
                .iter()
                .enumerate()
                .filter(|(_, row)| row.group() == group)
                .map(|(i, _)| Line::Row(i)),
        );
        if lines.len() == start + 1 {
            lines.pop();
        }
    }
    lines
}

/// The line number of row `row` in [`lines`].
pub fn line_of(state: &CompareState, row: usize) -> usize {
    lines(state)
        .iter()
        .position(|line| *line == Line::Row(row))
        .unwrap_or(0)
}

fn group_count(state: &CompareState, group: Group) -> usize {
    state.rows.iter().filter(|r| r.group() == group).count()
}

/// A group header's text, with its count.
pub fn group_title(state: &CompareState, group: Group) -> String {
    let count = group_count(state, group);
    match group {
        Group::OnlyLocal => strings::compare_group_only_local(count),
        Group::OnlyRemote => strings::compare_group_only_remote(count),
        Group::Different => strings::compare_group_different(count),
        Group::Unreadable => strings::compare_group_unreadable(count),
    }
}

/// Whether a group's select-all is checked: it has rows to select and all are selected.
pub fn group_checked(state: &CompareState, group: Group) -> bool {
    let mut rows = state
        .rows
        .iter()
        .filter(|r| r.group() == group && r.selectable() && r.transferable())
        .peekable();
    rows.peek().is_some() && rows.all(|r| r.selected)
}

/// What a row says.
#[derive(Debug, Clone, PartialEq)]
pub struct RowText {
    /// Relative path (a folder ends with `/`).
    pub path: String,
    /// What differs, or the row's transfer state; empty for one-sided items.
    pub status: String,
    /// `local · remote` sizes, or the one there is; empty for folders.
    pub sizes: String,
    /// Both dates, on hover.
    pub tip: Option<String>,
}

/// The texts of one row.
pub fn row_text(row: &Row, this_year: i32) -> RowText {
    let diff = &row.diff;
    let is_dir = diff.kind == EntryKind::Dir;
    let path = if is_dir {
        format!("{}{}", diff.path, strings::path_separator())
    } else {
        diff.path.clone()
    };
    let status = match &row.state {
        RowState::Idle => match &diff.status {
            Status::OnlyLocal | Status::OnlyRemote => String::new(),
            Status::LocalNewer => strings::compare_local_newer().to_owned(),
            Status::RemoteNewer => strings::compare_remote_newer().to_owned(),
            Status::SizeDiffers => strings::compare_size_differs().to_owned(),
            Status::KindDiffers => strings::compare_kind_differs().to_owned(),
            Status::Unreadable { error, .. } => strings::compare_unreadable(&strings::error(error)),
        },
        RowState::Queued(_) => strings::compare_queued().to_owned(),
        RowState::Done => strings::transfer_done().to_owned(),
        RowState::Failed(error) => strings::error(error),
        RowState::Skipped(reason) => strings::skipped(reason),
    };
    let size = |entry: &Option<hoprs_core::RemoteEntry>| {
        entry
            .as_ref()
            .filter(|e| e.kind == EntryKind::File)
            .map(|e| format::size(e.size))
    };
    let sizes = match (size(&diff.local), size(&diff.remote)) {
        (Some(l), Some(r)) => format!("{l} · {r}"),
        (Some(one), None) | (None, Some(one)) => one,
        (None, None) => String::new(),
    };
    let tip = match (&diff.local, &diff.remote) {
        (Some(l), Some(r)) => Some(format!(
            "{}\n{}",
            strings::compare_tip_local(&format::modified(l.modified, this_year)),
            strings::compare_tip_remote(&format::modified(r.modified, this_year))
        )),
        _ => None,
    };
    RowText {
        path,
        status,
        sizes,
        tip,
    }
}

/// A button of the foot besides Upload and Download.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Act {
    Stop,
    Continue,
    Again,
}

/// What the foot says and offers.
#[derive(Debug, Clone, PartialEq)]
pub struct Footer {
    pub text: String,
    pub actions: Vec<Act>,
    /// "The two folders are identical (n items)", shown in the body when nothing differs.
    pub all_identical: Option<String>,
    pub upload: bool,
    pub download: bool,
}

/// The foot: the progress, limit, stopped or result line, its buttons, and whether Upload
/// selected / Download selected are enabled.
pub fn footer(state: &CompareState) -> Footer {
    let (text, actions) = match &state.phase {
        // The wait before a run keeps the last result on screen: its summary, no buttons.
        Phase::Idle | Phase::Debouncing => (
            strings::compare_identical(state.identical as usize),
            Vec::new(),
        ),
        Phase::Running { seen } => (strings::compare_running(seen.0 + seen.1), vec![Act::Stop]),
        Phase::Paused { seen } => (
            strings::compare_paused(seen.0 + seen.1),
            vec![Act::Continue, Act::Stop],
        ),
        Phase::Done { partial: false } => (
            strings::compare_identical(state.identical as usize),
            Vec::new(),
        ),
        Phase::Done { partial: true } => (strings::compare_partial().to_owned(), vec![Act::Again]),
        Phase::Stopped(error) => (
            strings::compare_stopped(&strings::error(error)),
            vec![Act::Again],
        ),
    };
    let all_identical = (matches!(state.phase, Phase::Done { partial: false })
        && state.rows.is_empty())
    .then(|| strings::compare_all_identical(state.identical as usize));
    Footer {
        text,
        actions,
        all_identical,
        upload: state.selected_uploads().next().is_some(),
        download: state.selected_downloads().next().is_some(),
    }
}

/// What the collapsed panel's single line says.
#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    pub only_local: usize,
    pub only_remote: usize,
    pub different: usize,
    /// The footer's text while a run is going, paused or failed (or nothing differs); the
    /// counts are shown otherwise.
    pub progress: Option<String>,
}

/// The counts of the three groups (not the unreadable ones), or the run's progress.
pub fn summary(state: &CompareState) -> Summary {
    // A run in flight, paused or failed always says so; an identical result or the
    // wait before a run shows the footer's text only when there are no rows to count.
    let progress = match state.phase {
        Phase::Running { .. }
        | Phase::Paused { .. }
        | Phase::Stopped(_)
        | Phase::Done { partial: true } => true,
        Phase::Debouncing | Phase::Done { partial: false } => state.rows.is_empty(),
        Phase::Idle => false,
    };
    Summary {
        only_local: group_count(state, Group::OnlyLocal),
        only_remote: group_count(state, Group::OnlyRemote),
        different: group_count(state, Group::Different),
        progress: progress.then(|| footer(state).text),
    }
}

/// The collapsed panel: one line with the title, the counts, the toggle and the buttons.
fn summary_line<'a>(
    id: TabId,
    state: &'a CompareState,
    auto: bool,
    msg: impl Fn(CompareMsg) -> Message + Copy + 'a,
) -> Element<'a, Message> {
    let p = palette();
    let sum = summary(state);
    let count = |group: Group, label: String| {
        row![
            icons::icon(group_icon(group), icons::SMALL).color(p.muted),
            text(label)
                .size(theme::TEXT_SM)
                .color(p.muted)
                .wrapping(text::Wrapping::None),
        ]
        .spacing(theme::SPACE_3XS)
        .align_y(Alignment::Center)
    };
    let middle: Element<'a, Message> = match sum.progress {
        Some(progress) => text(progress)
            .size(theme::TEXT_SM)
            .color(p.muted)
            .wrapping(text::Wrapping::None)
            .into(),
        None => row![
            count(
                Group::OnlyLocal,
                strings::compare_summary_only_here(sum.only_local)
            ),
            count(
                Group::OnlyRemote,
                strings::compare_summary_only_server(sum.only_remote)
            ),
            count(
                Group::Different,
                strings::compare_summary_different(sum.different)
            ),
        ]
        .spacing(theme::SPACE_MD)
        .align_y(Alignment::Center)
        .into(),
    };
    row![
        icons::icon(Icon::GitDiff, icons::SMALL).color(p.muted),
        text(strings::compare_title())
            .font(theme::INTER_SEMIBOLD)
            .wrapping(text::Wrapping::None),
        container(middle).width(Length::Fill).clip(true),
        checkbox(auto)
            .label(strings::compare_auto())
            .text_size(theme::TEXT_XS)
            .size(theme::TEXT_SM)
            .style(theme::checkbox)
            .on_toggle(move |on| msg(CompareMsg::ToggleAuto { tab: id, on })),
        widgets::icon_button(
            Icon::CaretUp,
            strings::compare_expand(),
            None,
            Some(msg(CompareMsg::ToggleCollapsed { tab: id })),
        ),
        widgets::icon_button(
            Icon::X,
            strings::compare_close(),
            None,
            Some(msg(CompareMsg::Hide { tab: id })),
        ),
    ]
    .spacing(theme::SPACE_MD)
    .align_y(Alignment::Center)
    .height(QUEUE_HEAD_HEIGHT)
    .padding(Padding::ZERO.left(theme::SPACE_MD).right(theme::SPACE_SM))
    .into()
}

/// The panel, when the tab shows one.
pub fn view<'a>(app: &'a App, tab: &'a Tab, area: f32) -> Option<Element<'a, Message>> {
    let state = tab.compare().filter(|s| s.visible())?;
    let id = tab.id;
    let p = palette();
    let msg = move |m: CompareMsg| Message::Compare(m);
    if state.collapsed {
        // No divider: there is no height to drag, and expanding restores the saved one.
        return Some(
            container(column![
                rule::horizontal(theme::HAIRLINE).style(theme::rule_1px),
                summary_line(id, state, app.settings.compare_auto, msg),
            ])
            .width(Length::Fill)
            .into(),
        );
    }
    let head = head(state, app.settings.compare_auto);
    let compact = app.window_width < 1180.0;
    let height = fit::compare_height(area, app.settings.compare_height);

    let (local, remote) = if compact {
        (short_root(&head.local), short_root(&head.remote))
    } else {
        (head.local.clone(), head.remote.clone())
    };
    let full = strings::compare_roots(&head.local, &head.remote);
    let roots = row![
        text(local)
            .font(theme::MONO)
            .size(theme::TEXT_MONO)
            .color(p.muted)
            .wrapping(text::Wrapping::None),
        text(strings::compare_roots("", ""))
            .size(theme::TEXT_SM)
            .color(p.muted)
            .wrapping(text::Wrapping::None),
        text(remote)
            .font(theme::MONO)
            .size(theme::TEXT_MONO)
            .color(p.muted)
            .wrapping(text::Wrapping::None),
    ]
    .spacing(theme::SPACE_XS)
    .align_y(Alignment::Center);
    let top = row![
        text(head.title)
            .font(theme::INTER_SEMIBOLD)
            .wrapping(text::Wrapping::None),
        tooltip(
            container(roots).width(Length::Fill).clip(true),
            container(text(full).font(theme::MONO).size(theme::TEXT_MONO))
                .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_XS]))
                .style(theme::dialog_box),
            tooltip::Position::Top,
        ),
        checkbox(head.auto)
            .label(strings::compare_auto())
            .text_size(theme::TEXT_XS)
            .size(theme::TEXT_SM)
            .style(theme::checkbox)
            .on_toggle(move |on| msg(CompareMsg::ToggleAuto { tab: id, on })),
        widgets::icon_button(
            Icon::CaretDown,
            strings::compare_collapse(),
            None,
            Some(msg(CompareMsg::ToggleCollapsed { tab: id })),
        ),
        widgets::icon_button(
            Icon::X,
            strings::compare_close(),
            None,
            Some(msg(CompareMsg::Hide { tab: id })),
        ),
    ]
    .spacing(theme::SPACE_MD)
    .align_y(Alignment::Center)
    .height(QUEUE_HEAD_HEIGHT)
    .padding(Padding::ZERO.left(theme::SPACE_MD).right(theme::SPACE_SM));

    let foot_content = footer(state);
    let lines = lines(state);
    let dragged = app.resize.is_some_and(|r| r.panel == Panel::Compare);
    let cap = fit::compare_height(area, Some(f32::INFINITY));
    let divider = divider::view(Panel::Compare, height, cap, dragged);
    let region = column![divider, top]
        .push(rule::horizontal(theme::HAIRLINE).style(theme::rule_1px))
        .push(
            container(body(
                tab,
                state,
                &foot_content,
                lines,
                height - CHROME_HEIGHT,
                app.window_width,
            ))
            .width(Length::Fill)
            .height(height - CHROME_HEIGHT)
            .clip(true),
        )
        .push(rule::horizontal(theme::HAIRLINE).style(theme::rule_1px))
        .push(foot(id, foot_content, compact));
    Some(container(region).width(Length::Fill).into())
}

fn body<'a>(
    tab: &'a Tab,
    state: &'a CompareState,
    footer: &Footer,
    lines: Vec<Line>,
    max_height: f32,
    window: f32,
) -> Element<'a, Message> {
    let id = tab.id;
    if let Some(identical) = footer.all_identical.clone() {
        return container(text(identical).color(palette().muted))
            .padding(Padding::from([theme::SPACE_SM, theme::SPACE_MD]))
            .width(Length::Fill)
            .into();
    }
    if lines.is_empty() {
        // Running with nothing found yet.
        return space().height(ROW_HEIGHT).into();
    }
    let height = (lines.len() as f32 * ROW_HEIGHT).min(max_height);
    let this_year = format::current_year();
    let list = virtual_list::view(
        scroll_id(id),
        lines.len(),
        state.offset_y,
        move |viewport| {
            Message::Compare(CompareMsg::Scrolled {
                tab: id,
                offset_y: viewport.absolute_offset().y,
            })
        },
        move |i| match lines.get(i) {
            Some(Line::Header(group)) => header_row(id, state, *group),
            Some(Line::Row(index)) => diff_row(id, state, *index, this_year, window),
            None => space().height(ROW_HEIGHT).into(),
        },
    );
    // Where the list is, for a menu opened from the keyboard (Shift+F10).
    crate::ui::bounds::report(
        container(list).height(height),
        state.bounds,
        move |bounds| Message::Compare(CompareMsg::Placed { tab: id, bounds }),
    )
}

fn header_row<'a>(id: TabId, state: &'a CompareState, group: Group) -> Element<'a, Message> {
    let any = state
        .rows
        .iter()
        .any(|r| r.group() == group && r.selectable() && r.transferable());
    let title = text(group_title(state, group))
        .font(theme::INTER_SEMIBOLD)
        .size(theme::TEXT_SM)
        .wrapping(text::Wrapping::None);
    let content: Element<'a, Message> = if any {
        checkbox(group_checked(state, group))
            .label(group_title(state, group))
            .text_size(theme::TEXT_SM)
            .size(theme::TEXT_SM)
            .font(theme::INTER_SEMIBOLD)
            .style(theme::checkbox)
            .on_toggle(move |on| Message::Compare(CompareMsg::SelectGroup { tab: id, group, on }))
            .into()
    } else {
        title.into()
    };
    let content = row![
        icons::icon(group_icon(group), icons::ROW).color(palette().muted),
        content
    ]
    .spacing(theme::SPACE_XS)
    .align_y(Alignment::Center);
    container(content)
        .width(Length::Fill)
        .height(ROW_HEIGHT)
        .padding(Padding::ZERO.left(theme::SPACE_MD).right(theme::SPACE_MD))
        .align_y(Alignment::Center)
        .style(theme::pane_bar)
        .into()
}

/// The text width of a row's path cell in a window `window` px wide: the panel's width
/// minus the paddings, the checkbox, the icon, the status and sizes columns and the gaps.
fn path_width(window: f32) -> f32 {
    let fixed = 2.0
        + 2.0 * theme::SPACE_MD
        + theme::TEXT_SM
        + icons::ROW
        + theme::SPACE_XS
        + STATUS_WIDTH
        + SIZES_WIDTH
        + 3.0 * theme::SPACE_SM;
    (fit::center_width(window) - fixed).max(0.0)
}

fn diff_row<'a>(
    id: TabId,
    state: &'a CompareState,
    index: usize,
    this_year: i32,
    window: f32,
) -> Element<'a, Message> {
    let p = palette();
    let Some(row_data) = state.rows.get(index) else {
        return space().height(ROW_HEIGHT).into();
    };
    let t = row_text(row_data, this_year);
    let enabled = row_data.selectable() && row_data.transferable();
    let failed = matches!(row_data.state, RowState::Failed(_))
        || matches!(row_data.diff.status, Status::Unreadable { .. })
            && matches!(row_data.state, RowState::Idle);
    let status_color = if failed { p.error } else { p.muted };
    let mut check = checkbox(row_data.selected)
        .size(theme::TEXT_SM)
        .style(theme::checkbox);
    if enabled {
        check =
            check.on_toggle(move |on| Message::Compare(CompareMsg::Select { tab: id, index, on }));
    }
    let cursor = state.focused && state.cursor == Some(index);
    let cell = |content: Element<'a, Message>, width: Length, right: bool| {
        container(content)
            .width(width)
            .height(Length::Fill)
            .align_x(if right {
                Alignment::End
            } else {
                Alignment::Start
            })
            .align_y(Alignment::Center)
            .clip(true)
    };
    let name = row_data
        .diff
        .path
        .rsplit('/')
        .next()
        .unwrap_or(&row_data.diff.path);
    let kind_icon = icons::for_entry(name, row_data.diff.kind);
    let path_room = path_width(window);
    let shown = fit::ellipsize(&t.path, path_room, theme::TEXT_MONO, fit::MONO_ADVANCE);
    let cut = matches!(shown, std::borrow::Cow::Owned(_));
    let path: Element<'a, Message> = text(shown.into_owned())
        .font(theme::MONO)
        .size(theme::TEXT_MONO)
        .wrapping(text::Wrapping::None)
        .into();
    let path = if cut {
        widgets::tip(path, &row_data.diff.path, None)
    } else {
        path
    };
    let path = row![
        icons::icon(kind_icon, icons::ROW).color(if row_data.selected { p.ink } else { p.muted }),
        path
    ]
    .spacing(theme::SPACE_XS)
    .align_y(Alignment::Center);
    let status = text(t.status)
        .size(theme::TEXT_SM)
        .color(status_color)
        .wrapping(text::Wrapping::None);
    let sizes = text(t.sizes)
        .font(theme::MONO)
        .size(theme::TEXT_MONO)
        .color(p.muted)
        .wrapping(text::Wrapping::None);
    let line = row![
        container(check)
            .height(Length::Fill)
            .align_y(Alignment::Center),
        cell(path.into(), Length::Fill, false),
        cell(status.into(), Length::Fixed(STATUS_WIDTH), false),
        cell(sizes.into(), Length::Fixed(SIZES_WIDTH), true),
    ]
    .spacing(theme::SPACE_SM)
    .padding(Padding::ZERO.left(theme::SPACE_MD - 2.0))
    .height(ROW_HEIGHT);
    let bar = container(space())
        .width(2)
        .height(Length::Fill)
        .style(move |t: &iced::Theme| {
            if row_data.selected {
                theme::selection_bar(state.focused)(t)
            } else {
                container::Style::default()
            }
        });
    let line = container(row![bar, line])
        .padding(Padding::ZERO.right(theme::SPACE_MD))
        .style(theme::file_row(row_data.selected, false, cursor, false));
    let line: Element<'a, Message> = match t.tip {
        Some(tip) => tooltip(
            line,
            container(text(tip).size(theme::TEXT_XS))
                .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_XS]))
                .style(theme::dialog_box),
            tooltip::Position::Bottom,
        )
        .into(),
        None => line.into(),
    };
    mouse_area(line)
        .on_press(Message::Compare(CompareMsg::Focus {
            tab: id,
            cursor: Some(index),
        }))
        .on_double_click(Message::Compare(CompareMsg::Open { tab: id, index }))
        .on_right_press(Message::Compare(CompareMsg::ContextMenu { tab: id, index }))
        .into()
}

fn foot(id: TabId, f: Footer, compact: bool) -> Element<'static, Message> {
    let p = palette();
    let button_of = |label: &'static str, on: Option<CompareMsg>| {
        let enabled = on.is_some();
        widgets::outlined(
            button(
                text(label)
                    .size(theme::TEXT_XS)
                    .wrapping(text::Wrapping::None),
            )
            .padding(Padding::from([theme::SPACE_3XS, theme::SPACE_XS]))
            .on_press_maybe(on.map(Message::Compare)),
            enabled,
        )
    };
    let mut bar = row![
        container(
            text(f.text)
                .size(theme::TEXT_SM)
                .color(p.muted)
                .wrapping(text::Wrapping::None)
        )
        .width(Length::Fill)
        .clip(true),
    ]
    .spacing(theme::SPACE_XS)
    .align_y(Alignment::Center);
    for act in f.actions {
        bar = bar.push(match act {
            Act::Stop => button_of(strings::compare_stop(), Some(CompareMsg::Stop { tab: id })),
            Act::Continue => button_of(
                strings::compare_continue(),
                Some(CompareMsg::Continue { tab: id }),
            ),
            Act::Again => button_of(
                strings::compare_again(),
                Some(CompareMsg::Start { tab: id }),
            ),
        });
    }
    let (up, down) = if compact {
        (strings::upload(), strings::download())
    } else {
        (
            strings::compare_upload_selected(),
            strings::compare_download_selected(),
        )
    };
    bar = bar
        .push(button_of(
            up,
            f.upload.then_some(CompareMsg::UploadSelected { tab: id }),
        ))
        .push(button_of(
            down,
            f.download
                .then_some(CompareMsg::DownloadSelected { tab: id }),
        ));
    container(bar)
        .height(QUEUE_HEAD_HEIGHT)
        .padding(Padding::ZERO.left(theme::SPACE_MD).right(theme::SPACE_SM))
        .align_y(Alignment::Center)
        .into()
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use hoprs_core::{EntryKind, Error, ErrorKind, RemoteEntry, RemotePath};
    use hoprs_remote::{CompareSide, Difference, Status};

    use super::*;
    use crate::app::compare::{CompareState, Group, Phase, Row, RowState};
    use crate::i18n::{Language, set_language};
    use crate::ui::icons::Icon;

    #[test]
    fn group_icons() {
        assert_eq!(group_icon(Group::OnlyLocal), Icon::Desktop);
        assert_eq!(group_icon(Group::OnlyRemote), Icon::HardDrives);
        assert_eq!(group_icon(Group::Different), Icon::ArrowsLeftRight);
        assert_eq!(group_icon(Group::Unreadable), Icon::WarningCircle);
    }

    struct RestoreEnglish;
    impl Drop for RestoreEnglish {
        fn drop(&mut self) {
            set_language(Language::En);
        }
    }

    fn entry(name: &str, kind: EntryKind, size: u64, secs: u64) -> RemoteEntry {
        RemoteEntry {
            name: name.into(),
            path: RemotePath::new(&format!("/x/{name}")).unwrap(),
            kind,
            size,
            modified: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(secs)),
            mode: None,
        }
    }

    fn diff(path: &str, status: Status, local: Option<u64>, remote: Option<u64>) -> Difference {
        Difference {
            path: path.into(),
            kind: EntryKind::File,
            status,
            local: local.map(|s| entry(path, EntryKind::File, s, 1_000)),
            remote: remote.map(|s| entry(path, EntryKind::File, s, 2_000)),
            entries_inside: None,
        }
    }

    fn row(diff: Difference) -> Row {
        Row {
            diff,
            selected: false,
            state: RowState::Idle,
        }
    }

    fn state(rows: Vec<Row>) -> CompareState {
        CompareState {
            roots: Some((
                RemotePath::new("/home/me/proj").unwrap(),
                RemotePath::new("/srv/app").unwrap(),
            )),
            phase: Phase::Done { partial: false },
            rows,
            identical: 1204,
            ..CompareState::default()
        }
    }

    fn mixed() -> CompareState {
        state(vec![
            row(diff(
                "src/app.js",
                Status::LocalNewer,
                Some(12 * 1024),
                Some(11 * 1024),
            )),
            row(diff("docs/new.md", Status::OnlyLocal, Some(1024), None)),
            row(diff(
                "public/old.css",
                Status::OnlyRemote,
                None,
                Some(8 * 1024),
            )),
            row(diff(".env", Status::RemoteNewer, Some(1024), Some(2048))),
            row(diff(
                "src/components/nav.js",
                Status::OnlyLocal,
                Some(3 * 1024),
                None,
            )),
        ])
    }

    #[test]
    fn head_shows_both_roots_and_the_toggle() {
        let s = mixed();
        let head = head(&s, true);
        assert_eq!(head.title, "Differences");
        assert_eq!(head.local, "/home/me/proj");
        assert_eq!(head.remote, "/srv/app");
        assert!(head.auto);
        assert!(!super::head(&s, false).auto);
    }

    #[test]
    fn narrow_heads_keep_the_last_folder() {
        assert_eq!(short_root("/home/me/proj"), "…/proj");
        assert_eq!(short_root("/srv/app/"), "…/app");
        assert_eq!(short_root("/srv"), "/srv");
        assert_eq!(short_root("/"), "/");
    }

    #[test]
    fn groups_come_in_order_with_counts_and_empty_ones_are_hidden() {
        let s = mixed();
        let lines = lines(&s);
        let shown: Vec<String> = lines
            .iter()
            .map(|l| match l {
                Line::Header(g) => group_title(&s, *g),
                Line::Row(i) => s.rows[*i].diff.path.clone(),
            })
            .collect();
        assert_eq!(
            shown,
            [
                "Only on this computer (2)",
                "docs/new.md",
                "src/components/nav.js",
                "Only on the server (1)",
                "public/old.css",
                "Different (2)",
                "src/app.js",
                ".env",
            ]
        );
        let only_remote = state(vec![row(diff("a", Status::OnlyRemote, None, Some(1)))]);
        assert_eq!(lines_len_headers(&only_remote), 1);
        assert_eq!(line_of(&s, 2), 4);
    }

    fn lines_len_headers(s: &CompareState) -> usize {
        lines(s)
            .iter()
            .filter(|l| matches!(l, Line::Header(_)))
            .count()
    }

    #[test]
    fn row_texts_per_status() {
        let _r = RestoreEnglish;
        let t = |d: Difference| row_text(&row(d), 2026);
        let a = t(diff("a.txt", Status::OnlyLocal, Some(3 * 1024), None));
        assert_eq!(
            (a.path.as_str(), a.status.as_str(), a.sizes.as_str()),
            ("a.txt", "", "3.0 KB")
        );
        let b = t(diff("b.txt", Status::OnlyRemote, None, Some(8 * 1024)));
        assert_eq!(b.sizes, "8.0 KB");
        let c = t(diff(
            "c",
            Status::LocalNewer,
            Some(12 * 1024),
            Some(11 * 1024),
        ));
        assert_eq!(c.status, "Local is newer");
        assert_eq!(c.sizes, "12 KB · 11 KB");
        let d = t(diff("d", Status::RemoteNewer, Some(1024), Some(2048)));
        assert_eq!(d.status, "Server is newer");
        assert_eq!(d.sizes, "1.0 KB · 2.0 KB");
        let e = t(diff("e", Status::SizeDiffers, Some(1), Some(2)));
        assert_eq!(e.status, "Size differs");
        let mut k = diff("k", Status::KindDiffers, Some(1), Some(0));
        k.remote = Some(entry("k", EntryKind::Dir, 0, 1));
        assert_eq!(t(k).status, "File vs folder");
        let mut dir = diff("src", Status::OnlyLocal, None, None);
        dir.kind = EntryKind::Dir;
        dir.local = Some(entry("src", EntryKind::Dir, 0, 1));
        let r = t(dir);
        assert_eq!(r.path, "src/");
        assert_eq!(r.sizes, "");
        // Dates are the tooltip.
        assert!(
            c.tip
                .as_deref()
                .is_some_and(|t| t.contains("This computer:") && t.contains("Server:"))
        );
    }

    #[test]
    fn unreadable_row_shows_the_translated_error() {
        let _r = RestoreEnglish;
        let error = Error::coded(ErrorKind::Permission, "session-closed", "closed");
        let mut d = diff("secret", Status::OnlyLocal, None, None);
        d.kind = EntryKind::Dir;
        d.status = Status::Unreadable {
            side: CompareSide::Remote,
            error,
        };
        d.local = None;
        let text = row_text(&row(d.clone()), 2026);
        assert_eq!(text.path, "secret/");
        assert_eq!(
            text.status,
            format!(
                "Could not read: {}",
                crate::ui::strings::error(&Error::coded(
                    ErrorKind::Permission,
                    "session-closed",
                    "closed"
                ))
            )
        );
        set_language(Language::PtBr);
        assert!(
            row_text(&row(d), 2026)
                .status
                .starts_with("Não foi possível ler: ")
        );
    }

    #[test]
    fn row_state_replaces_the_status() {
        let _r = RestoreEnglish;
        let mut r = row(diff("a", Status::LocalNewer, Some(1), Some(2)));
        r.state = RowState::Queued(hoprs_transfer::TransferId(1));
        assert_eq!(row_text(&r, 2026).status, "Queued");
        r.state = RowState::Done;
        assert_eq!(
            row_text(&r, 2026).status,
            crate::ui::strings::transfer_done()
        );
        r.state = RowState::Skipped("exists".into());
        assert_eq!(
            row_text(&r, 2026).status,
            crate::ui::strings::skipped("exists")
        );
    }

    #[test]
    fn footer_per_phase() {
        let _r = RestoreEnglish;
        let mut s = mixed();
        let f = footer(&s);
        assert_eq!(f.text, "1,204 identical");
        assert!(f.actions.is_empty());
        assert!(f.all_identical.is_none());

        s.phase = Phase::Running { seen: (1000, 2120) };
        let f = footer(&s);
        assert_eq!(f.text, "Comparing… 3,120 items");
        assert_eq!(f.actions, [Act::Stop]);

        s.phase = Phase::Paused {
            seen: (10_000, 4_000),
        };
        let f = footer(&s);
        assert_eq!(f.text, "14,000 items seen so far; the list is partial.");
        assert_eq!(f.actions, [Act::Continue, Act::Stop]);

        s.phase = Phase::Done { partial: true };
        let f = footer(&s);
        assert_eq!(f.text, "Stopped; the list is partial.");
        assert_eq!(f.actions, [Act::Again]);

        s.phase = Phase::Stopped(Error::coded(ErrorKind::Network, "session-closed", "x"));
        let f = footer(&s);
        assert!(f.text.starts_with("Comparison stopped: "));
        assert_eq!(f.actions, [Act::Again]);

        let identical = state(Vec::new());
        assert_eq!(
            footer(&identical).all_identical.as_deref(),
            Some("The two folders are identical (1,204 items)")
        );
        let mut one = state(Vec::new());
        one.identical = 1;
        assert_eq!(footer(&one).text, "1 identical");
    }

    #[test]
    fn debouncing_keeps_the_previous_summary() {
        let mut s = mixed();
        s.phase = Phase::Debouncing;
        let f = footer(&s);
        assert_eq!(f.text, "1,204 identical");
        assert!(f.actions.is_empty());
    }

    #[test]
    fn upload_and_download_follow_the_selection() {
        let mut s = mixed();
        let f = footer(&s);
        assert!(!f.upload && !f.download);
        // Only local (index 1): upload only.
        s.rows[1].selected = true;
        let f = footer(&s);
        assert!(f.upload && !f.download);
        // Only remote added: both.
        s.rows[2].selected = true;
        let f = footer(&s);
        assert!(f.upload && f.download);
        s.rows[1].selected = false;
        let f = footer(&s);
        assert!(!f.upload && f.download);
        // A different file can go either way.
        s.rows[2].selected = false;
        s.rows[0].selected = true;
        let f = footer(&s);
        assert!(f.upload && f.download);
        // File vs folder and unreadable folders go nowhere.
        let mut k = diff("k", Status::KindDiffers, Some(1), Some(0));
        k.remote = Some(entry("k", EntryKind::Dir, 0, 1));
        let mut s = state(vec![row(k)]);
        s.rows[0].selected = true;
        let f = footer(&s);
        assert!(!f.upload && !f.download);
    }

    #[test]
    fn group_checkbox_reflects_its_rows() {
        let mut s = mixed();
        assert!(!group_checked(&s, Group::OnlyLocal));
        s.rows[1].selected = true;
        assert!(!group_checked(&s, Group::OnlyLocal));
        s.rows[4].selected = true;
        assert!(group_checked(&s, Group::OnlyLocal));
    }

    #[test]
    fn visibility() {
        let mut s = CompareState::default();
        assert!(!s.visible());
        s.phase = Phase::Running { seen: (0, 0) };
        assert!(s.visible());
        s.hidden = true;
        assert!(!s.visible());
        s.hidden = false;
        s.phase = Phase::Debouncing;
        assert!(!s.visible(), "a first wait shows nothing");
        s.roots = Some((
            RemotePath::new("/a").unwrap(),
            RemotePath::new("/b").unwrap(),
        ));
        assert!(s.visible(), "a refresh keeps the last result on screen");
    }

    #[test]
    fn panel_labels_in_portuguese() {
        let _r = RestoreEnglish;
        set_language(Language::PtBr);
        let mut s = mixed();
        assert_eq!(head(&s, true).title, "Diferenças");
        let titles: Vec<String> = Group::ORDER.iter().map(|g| group_title(&s, *g)).collect();
        assert_eq!(titles[0], "Só neste computador (2)");
        assert_eq!(titles[1], "Só no servidor (1)");
        assert_eq!(titles[2], "Diferentes (2)");
        let t = |st| row_text(&row(diff("x", st, Some(1024), Some(2048))), 2026);
        assert_eq!(t(Status::LocalNewer).status, "Local mais novo");
        assert_eq!(t(Status::RemoteNewer).status, "Servidor mais novo");
        assert_eq!(t(Status::SizeDiffers).status, "Tamanho diferente");
        assert_eq!(t(Status::KindDiffers).status, "Arquivo × pasta");
        assert_eq!(t(Status::LocalNewer).sizes, "1,0 KB · 2,0 KB");
        assert_eq!(footer(&s).text, "1.204 idênticos");
        assert_eq!(
            crate::ui::strings::compare_upload_selected(),
            "Fazer upload dos selecionados"
        );
        assert_eq!(
            crate::ui::strings::compare_download_selected(),
            "Fazer download dos selecionados"
        );
        s.phase = Phase::Running { seen: (1000, 2120) };
        assert_eq!(footer(&s).text, "Comparando… 3.120 itens");
        s.phase = Phase::Paused { seen: (1, 1) };
        assert_eq!(footer(&s).actions, [Act::Continue, Act::Stop]);
        assert_eq!(crate::ui::strings::compare_continue(), "Comparar tudo");
        assert_eq!(crate::ui::strings::compare_again(), "Comparar de novo");
        assert_eq!(
            footer(&state(Vec::new())).all_identical.as_deref(),
            Some("As duas pastas são idênticas (1.204 itens)")
        );
    }

    #[test]
    fn summary_counts_groups() {
        let mut s = mixed();
        s.rows.push(row(diff(
            "x",
            Status::Unreadable {
                side: CompareSide::Local,
                error: Error::coded(ErrorKind::Network, "session-closed", "x"),
            },
            None,
            None,
        )));
        let sm = summary(&s);
        assert_eq!((sm.only_local, sm.only_remote, sm.different), (2, 1, 2));
        assert_eq!(sm.progress, None);
    }

    #[test]
    fn summary_shows_progress_while_running() {
        let _r = RestoreEnglish;
        let mut s = state(Vec::new());
        s.phase = Phase::Running { seen: (1000, 2120) };
        assert_eq!(
            summary(&s).progress.as_deref(),
            Some("Comparing… 3,120 items")
        );
        s.phase = Phase::Paused { seen: (1, 1) };
        assert!(summary(&s).progress.is_some());
        s.phase = Phase::Debouncing;
        assert_eq!(summary(&s).progress.as_deref(), Some("1,204 identical"));
        // A run in flight says so even with rows already found.
        let mut m = mixed();
        m.phase = Phase::Running { seen: (1, 1) };
        assert_eq!(summary(&m).progress.as_deref(), Some("Comparing… 2 items"));
        m.phase = Phase::Paused { seen: (1, 1) };
        assert_eq!(
            summary(&m).progress.as_deref(),
            Some("2 items seen so far; the list is partial.")
        );
        m.phase = Phase::Done { partial: true };
        assert_eq!(
            summary(&m).progress.as_deref(),
            Some("Stopped; the list is partial.")
        );
        m.phase = Phase::Stopped(Error::coded(ErrorKind::Network, "session-closed", "x"));
        assert!(
            summary(&m)
                .progress
                .is_some_and(|t| t.starts_with("Comparison stopped: "))
        );
        // Finished: the counts when something differs, the identical text otherwise.
        m.phase = Phase::Done { partial: false };
        assert_eq!(summary(&m).progress, None);
        let mut clean = state(Vec::new());
        clean.phase = Phase::Done { partial: false };
        assert_eq!(summary(&clean).progress.as_deref(), Some("1,204 identical"));
    }

    #[test]
    fn summary_in_portuguese() {
        let _r = RestoreEnglish;
        set_language(Language::PtBr);
        let one = crate::ui::strings::compare_summary_only_here(1);
        let two = crate::ui::strings::compare_summary_only_here(2);
        assert_eq!(one, "1 só aqui");
        assert_eq!(two, "2 só aqui");
        assert_eq!(
            crate::ui::strings::compare_summary_only_server(1),
            "1 só no servidor"
        );
        assert_eq!(
            crate::ui::strings::compare_summary_different(1),
            "1 diferente"
        );
        assert_eq!(
            crate::ui::strings::compare_summary_different(2),
            "2 diferentes"
        );
        set_language(Language::En);
        assert_eq!(
            crate::ui::strings::compare_summary_different(1),
            "1 different"
        );
        assert_eq!(
            crate::ui::strings::compare_summary_only_here(2),
            "2 only here"
        );
    }
}
