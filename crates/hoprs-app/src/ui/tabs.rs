//! The session tabs in the top bar: one per open site, its name, the protocol tag in mono
//! and a × that closes it (and its session). The active tab sits on `paper_3` over the
//! 2 px white underline.

use iced::widget::{button, column, container, row, space, text};
use iced::{Alignment, Element, Length, Padding};

use crate::app::{App, Message, Tab};
use crate::ui::layout::UNDERLINE;
use crate::ui::theme::{self, palette};
use crate::ui::{sidebar, strings};

/// The tab strip, or the "No open sessions" placeholder.
pub fn view(app: &App) -> Element<'_, Message> {
    if app.tabs.is_empty() {
        return container(
            text(strings::NO_SESSIONS)
                .size(theme::TEXT_XS)
                .color(palette().muted),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(Padding::ZERO.left(theme::SPACE_MD))
        .align_y(Alignment::Center)
        .into();
    }
    container(
        row(app
            .tabs
            .iter()
            .enumerate()
            .map(|(i, tab)| session_tab(app, i, tab)))
        .spacing(theme::SPACE_3XS),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .padding(Padding::ZERO.top(theme::SPACE_2XS).left(theme::SPACE_XS))
    .into()
}

fn session_tab<'a>(app: &'a App, index: usize, tab: &'a Tab) -> Element<'a, Message> {
    let active = app.active_tab == Some(index);
    let (name, tag) = match app.site(&tab.site) {
        Some(site) => (site.name.as_str(), sidebar::protocol_tag(site)),
        None => (strings::SITE_MISSING, ""),
    };
    let p = palette();
    let close = button(text(strings::CLOSE_TAB).size(theme::TEXT_SM))
        .padding(Padding::from([0.0, theme::SPACE_2XS]))
        .style(theme::button_text)
        .on_press(Message::CloseTab(tab.id));
    // The site's transfers stopped on a lost connection, until they run again.
    let paused = app.queue.site_paused(&tab.site).map(|_| {
        text(strings::SITE_PAUSED)
            .size(theme::TEXT_TAG)
            .color(p.error)
            .wrapping(text::Wrapping::None)
    });
    let label = row![
        text(name),
        text(tag)
            .font(theme::MONO)
            .size(theme::TEXT_TAG)
            .color(if active { p.ink_2 } else { p.muted }),
        paused,
        close,
    ]
    .spacing(theme::SPACE_XS)
    .align_y(Alignment::Center);
    let button = button(
        container(label)
            .height(Length::Fill)
            .align_y(Alignment::Center),
    )
    .height(Length::Fill)
    .padding(Padding::ZERO.left(theme::SPACE_MD).right(theme::SPACE_XS))
    .style(theme::tab(active))
    .on_press(Message::SelectTab(index));
    let underline = container(space()).width(Length::Fill).height(UNDERLINE);
    let underline = if active {
        underline.style(theme::underline)
    } else {
        underline
    };
    column![button, underline].width(Length::Shrink).into()
}
