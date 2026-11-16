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

use hoprs_transfer::{Direction, ResolveAction};

use crate::app::queue::{QueueRow, RowStatus};
use crate::app::{App, Message, QueueMsg, QueueTab, RowAction};
use crate::ui::file_actions::error_bar;
use crate::ui::fit;
use crate::ui::icons::{self, Icon};
use crate::ui::layout::{QUEUE_HEAD_HEIGHT, QUEUE_MAX_HEIGHT, UNDERLINE};
use crate::ui::strings;
use crate::ui::theme::{self, palette};
use crate::ui::virtual_list::{self, ROW_HEIGHT};
use crate::ui::widgets;

/// Column widths (mockup: `.bar { width: 180px }`).
const SITE_WIDTH: f32 = 128.0;
const BAR_WIDTH: f32 = 180.0;
const PERCENT_WIDTH: f32 = 48.0;
const SPEED_WIDTH: f32 = 96.0;
const STATUS_WIDTH: f32 = BAR_WIDTH + PERCENT_WIDTH + SPEED_WIDTH + 2.0 * theme::SPACE_MD;
/// The row actions: the icon ones need only about 3 x 22 px, but a conflict row keeps text
/// answers (Skip, Overwrite, Resume) beside its Cancel, so the column keeps room for those
/// and every row lines up.
const ACTIONS_WIDTH: f32 = 236.0;
/// The rows' part of the 210 px region: the rest is the head and its two rules.
const BODY_MAX_HEIGHT: f32 = QUEUE_MAX_HEIGHT - QUEUE_HEAD_HEIGHT - 2.0 * theme::HAIRLINE;
/// The 4 px progress bar.
const BAR_GIRTH: f32 = 4.0;

/// The arrow that starts a row: up for an upload, down for a download.
pub fn direction_icon(direction: Direction) -> Icon {
    match direction {
        Direction::Upload => Icon::ArrowUp,
        Direction::Download => Icon::ArrowDown,
    }
}

/// The icon of a row action; the conflict answers keep their text buttons.
pub fn action_icon(action: RowAction) -> Option<Icon> {
    match action {
        RowAction::Pause => Some(Icon::Pause),
        RowAction::Resume => Some(Icon::Play),
        RowAction::Cancel => Some(Icon::X),
        RowAction::Retry => Some(Icon::ArrowsClockwise),
        RowAction::Dismiss => Some(Icon::Check),
        RowAction::Resolve(_) => None,
    }
}

/// The id of the queue's list, for its scroll offset.
fn scroll_id() -> Id {
    Id::new("queue")
}

/// Mouse button releases and the pointer leaving the window (they end a drag between
/// panes) and files dragged from the system
/// file manager.
pub fn pointer_message(event: Event, _: Status, _: window::Id) -> Option<Message> {
    match event {
        // Any button: a release the drop doesn't use still ends the drag.
        Event::Mouse(mouse::Event::ButtonReleased(_)) => Some(Message::DragEnd),
        Event::Mouse(mouse::Event::CursorLeft) => Some(Message::DragCancel),
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
        strings::queue_active(),
        strings::queue_queued(),
        strings::queue_failed(),
        strings::queue_done(),
    ];
    // A narrow window: short labels, no worker count, the session policy as a tag in
    // place of the title, tighter gaps.
    let fit = fit::queue_head(app.window_width, queue.session_policy().is_some());
    let compact = fit.compact;
    let mut head = row![fit.title.then(|| {
        text(strings::transfers())
            .font(theme::INTER_SEMIBOLD)
            .wrapping(text::Wrapping::None)
    })]
    .spacing(if compact {
        theme::SPACE_MD
    } else {
        theme::SPACE_LG
    })
    .align_y(Alignment::Center);
    for ((tab, label), count) in QueueTab::ALL.into_iter().zip(labels).zip(counts) {
        let active = queue.tab == tab;
        head = head.push(with_underline(
            button(text(strings::queue_tab(label, count)).wrapping(text::Wrapping::None))
                .padding(0)
                .style(theme::queue_tab(active))
                .on_press(Message::Queue(QueueMsg::SelectTab(tab)))
                .into(),
            active,
        ));
    }
    let toggle = if app.queue_open {
        strings::queue_hide()
    } else {
        strings::queue_show()
    };
    let head = head
        .push(space::horizontal())
        // An answer "to all" became the engine's policy for this run: say so.
        .push(
            queue
                .session_policy()
                .zip(fit.policy)
                .map(|(policy, note)| match note {
                    fit::PolicyNote::Full => text(strings::session_policy(policy))
                        .size(theme::TEXT_XS)
                        .color(p.muted)
                        .wrapping(text::Wrapping::None),
                    fit::PolicyNote::Tag => text(strings::session_policy_tag(policy))
                        .font(theme::MONO)
                        .size(theme::TEXT_TAG)
                        .color(p.muted)
                        .wrapping(text::Wrapping::None),
                }),
        )
        .push(
            row![
                text(strings::speed(queue.total_speed()))
                    .font(theme::INTER_SEMIBOLD)
                    .wrapping(text::Wrapping::None),
            ]
            .push((!compact).then(|| {
                text(strings::total_workers(queue.workers))
                    .color(p.muted)
                    .wrapping(text::Wrapping::None)
            }))
            .align_y(Alignment::Center),
        )
        .push(
            row![
                widgets::icon_button(
                    Icon::Broom,
                    strings::clear_finished(),
                    None,
                    queue
                        .has_finished()
                        .then_some(Message::Queue(QueueMsg::ClearFinished)),
                ),
                widgets::icon_button(
                    if app.queue_open {
                        Icon::CaretDown
                    } else {
                        Icon::CaretUp
                    },
                    toggle,
                    None,
                    Some(Message::ToggleQueue),
                ),
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
            return note(text(strings::queue_loading()).color(p.muted).into());
        }
        return note(
            column![
                text(strings::no_transfers()),
                text(strings::no_transfers_hint())
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
            QueueTab::Active => strings::nothing_active(),
            QueueTab::Queued => strings::nothing_queued(),
            QueueTab::Failed => strings::nothing_failed(),
            QueueTab::Done => strings::nothing_done(),
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
    let path = match item.spec.direction {
        Direction::Upload => &item.spec.local,
        Direction::Download => &item.spec.remote,
    };
    // The name first (a long folder must not hide it), then the source folder, muted.
    let mono = |label: &'a str| {
        text(label)
            .font(theme::MONO)
            .size(theme::TEXT_MONO)
            .wrapping(text::Wrapping::None)
    };
    let file_name = path.file_name().unwrap_or(path.as_str());
    // What the name column has left of the center area, beside the arrow.
    let room = (fit::center_width(app.window_width)
        - SITE_WIDTH
        - STATUS_WIDTH
        - ACTIONS_WIDTH
        - 2.0 * theme::SPACE_MD
        - icons::ROW
        - theme::SPACE_XS)
        .max(0.0);
    let natural = file_name.chars().count() as f32 * theme::TEXT_MONO * fit::MONO_ADVANCE;
    let mut name = row![
        icons::icon(direction_icon(item.spec.direction), icons::ROW).color(p.muted),
        widgets::fitted(
            file_name,
            (natural + 1.0).min(room),
            theme::TEXT_MONO,
            theme::MONO
        ),
    ]
    .align_y(Alignment::Center);
    if item.is_dir == Some(true) {
        name = name.push(mono(strings::path_separator()).color(p.muted));
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
        .map_or(strings::site_missing(), |s| s.name.as_str());
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
    let status: Element<'a, Message> = match app.queue.status(row) {
        RowStatus::Error(error) => error_text(crate::ui::strings::error(error)),
        RowStatus::Progress {
            fraction,
            bytes_per_sec,
        } => row![
            progress_bar(0.0..=1.0, fraction)
                .length(BAR_WIDTH)
                .girth(BAR_GIRTH)
                .style(theme::progress),
            container(text(strings::percent(fraction)).size(theme::TEXT_SM))
                .width(PERCENT_WIDTH)
                .align_x(Alignment::End),
            container(
                text(strings::speed(bytes_per_sec))
                    .size(theme::TEXT_SM)
                    .wrapping(text::Wrapping::None),
            )
            .width(SPEED_WIDTH)
            .align_x(Alignment::End),
        ]
        .spacing(theme::SPACE_MD)
        .align_y(Alignment::Center)
        .into(),
        RowStatus::Bytes {
            done,
            bytes_per_sec,
        } => text(strings::bytes_and_speed(done, bytes_per_sec))
            .size(theme::TEXT_SM)
            .wrapping(text::Wrapping::None)
            .into(),
        RowStatus::Transferring => muted(strings::transferring().to_owned()),
        RowStatus::Starting => muted(strings::starting().to_owned()),
        RowStatus::Waiting => muted(strings::waiting_for_worker().to_owned()),
        RowStatus::Paused => muted(strings::paused().to_owned()),
        RowStatus::Conflict => row![
            text(strings::file_exists()).size(theme::TEXT_SM),
            checkbox(app.queue.apply_all(id))
                .label(strings::apply_to_all())
                .text_size(theme::TEXT_XS)
                .size(theme::TEXT_SM)
                .style(theme::checkbox)
                .on_toggle(move |_| Message::Queue(QueueMsg::ToggleApplyAll(id))),
        ]
        .spacing(theme::SPACE_SM)
        .align_y(Alignment::Center)
        .into(),
        RowStatus::Done => muted(strings::transfer_done().to_owned()),
        RowStatus::Skipped(reason) => muted(strings::skipped(reason)),
        RowStatus::Dismissed => muted(strings::dismissed().to_owned()),
        RowStatus::Cancelled => muted(strings::cancelled().to_owned()),
    };
    let mut actions = row![].spacing(theme::SPACE_3XS).align_y(Alignment::Center);
    for action in app.queue.actions(row) {
        let label = match action {
            RowAction::Pause => strings::pause(),
            RowAction::Resume | RowAction::Resolve(ResolveAction::Resume) => strings::resume(),
            RowAction::Cancel => strings::cancel(),
            RowAction::Retry => strings::retry(),
            RowAction::Dismiss => strings::dismiss(),
            RowAction::Resolve(ResolveAction::Skip) => strings::skip(),
            RowAction::Resolve(ResolveAction::Overwrite) => strings::overwrite(),
        };
        let press = Message::Queue(QueueMsg::Action(id, action));
        actions = actions.push(match action_icon(action) {
            Some(icon) => widgets::icon_button(icon, label, None, Some(press)),
            None => button(text(label).size(theme::TEXT_XS))
                .padding(Padding::from([theme::SPACE_3XS, theme::SPACE_XS]))
                .style(theme::queue_action)
                .on_press(press)
                .into(),
        });
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

fn error_text<'a>(message: String) -> Element<'a, Message> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_icons_per_direction_and_action() {
        assert_eq!(direction_icon(Direction::Upload), Icon::ArrowUp);
        assert_eq!(direction_icon(Direction::Download), Icon::ArrowDown);
        assert_eq!(action_icon(RowAction::Pause), Some(Icon::Pause));
        assert_eq!(action_icon(RowAction::Resume), Some(Icon::Play));
        assert_eq!(action_icon(RowAction::Cancel), Some(Icon::X));
        assert_eq!(action_icon(RowAction::Retry), Some(Icon::ArrowsClockwise));
        assert_eq!(action_icon(RowAction::Dismiss), Some(Icon::Check));
        // The conflict answers keep their text buttons.
        for answer in [
            ResolveAction::Skip,
            ResolveAction::Overwrite,
            ResolveAction::Resume,
        ] {
            assert_eq!(action_icon(RowAction::Resolve(answer)), None);
        }
    }
}
