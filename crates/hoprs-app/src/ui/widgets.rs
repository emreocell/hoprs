//! Small pieces shared by the forms: labelled field, inline error, in-place confirmation.

use hoprs_core::Error;
use iced::widget::{button, column, container, row, space, text};
use iced::{Alignment, Element, Length, Padding};

use crate::app::Message;
use crate::ui::strings;
use crate::ui::theme::{self, palette};

/// The in-place delete confirmation .
pub fn confirm<'a>(question: &'a str, yes: Message, no: Message) -> Element<'a, Message> {
    container(
        row![
            text(question).size(theme::TEXT_SM).width(Length::Fill),
            button(text(strings::DELETE).font(theme::INTER_SEMIBOLD))
                .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_MD]))
                .style(theme::button_primary)
                .on_press(yes),
            button(text(strings::KEEP))
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
