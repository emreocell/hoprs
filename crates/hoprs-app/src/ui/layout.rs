//! The shell of mockup v2: top bar across the window; below it the
//! 256 px sidebar on the left and, on the right, the center, the
//! collapsible transfer queue and the status bar.

use iced::widget::{button, column, container, row, rule, space, text};
use iced::{Alignment, Element, Length, Padding};

use crate::app::{App, Form, Message, Tab, TabState};
use crate::ui::strings;
use crate::ui::theme::{self, palette};
use crate::ui::{sidebar, site_form, workspace_form};

pub const SIDEBAR_WIDTH: f32 = sidebar::WIDTH;
/// 4 px above the 36 px tabs.
pub const TOP_BAR_HEIGHT: f32 = 40.0;
pub const QUEUE_HEAD_HEIGHT: f32 = 36.0;
pub const QUEUE_MAX_HEIGHT: f32 = 210.0;
pub const STATUS_HEIGHT: f32 = 26.0;
/// The accent line under an active tab.
pub const UNDERLINE: f32 = 2.0;

pub fn view(app: &App) -> Element<'_, Message> {
    let right = column![center(app), queue(app), status_bar()].width(Length::Fill);
    let body = row![sidebar::view(app), vrule(), right].height(Length::Fill);

    container(column![top_bar(app), hrule(), body])
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

fn top_bar(app: &App) -> Element<'_, Message> {
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

    let tabs: Element<'_, Message> = if app.tabs.is_empty() {
        container(muted(strings::NO_SESSIONS, theme::TEXT_XS))
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(Padding::ZERO.left(theme::SPACE_MD))
            .align_y(Alignment::Center)
            .into()
    } else {
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
    };

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

/// The center: the open form, else the active tab, else the empty state.
fn center(app: &App) -> Element<'_, Message> {
    match &app.form {
        Some(Form::Site(form)) => return site_form::view(form),
        Some(Form::Workspace(form)) => return workspace_form::view(form, &app.sites),
        None => {}
    }
    if let Some(tab) = app.active_tab.and_then(|i| app.tabs.get(i)) {
        return tab_placeholder(app, tab);
    }
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

/// A session tab: site name and protocol tag (Task 6 adds closing and the panes).
fn session_tab<'a>(app: &'a App, index: usize, tab: &'a Tab) -> Element<'a, Message> {
    let active = app.active_tab == Some(index);
    let (name, tag) = match app.site(&tab.site) {
        Some(site) => (site.name.as_str(), sidebar::protocol_tag(site)),
        None => (strings::SITE_MISSING, ""),
    };
    let p = palette();
    let label = row![
        text(name),
        text(tag)
            .font(theme::MONO)
            .size(theme::TEXT_TAG)
            .color(if active { p.ink_2 } else { p.muted }),
    ]
    .spacing(theme::SPACE_XS)
    .align_y(Alignment::Center);
    let button = button(
        container(label)
            .height(Length::Fill)
            .align_y(Alignment::Center),
    )
    .height(Length::Fill)
    .padding(Padding::ZERO.left(theme::SPACE_MD).right(theme::SPACE_MD))
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

/// The tab's center until Task 6 brings the file panes: its connection state.
fn tab_placeholder<'a>(app: &'a App, tab: &'a Tab) -> Element<'a, Message> {
    let p = palette();
    let site = app.site(&tab.site);
    let address = site
        .map(|s| format!("{}@{}:{}", s.user, s.host, s.port))
        .unwrap_or_default();
    let mut body = column![
        text(site.map_or(strings::SITE_MISSING, |s| s.name.as_str())).font(theme::INTER_SEMIBOLD),
        text(address)
            .font(theme::MONO)
            .size(theme::TEXT_MONO)
            .color(p.muted),
    ]
    .spacing(theme::SPACE_2XS)
    .align_x(Alignment::Center);
    let keyring_note = |unavailable: bool| {
        unavailable.then(|| muted(strings::KEYRING_UNAVAILABLE, theme::TEXT_XS))
    };
    match &tab.state {
        TabState::Connecting => body = body.push(muted(strings::CONNECTING, theme::TEXT_SM)),
        TabState::Connected(connected) => {
            body = body
                .push(text(strings::CONNECTED).size(theme::TEXT_SM))
                .push(muted(strings::FILES_ARRIVE, theme::TEXT_SM))
                .push(keyring_note(connected.keyring_unavailable));
        }
        TabState::Failed(failure) => {
            body = body
                .push(text(strings::CANNOT_CONNECT).color(p.error))
                .push(text(&failure.error.message).size(theme::TEXT_SM))
                .push(failure.error.detail.as_ref().map(|detail| {
                    text(detail)
                        .font(theme::MONO)
                        .size(theme::TEXT_MONO)
                        .color(p.muted)
                }))
                .push(keyring_note(failure.keyring_unavailable));
            if site.is_some() {
                body = body.push(
                    row![
                        button(text(strings::RETRY))
                            .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_XS]))
                            .style(theme::button_outline)
                            .on_press(Message::ConnectSite(tab.site.clone())),
                        button(text(strings::EDIT_SITE))
                            .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_XS]))
                            .style(theme::button_text)
                            .on_press(Message::EditSite(tab.site.clone())),
                    ]
                    .spacing(theme::SPACE_XS),
                );
            }
        }
    }
    if site.is_some() && !matches!(tab.state, TabState::Failed(_)) {
        body = body.push(
            button(text(strings::EDIT_SITE).size(theme::TEXT_XS))
                .padding(0)
                .style(theme::button_text)
                .on_press(Message::EditSite(tab.site.clone())),
        );
    }
    container(body).center(Length::Fill).into()
}
