//! Small pieces shared by the forms: labelled field, inline error, in-place confirmation,
//! the indeterminate progress bar.

use hoprs_core::Error;
use iced::widget::{button, column, container, row, space, text};
use iced::{Alignment, Element, Length, Padding};

use crate::app::Message;
use crate::ui::strings;
use crate::ui::theme::{self, palette};

/// The in-place delete confirmation.
pub fn confirm<'a>(question: &'a str, yes: Message, no: Message) -> Element<'a, Message> {
    confirm_with(question, strings::delete(), yes, no)
}

/// An in-place confirmation whose action is `action` ("Delete", "Replace"…).
pub fn confirm_with<'a>(
    question: &'a str,
    action: &'a str,
    yes: Message,
    no: Message,
) -> Element<'a, Message> {
    container(
        row![
            text(question).size(theme::TEXT_SM).width(Length::Fill),
            button(text(action).font(theme::INTER_SEMIBOLD))
                .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_MD]))
                .style(theme::button_primary)
                .on_press(yes),
            button(text(strings::keep()))
                .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_XS]))
                .style(theme::button_text)
                .on_press(no),
        ]
        .spacing(theme::SPACE_XS)
        .align_y(Alignment::Center),
    )
    .padding(theme::SPACE_SM)
    .style(theme::outline_box)
    .into()
}

/// The backend's error, inline.
pub fn failure(error: Option<&Error>) -> Element<'_, Message> {
    match error {
        None => space().into(),
        Some(error) => {
            let mut lines = column![text(&error.message).color(palette().error)];
            if let Some(detail) = &error.detail {
                lines = lines.push(
                    text(detail)
                        .font(theme::MONO)
                        .size(theme::TEXT_MONO)
                        .color(palette().muted),
                );
            }
            lines.spacing(theme::SPACE_3XS).into()
        }
    }
}

/// A label above its control, with the inline error below.
pub fn field<'a>(
    label: &'a str,
    control: Element<'a, Message>,
    error: Option<&'a str>,
) -> Element<'a, Message> {
    let mut col = column![muted(label), control].spacing(theme::SPACE_2XS);
    if let Some(error) = error {
        col = col.push(text(error).size(theme::TEXT_XS).color(palette().error));
    }
    col.width(Length::Fill).into()
}

fn muted(label: &str) -> Element<'_, Message> {
    text(label)
        .size(theme::TEXT_XS)
        .color(palette().muted)
        .into()
}

/// Width of the indeterminate progress bar.
pub const BUSY_WIDTH: f32 = 240.0;
/// Its moving part, a quarter of it.
const BUSY_SEGMENT: f32 = BUSY_WIDTH / 4.0;
/// Frames ([`Message::BusyTick`]) for one sweep across.
pub const BUSY_FRAMES: u32 = 24;

/// Where the moving part starts at frame `phase`: it sweeps right, then back.
pub fn busy_offset(phase: u32) -> f32 {
    let frame = phase % (2 * BUSY_FRAMES);
    let step = if frame < BUSY_FRAMES {
        frame
    } else {
        2 * BUSY_FRAMES - frame
    };
    (BUSY_WIDTH - BUSY_SEGMENT) * step as f32 / BUSY_FRAMES as f32
}

/// The indeterminate progress bar (4 px, accent on a `rule` track) with its label: the
/// work has no known end (Argon2id, a whole import).
pub fn busy<'a>(label: &'a str, phase: u32) -> Element<'a, Message> {
    let bar = container(row![
        space().width(busy_offset(phase)),
        container(space())
            .width(BUSY_SEGMENT)
            .height(4)
            .style(theme::track_fill),
    ])
    .width(BUSY_WIDTH)
    .height(4)
    .style(theme::track);
    column![text(label).size(theme::TEXT_SM).color(palette().muted), bar]
        .spacing(theme::SPACE_XS)
        .into()
}
