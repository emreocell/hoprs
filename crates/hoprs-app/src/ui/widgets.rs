//! Small pieces shared by the forms: labelled field, inline error, in-place confirmation,
//! the indeterminate progress bar.

use hoprs_core::Error;
use std::borrow::Cow;

use iced::widget::{button, column, container, row, space, text, tooltip};
use iced::{Alignment, Element, Font, Length, Padding};

use crate::app::Message;
use crate::ui::dashed::dashed;
use crate::ui::fit;
use crate::ui::icons::{self, Icon};
use crate::ui::strings;
use crate::ui::theme::{self, palette};

/// Hit area (width and height) of an [`icon_button`].
pub const ICON_HIT: f32 = 22.0;

/// The tooltip body: `label`, then the shortcut in mono when there is one.
pub fn hint<'a>(label: &'a str, shortcut: Option<&'a str>) -> Element<'a, Message> {
    let line = row![text(label).size(theme::TEXT_XS)]
        .push(shortcut.map(|key| {
            text(key)
                .font(theme::MONO)
                .size(theme::TEXT_TAG)
                .color(palette().muted)
        }))
        .spacing(theme::SPACE_XS)
        .align_y(Alignment::Center);
    container(line)
        .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_XS]))
        .style(theme::dialog_box)
        .into()
}

/// `content` with a tooltip of `label` and `shortcut` below it.
pub fn tip<'a>(
    content: impl Into<Element<'a, Message>>,
    label: &'a str,
    shortcut: Option<&'a str>,
) -> Element<'a, Message> {
    tooltip(content, hint(label, shortcut), tooltip::Position::Bottom).into()
}

/// An icon-only button, at least [`ICON_HIT`] square, with a tooltip of `label` and
/// `shortcut`. Disabled (and dimmed) when `on_press` is `None`.
pub fn icon_button<'a>(
    icon: Icon,
    label: &'a str,
    shortcut: Option<&'a str>,
    on_press: Option<Message>,
) -> Element<'a, Message> {
    icon_toggle(icon, label, shortcut, on_press, false)
}

/// [`icon_button`] that keeps its ink look while `active` (the panel it toggles is open).
pub fn icon_toggle<'a>(
    icon: Icon,
    label: &'a str,
    shortcut: Option<&'a str>,
    on_press: Option<Message>,
    active: bool,
) -> Element<'a, Message> {
    let glyph = container(icons::icon(icon, icons::ROW))
        .center_x(Length::Fill)
        .center_y(Length::Fill);
    let body = button(glyph)
        .width(ICON_HIT)
        .height(ICON_HIT)
        .padding(0)
        .style(theme::icon_button_active(active))
        .on_press_maybe(on_press);
    tooltip(body, hint(label, shortcut), tooltip::Position::Bottom).into()
}

/// A toolbar button: the icon, then the label (the look of the pane toolbars' actions).
/// Disabled (and dimmed) when `on_press` is `None`; the tooltip carries the shortcut.
pub fn tool_button<'a>(
    icon: Icon,
    label: &'a str,
    shortcut: Option<&'a str>,
    on_press: Option<Message>,
) -> button::Button<'a, Message> {
    // The tooltip needs an element around the button, so the shortcut shows as a key
    // hint next to the label instead, like the toolbars did.
    let mut content = row![
        icons::icon(icon, icons::ROW),
        text(label)
            .size(theme::TEXT_XS)
            .wrapping(text::Wrapping::None)
    ]
    .spacing(theme::SPACE_2XS);
    if let Some(key) = shortcut {
        content = content.push(
            text(key)
                .font(theme::MONO)
                .size(theme::TEXT_TAG)
                .color(palette().disabled),
        );
    }
    button(content.align_y(Alignment::Center))
        .padding(Padding::from([theme::SPACE_3XS, theme::SPACE_2XS]))
        .style(theme::button_text)
        .on_press_maybe(on_press)
}

/// The dash colour of an outline button: `disabled` (about 3:1 on `paper`, since `rule_2`
/// was too faint to read as dashes) while it cannot be pressed, none while it can.
pub fn outline_dash(enabled: bool) -> Option<iced::Color> {
    (!enabled).then(|| palette().disabled)
}

/// An outline-style `button`, solid while `enabled` and dashed once it is disabled (give it
/// `on_press_maybe` with the same condition). Takes any button, so icons and custom
/// padding keep their layout.
pub fn outlined<'a>(button: button::Button<'a, Message>, enabled: bool) -> Element<'a, Message> {
    dashed(
        button.style(theme::button_outline_dashed),
        outline_dash(enabled),
    )
}

/// `full` cut with an ellipsis to the `width` it has, on one clipped line; the tooltip
/// shows `full` only when it was cut.
pub fn fitted<'a>(full: &'a str, width: f32, size: f32, font: Font) -> Element<'a, Message> {
    let advance = if font.family == theme::MONO.family {
        fit::MONO_ADVANCE
    } else {
        fit::INTER_ADVANCE
    };
    let shown = fit::ellipsize(full, width, size, advance);
    let cut = matches!(shown, Cow::Owned(_));
    let line: Element<'a, Message> = container(
        text(shown)
            .size(size)
            .font(font)
            .wrapping(text::Wrapping::None),
    )
    .width(width)
    .clip(true)
    .into();
    if cut {
        tooltip(line, hint(full, None), tooltip::Position::Bottom).into()
    } else {
        line
    }
}

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
            let mut lines = column![text(crate::ui::strings::error(error)).color(palette().error)];
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

#[cfg(test)]
mod tests {
    use super::*;
    use iced::widget::button as iced_button;

    #[test]
    fn a_disabled_outline_button_is_dashed_and_an_enabled_one_solid() {
        let p = palette();
        assert_eq!(outline_dash(false), Some(p.disabled));
        assert_eq!(outline_dash(true), None);
        // The wrapped button's own border is transparent only while disabled, so the
        // dashes are the outline and nothing shifts.
        let style = |status| theme::button_outline_dashed(&iced::Theme::Dark, status);
        assert_eq!(
            style(iced_button::Status::Disabled).border.color,
            iced::Color::TRANSPARENT
        );
        assert_eq!(style(iced_button::Status::Active).border.color, p.rule);
    }
}
