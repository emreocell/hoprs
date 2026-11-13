//! The technical log panel, under the transfer queue: a 36 px head (title, level
//! filter, text filter, line count, Copy all, Clear, Hide) and the lines in mono, a
//! virtual list. Closed, it takes no room; the status bar's "Log" opens it.

use iced::widget::{Id, button, column, container, row, rule, space, text, text_input};
use iced::{Alignment, Color, Element, Length, Padding};
use tracing::Level;

use crate::app::log::{LogRow, at_end};
use crate::app::{App, LogLevel, LogMsg, Message};
use crate::ui::layout::{LOG_BODY_HEIGHT, QUEUE_HEAD_HEIGHT};
use crate::ui::strings;
use crate::ui::theme::{self, palette};
use crate::ui::virtual_list::{self, ROW_HEIGHT};

/// The level column: fits `ERROR`.
const LEVEL_WIDTH: f32 = 48.0;
const TIME_WIDTH: f32 = 96.0;
const FILTER_WIDTH: f32 = 200.0;
const FILTER_HEIGHT: f32 = 24.0;

/// The id of the log's list, for following the newest line.
pub fn scroll_id() -> Id {
    Id::new("log")
}

fn msg(m: LogMsg) -> Message {
    Message::Log(m)
}

fn level_label(level: LogLevel) -> &'static str {
    match level {
        LogLevel::Error => strings::LOG_ERROR,
        LogLevel::Warn => strings::LOG_WARN,
        LogLevel::Info => strings::LOG_INFO,
        LogLevel::Debug => strings::LOG_DEBUG,
    }
}

/// The panel, or nothing while it is closed.
pub fn view(app: &App) -> Option<Element<'_, Message>> {
    let log = &app.log;
    if !log.open {
        return None;
    }
    let p = palette();
    let small = |label: &'static str, on_press: Option<Message>| {
        button(
            text(label)
                .size(theme::TEXT_XS)
                .wrapping(text::Wrapping::None),
        )
        .padding(Padding::from([theme::SPACE_3XS, theme::SPACE_XS]))
        .style(theme::button_text)
        .on_press_maybe(on_press)
    };
    let mut levels = row![].spacing(theme::SPACE_2XS);
    for level in LogLevel::ALL {
        levels = levels.push(
            button(
                text(level_label(level))
                    .size(theme::TEXT_XS)
                    .wrapping(text::Wrapping::None),
            )
            .padding(Padding::from([theme::SPACE_3XS, theme::SPACE_XS]))
            .style(theme::segment(log.level == level))
            .on_press(msg(LogMsg::Level(level))),
        );
    }
    let filter = container(
        text_input(strings::FILTER_LOG, &log.query)
            .on_input(|q| msg(LogMsg::Filter(q)))
            .padding(Padding::from([0.0, theme::SPACE_XS]))
            .size(theme::TEXT_XS)
            .line_height(iced::Pixels(FILTER_HEIGHT - 2.0 * theme::HAIRLINE))
            .style(theme::search),
    )
    .width(FILTER_WIDTH);
    let has_lines = log.shown_len() > 0;
    let head = row![
        text(strings::LOG)
            .font(theme::INTER_SEMIBOLD)
            .wrapping(text::Wrapping::None),
        levels,
        filter,
        space::horizontal(),
        text(strings::log_lines(log.shown_len()))
            .size(theme::TEXT_XS)
            .color(p.muted)
            .wrapping(text::Wrapping::None),
        row![
            small(strings::COPY_ALL, has_lines.then_some(msg(LogMsg::CopyAll))),
            small(
                strings::CLEAR,
                (!log.is_empty()).then_some(msg(LogMsg::Clear))
            ),
            small(strings::QUEUE_HIDE, Some(msg(LogMsg::Toggle))),
        ]
        .spacing(theme::SPACE_2XS)
        .align_y(Alignment::Center),
    ]
    .spacing(theme::SPACE_MD)
    .align_y(Alignment::Center)
    .height(QUEUE_HEAD_HEIGHT)
    .padding(Padding::ZERO.left(theme::SPACE_MD).right(theme::SPACE_SM));

    let body: Element<'_, Message> = if has_lines {
        virtual_list::view(
            scroll_id(),
            log.shown_len(),
            log.offset_y,
            |viewport| {
                let offset_y = viewport.absolute_offset().y;
                msg(LogMsg::Scrolled {
                    offset_y,
                    at_end: at_end(
                        offset_y,
                        viewport.bounds().height,
                        viewport.content_bounds().height,
                    ),
                })
            },
            move |i| match log.shown_row(i) {
                Some(row) => log_row(row),
                None => space().height(ROW_HEIGHT).into(),
            },
        )
    } else {
        let note = if log.is_empty() {
            strings::NO_LOG_LINES
        } else {
            strings::NO_LOG_MATCHES
        };
        container(text(note).color(p.muted))
            .padding(Padding::from([theme::SPACE_SM, theme::SPACE_MD]))
            .width(Length::Fill)
            .into()
    };

    Some(
        column![
            rule::horizontal(theme::HAIRLINE).style(theme::rule_strong),
            head,
            rule::horizontal(theme::HAIRLINE).style(theme::rule_1px),
            container(body).width(Length::Fill).height(LOG_BODY_HEIGHT),
        ]
        .width(Length::Fill)
        .into(),
    )
}

/// One 30 px line: time, level, target muted; the message in ink. All mono, one line,
/// clipped.
fn log_row<'a>(row: &'a LogRow) -> Element<'a, Message> {
    let p = palette();
    let line = &row.line;
    let level_color: Color = match line.level {
        Level::ERROR | Level::WARN => p.error,
        Level::INFO => p.ink_2,
        _ => p.muted,
    };
    let mono = |label: &'a str, color: Color| {
        text(label)
            .font(theme::MONO)
            .size(theme::TEXT_MONO)
            .color(color)
            .wrapping(text::Wrapping::None)
    };
    container(
        row![
            container(mono(&row.time, p.muted)).width(TIME_WIDTH),
            container(mono(line.level.as_str(), level_color)).width(LEVEL_WIDTH),
            mono(&line.target, p.muted),
            mono(&line.message, p.ink),
        ]
        .spacing(theme::SPACE_SM)
        .align_y(Alignment::Center),
    )
    .height(ROW_HEIGHT)
    .width(Length::Fill)
    .padding(Padding::ZERO.left(theme::SPACE_MD).right(theme::SPACE_MD))
    .align_y(Alignment::Center)
    .clip(true)
    .into()
}
