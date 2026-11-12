//! The shell of mockup v2: top bar across the window; below it the
//! 256 px sidebar on the left and, on the right, the center, the
//! collapsible transfer queue and the status bar.

use iced::widget::{button, column, container, row, rule, space, text, text_input};
use iced::{Alignment, Element, Length, Padding};

use crate::app::{App, Message};
use crate::ui::strings;
use crate::ui::theme::{self, palette};

pub const SIDEBAR_WIDTH: f32 = 256.0;
/// 4 px above the 36 px tabs.
pub const TOP_BAR_HEIGHT: f32 = 40.0;
pub const SEARCH_HEIGHT: f32 = 32.0;
pub const QUEUE_HEAD_HEIGHT: f32 = 36.0;
pub const QUEUE_MAX_HEIGHT: f32 = 210.0;
pub const STATUS_HEIGHT: f32 = 26.0;
/// The accent line under an active tab.
pub const UNDERLINE: f32 = 2.0;

pub fn view(app: &App) -> Element<'_, Message> {
    let right = column![center(), queue(app), status_bar()].width(Length::Fill);
    let body = row![sidebar(app), vrule(), right].height(Length::Fill);

    container(column![top_bar(), hrule(), body])
        .width(Length::Fill)
        .height(Length::Fill)
        .style(theme::paper)
        .into()
}

fn hrule<'a>() -> Element<'a, Message> {
    rule::horizontal(theme::HAIRLINE)
        .style(theme::rule_1px)
        .into()
}

fn vrule<'a>() -> Element<'a, Message> {
    rule::vertical(theme::HAIRLINE)
        .style(theme::rule_1px)
        .into()
}

fn muted<'a>(label: &'a str, size: f32) -> Element<'a, Message> {
    text(label).size(size).color(palette().muted).into()
}

fn top_bar<'a>() -> Element<'a, Message> {
    let brand = container(
        row![
            text(strings::BRAND)
                .size(theme::TEXT_MD)
                .font(theme::INTER_SEMIBOLD),
            muted(strings::VERSION, theme::TEXT_XS),
        ]
        .spacing(theme::SPACE_XS)
        .align_y(Alignment::Center),
    )
    .width(SIDEBAR_WIDTH - theme::HAIRLINE)
    .height(Length::Fill)
    .padding(Padding::ZERO.left(theme::SPACE_MD).right(theme::SPACE_MD))
    .align_y(Alignment::Center);

    // Session tabs arrive with Task 6; until then, the empty state.
    let tabs = container(muted(strings::NO_SESSIONS, theme::TEXT_XS))
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(Padding::ZERO.left(theme::SPACE_MD))
        .align_y(Alignment::Center);

    // A slot for the Ctrl+K palette (Plan 6).
    let palette_slot = container(
        container(
            row![
                text(strings::GO_TO_ANYTHING),
                container(text(strings::GO_TO_ANYTHING_KEY).size(theme::TEXT_XS))
                    .padding(Padding::ZERO.left(theme::SPACE_2XS).right(theme::SPACE_2XS))
                    .style(theme::kbd),
            ]
            .spacing(theme::SPACE_SM)
            .align_y(Alignment::Center),
        )
        .height(Length::Fill)
        .padding(Padding::ZERO.left(theme::SPACE_SM).right(theme::SPACE_SM))
        .align_y(Alignment::Center)
        .style(theme::outline_box),
    )
    .height(Length::Fill)
    .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_SM]));

    row![brand, vrule(), tabs, palette_slot]
        .height(TOP_BAR_HEIGHT)
        .into()
}

fn sidebar(app: &App) -> Element<'_, Message> {
    let search = container(
        text_input(strings::FILTER_SITES, &app.site_filter)
            .on_input(Message::FilterSites)
            .padding(Padding::from([0.0, theme::SPACE_XS]))
            .size(theme::TEXT_SM)
            .line_height(iced::Pixels(SEARCH_HEIGHT - 2.0 * theme::HAIRLINE))
            .style(theme::search),
    )
    .padding(theme::SPACE_SM);

    let footer = row![
        button(text(strings::IMPORT_FILEZILLA).size(theme::TEXT_XS))
            .padding(0)
            .style(theme::button_text),
        button(text(strings::SETTINGS).size(theme::TEXT_XS))
            .padding(0)
            .style(theme::button_text),
    ]
    .spacing(theme::SPACE_MD);

    let content = column![
        search,
        group(strings::WORKSPACES, strings::NO_WORKSPACES),
        group(strings::ALL_SITES, strings::NO_SITES),
        space::vertical(),
        hrule(),
        container(footer).padding(Padding::from([theme::SPACE_SM, theme::SPACE_MD])),
    ]
    .height(Length::Fill);

    container(content)
        .width(SIDEBAR_WIDTH - theme::HAIRLINE)
        .height(Length::Fill)
        .style(theme::sidebar)
        .into()
}

/// A sidebar group: rule, heading with "+ New", then its (empty) list.
fn group<'a>(title: &'a str, empty: &'a str) -> Element<'a, Message> {
    let head = row![
        muted(title, theme::TEXT_XS),
        space::horizontal(),
        button(text(strings::NEW).size(theme::TEXT_XS))
            .padding(Padding::from([0.0, theme::SPACE_2XS]))
            .style(theme::button_text),
    ]
    .align_y(Alignment::Center)
    .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_MD]));

    column![
        hrule(),
        container(
            column![
                head,
                container(muted(empty, theme::TEXT_SM))
                    .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_MD])),
            ]
            .spacing(theme::SPACE_2XS)
        )
        .padding(Padding::from([theme::SPACE_XS, 0.0])),
    ]
    .into()
}

fn center<'a>() -> Element<'a, Message> {
    container(
        column![
            text(strings::NO_SESSION_TITLE).font(theme::INTER_SEMIBOLD),
            muted(strings::NO_SESSION_HINT, theme::TEXT_SM),
        ]
        .spacing(theme::SPACE_2XS)
        .align_x(Alignment::Center),
    )
    .center(Length::Fill)
    .into()
}

fn queue(app: &App) -> Element<'_, Message> {
    let tab = |label: &str, count: usize, active: bool| {
        tab_with_underline(
            button(text(format!("{label} {count}")))
                .padding(0)
                .style(theme::queue_tab(active))
                .into(),
            active,
        )
    };
    let toggle = if app.queue_open {
        strings::QUEUE_HIDE
    } else {
        strings::QUEUE_SHOW
    };

    let head = row![
        text(strings::TRANSFERS).font(theme::INTER_SEMIBOLD),
        tab(strings::QUEUE_ACTIVE, 0, true),
        tab(strings::QUEUE_QUEUED, 0, false),
        tab(strings::QUEUE_FAILED, 0, false),
        tab(strings::QUEUE_DONE, 0, false),
        space::horizontal(),
        muted(strings::QUEUE_TOTAL_IDLE, theme::TEXT_SM),
        button(text(toggle))
            .padding(Padding::from([theme::SPACE_3XS, theme::SPACE_XS]))
            .on_press(Message::ToggleQueue)
            .style(theme::button_text),
    ]
    .spacing(theme::SPACE_LG)
    .align_y(Alignment::Center)
    .height(QUEUE_HEAD_HEIGHT)
    .padding(Padding::ZERO.left(theme::SPACE_MD).right(theme::SPACE_MD));

    let strong = rule::horizontal(theme::HAIRLINE).style(theme::rule_strong);
    let queue = if app.queue_open {
        column![
            strong,
            head,
            hrule(),
            container(muted(strings::NO_TRANSFERS, theme::TEXT_SM))
                .padding(Padding::from([theme::SPACE_SM, theme::SPACE_MD])),
        ]
    } else {
        column![strong, head]
    };

    container(queue)
        .width(Length::Fill)
        .max_height(QUEUE_MAX_HEIGHT)
        .into()
}

/// A tab label over the 2 px accent underline of the active tab.
fn tab_with_underline(label: Element<'_, Message>, active: bool) -> Element<'_, Message> {
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

fn status_bar<'a>() -> Element<'a, Message> {
    column![
        hrule(),
        container(text(strings::NOT_CONNECTED).size(theme::TEXT_XS))
            .width(Length::Fill)
            .height(STATUS_HEIGHT - theme::HAIRLINE)
            .padding(Padding::ZERO.left(theme::SPACE_MD).right(theme::SPACE_MD))
            .align_y(Alignment::Center)
            .style(theme::status_bar),
    ]
    .into()
}
