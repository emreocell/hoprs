//! The transfer queue (mockup v2 `.queue`): the head with the Active / Queued / Failed /
//! Done tabs and their counts, the total speed and the workers, "Clear finished" and
//! Hide / Show; under it, when open, the rows of the selected tab. A row is the direction
//! arrow and the path in mono, the site, then by state the 4 px bar with percent and speed,
//! "Waiting for a worker", the inline conflict choice, or the error in red; its actions on
//! the right. Nothing here is a modal.

use iced::event::Status;
use iced::mouse;
use iced::widget::{Id, button, checkbox, column, container, progress_bar, row, rule, space, text};
use iced::{Alignment, Element, Event, Length, Padding, window};

use hoprs_transfer::{Direction, ResolveAction, TransferState};

use crate::app::queue::QueueRow;
use crate::app::{App, Message, QueueMsg, QueueTab, RowAction};
use crate::ui::file_actions::error_bar;
use crate::ui::layout::{QUEUE_HEAD_HEIGHT, QUEUE_MAX_HEIGHT, UNDERLINE};
use crate::ui::strings;
use crate::ui::theme::{self, palette};
use crate::ui::virtual_list::{self, ROW_HEIGHT};

/// Column widths (mockup: `.bar { width: 180px }`).
const SITE_WIDTH: f32 = 128.0;
const BAR_WIDTH: f32 = 180.0;
const PERCENT_WIDTH: f32 = 48.0;
const SPEED_WIDTH: f32 = 96.0;
const STATUS_WIDTH: f32 = BAR_WIDTH + PERCENT_WIDTH + SPEED_WIDTH + 2.0 * theme::SPACE_MD;
const ACTIONS_WIDTH: f32 = 236.0;
/// The rows' part of the 210 px region: the rest is the head and its two rules.
const BODY_MAX_HEIGHT: f32 = QUEUE_MAX_HEIGHT - QUEUE_HEAD_HEIGHT - 2.0 * theme::HAIRLINE;
/// The 4 px progress bar.
const BAR_GIRTH: f32 = 4.0;

/// The id of the queue's list, for its scroll offset.
fn scroll_id() -> Id {
    Id::new("queue")
}

/// Mouse button releases (they end a drag between panes) and files dragged from the system
/// file manager.
pub fn pointer_message(event: Event, _: Status, _: window::Id) -> Option<Message> {
    match event {
        Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => Some(Message::DragEnd),
        Event::Window(window::Event::FileHovered(_)) => Some(Message::FilesHovered(true)),
        Event::Window(window::Event::FilesHoveredLeft) => Some(Message::FilesHovered(false)),
        Event::Window(window::Event::FileDropped(path)) => Some(Message::FileDropped(path)),
        _ => None,
    }
}

/// The whole queue region.
pub fn view(app: &App) -> Element<'_, Message> {
    let queue = &app.queue;
    let p = palette();
    let counts = queue.counts();
    let labels = [
        strings::QUEUE_ACTIVE,
        strings::QUEUE_QUEUED,
        strings::QUEUE_FAILED,
        strings::QUEUE_DONE,
    ];
    let mut head = row![text(strings::TRANSFERS).font(theme::INTER_SEMIBOLD)]
        .spacing(theme::SPACE_LG)
        .align_y(Alignment::Center);
    for ((tab, label), count) in QueueTab::ALL.into_iter().zip(labels).zip(counts) {
        let active = queue.tab == tab;
        head = head.push(with_underline(
            button(text(strings::queue_tab(label, count)))
                .padding(0)
                .style(theme::queue_tab(active))
                .on_press(Message::Queue(QueueMsg::SelectTab(tab)))
                .into(),
            active,
        ));
    }
    let toggle = if app.queue_open {
        strings::QUEUE_HIDE
    } else {
        strings::QUEUE_SHOW
    };
    let head = head
        .push(space::horizontal())
        .push(
            row![
                text(strings::speed(queue.total_speed())).font(theme::INTER_SEMIBOLD),
                text(strings::total_workers(queue.workers)).color(p.muted),
            ]
            .align_y(Alignment::Center),
        )
        .push(
            row![
                button(text(strings::CLEAR_FINISHED).size(theme::TEXT_XS))
                    .padding(Padding::from([theme::SPACE_3XS, theme::SPACE_XS]))
                    .style(theme::button_text)
                    .on_press_maybe(
                        queue
                            .has_finished()
                            .then_some(Message::Queue(QueueMsg::ClearFinished)),
                    ),
                button(text(toggle).size(theme::TEXT_XS))
                    .padding(Padding::from([theme::SPACE_3XS, theme::SPACE_XS]))
                    .on_press(Message::ToggleQueue)
                    .style(theme::button_text),
            ]
            .spacing(theme::SPACE_2XS)
            .align_y(Alignment::Center),
        )
        .height(QUEUE_HEAD_HEIGHT)
        .padding(Padding::ZERO.left(theme::SPACE_MD).right(theme::SPACE_SM));

    let strong = rule::horizontal(theme::HAIRLINE).style(theme::rule_strong);
    let mut region = column![strong, head];
    if app.queue_open {
        region = region.push(rule::horizontal(theme::HAIRLINE).style(theme::rule_1px));
        if let Some(error) = &queue.error {
            region = region.push(error_bar(error, Message::Queue(QueueMsg::DismissError)));
        }
        region = region.push(body(app));
    }
    container(region)
        .width(Length::Fill)
        .max_height(QUEUE_MAX_HEIGHT)
        .into()
}

/// The rows of the selected tab, or what stands for them.
fn body(app: &App) -> Element<'_, Message> {
    let queue = &app.queue;
    let p = palette();
    let note = |line: Element<'static, Message>| -> Element<'static, Message> {
        container(line)
            .padding(Padding::from([theme::SPACE_SM, theme::SPACE_MD]))
            .into()
    };
    if queue.len() == 0 {
        if queue.resyncing() {
            return note(text(strings::QUEUE_LOADING).color(p.muted).into());
        }
        return note(
            column![
                text(strings::NO_TRANSFERS),
                text(strings::NO_TRANSFERS_HINT)
                    .size(theme::TEXT_XS)
                    .color(p.muted),
            ]
            .spacing(theme::SPACE_3XS)
            .into(),
        );
    }
    let rows = queue.rows_in(queue.tab);
    if rows.is_empty() {
        let empty = match queue.tab {
            QueueTab::Active => strings::NOTHING_ACTIVE,
            QueueTab::Queued => strings::NOTHING_QUEUED,
            QueueTab::Failed => strings::NOTHING_FAILED,
            QueueTab::Done => strings::NOTHING_DONE,
        };
        return note(text(empty).color(p.muted).into());
    }
    let count = rows.len();
    // As tall as the rows, up to the queue's cap (a `Fill` list would grow the region).
    let height = (count as f32 * ROW_HEIGHT).min(BODY_MAX_HEIGHT);
    let list = virtual_list::view(
        scroll_id(),
        count,
        queue.offset_y,
        |viewport| {
            Message::Queue(QueueMsg::Scrolled {
                offset_y: viewport.absolute_offset().y,
            })
        },
        move |i| match rows.get(i) {
            Some(row) => queue_row(app, row),
            None => space().height(ROW_HEIGHT).into(),
        },
    );
    container(list).height(height).into()
}

/// One 30 px row.
fn queue_row<'a>(app: &'a App, row: &'a QueueRow) -> Element<'a, Message> {
    let p = palette();
    let item = &row.item;
    let id = item.id;
    let (arrow, path) = match item.spec.direction {
        Direction::Upload => (strings::UPLOAD_ARROW, &item.spec.local),
        Direction::Download => (strings::DOWNLOAD_ARROW, &item.spec.remote),
    };
    // The name first (a long folder must not hide it), then the source folder, muted.
    let mono = |label: &'a str| {
        text(label)
            .font(theme::MONO)
            .size(theme::TEXT_MONO)
            .wrapping(text::Wrapping::None)
    };
    let mut name = row![
        text(arrow).size(theme::TEXT_SM).color(p.muted),
        mono(path.file_name().unwrap_or(path.as_str())),
    ]
    .align_y(Alignment::Center);
    if item.is_dir == Some(true) {
        name = name.push(mono(strings::PATH_SEPARATOR).color(p.muted));
    }
    let name = row![name.spacing(theme::SPACE_XS)]
        .push(path.parent().map(|parent| {
            text(parent.to_string())
                .font(theme::MONO)
                .size(theme::TEXT_MONO)
                .color(p.muted)
                .wrapping(text::Wrapping::None)
        }))
        .spacing(theme::SPACE_SM)
        .align_y(Alignment::Center);
    let site = app
        .site(&item.spec.site)
        .map_or(strings::SITE_MISSING, |s| s.name.as_str());
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
    let muted = |label: String| -> Element<'a, Message> {
        text(label)
            .size(theme::TEXT_SM)
            .color(p.muted)
            .wrapping(text::Wrapping::None)
            .into()
    };
    let status: Element<'a, Message> = match (app.queue.row_error(id), &item.state) {
        (Some(error), _) => error_text(&error.message),
        (None, TransferState::Active) => match (item.is_dir, row.fraction()) {
            (Some(false) | None, Some(fraction)) => row![
                progress_bar(0.0..=1.0, fraction)
                    .length(BAR_WIDTH)
                    .girth(BAR_GIRTH)
                    .style(theme::progress),
                container(text(strings::percent(fraction)).size(theme::TEXT_SM))
                    .width(PERCENT_WIDTH)
                    .align_x(Alignment::End),
                container(
                    text(strings::speed(row.bytes_per_sec))
                        .size(theme::TEXT_SM)
                        .wrapping(text::Wrapping::None),
                )
                .width(SPEED_WIDTH)
                .align_x(Alignment::End),
            ]
            .spacing(theme::SPACE_MD)
            .align_y(Alignment::Center)
            .into(),
            _ => muted(strings::STARTING.to_owned()),
        },
        (None, TransferState::Queued) => muted(strings::WAITING_FOR_WORKER.to_owned()),
        (None, TransferState::Paused) => muted(strings::PAUSED.to_owned()),
        (None, TransferState::AwaitingDecision(_)) => row![
            text(strings::FILE_EXISTS).size(theme::TEXT_SM),
            checkbox(app.queue.apply_all(id))
                .label(strings::APPLY_TO_ALL)
                .text_size(theme::TEXT_XS)
                .size(theme::TEXT_SM)
                .style(theme::checkbox)
                .on_toggle(move |_| Message::Queue(QueueMsg::ToggleApplyAll(id))),
        ]
        .spacing(theme::SPACE_SM)
        .align_y(Alignment::Center)
        .into(),
        (None, TransferState::Failed(error)) => error_text(&error.message),
        (None, TransferState::Done) => muted(strings::TRANSFER_DONE.to_owned()),
        (None, TransferState::Skipped(reason)) => muted(strings::skipped(reason)),
        (None, TransferState::Cancelled) => muted(strings::CANCELLED.to_owned()),
    };
    let skipped_failure =
        matches!(item.state, TransferState::Failed(_)) && app.queue.tab_of(row) == QueueTab::Done;
    let status = if skipped_failure {
        muted(strings::SKIPPED.to_owned())
    } else {
        status
    };
    let mut actions = row![].spacing(theme::SPACE_3XS).align_y(Alignment::Center);
    for action in app.queue.actions(row) {
        let label = match action {
            RowAction::Pause => strings::PAUSE,
            RowAction::Resume | RowAction::Resolve(ResolveAction::Resume) => strings::RESUME,
            RowAction::Cancel => strings::CANCEL,
            RowAction::Retry => strings::RETRY,
            RowAction::Skip | RowAction::Resolve(ResolveAction::Skip) => strings::SKIP,
            RowAction::Resolve(ResolveAction::Overwrite) => strings::OVERWRITE,
        };
        actions = actions.push(
            button(text(label).size(theme::TEXT_XS))
                .padding(Padding::from([theme::SPACE_3XS, theme::SPACE_XS]))
                .style(theme::queue_action)
                .on_press(Message::Queue(QueueMsg::Action(id, action))),
        );
    }
    row![
        cell(name.into(), Length::Fill, false),
        cell(muted(site.to_owned()), Length::Fixed(SITE_WIDTH), false),
        cell(status, Length::Fixed(STATUS_WIDTH), false),
        cell(actions.into(), Length::Fixed(ACTIONS_WIDTH), true),
    ]
    .height(ROW_HEIGHT)
    .into()
}

fn error_text<'a>(message: &'a str) -> Element<'a, Message> {
    text(message)
        .size(theme::TEXT_SM)
        .color(palette().error)
        .wrapping(text::Wrapping::None)
        .into()
}

/// A tab label over the 2 px accent underline of the active tab.
fn with_underline(label: Element<'_, Message>, active: bool) -> Element<'_, Message> {
    let underline = container(space()).width(Length::Fill).height(UNDERLINE);
    let underline = if active {
        underline.style(theme::underline)
    } else {
        underline
    };
    column![
        container(label)
            .height(Length::Fill)
            .align_y(Alignment::Center),
        underline,
    ]
    .width(Length::Shrink)
    .into()
}
