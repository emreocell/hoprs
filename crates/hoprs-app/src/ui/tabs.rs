//! The session tabs in the top bar: a site's terminal and files tabs, each with the site's
//! name, its protocol tag in mono (`ssh` for an SSH terminal, `sftp` for its files) and a
//! × that closes it (the last tab of a site also closes its session). On a connected SSH
//! tab, "New terminal tab" and "New files tab" add one on the same session. The active
//! tab sits on `paper_3` over the 2 px white underline. Tabs past the window's width
//! scroll sideways (Shift+wheel or a touchpad), as the mockup's `overflow-x: auto` strip
//! does.

use hoprs_core::{Protocol, Site, TabKind};
use iced::widget::scrollable::{Direction, Scrollbar};
use iced::widget::{Id, button, column, container, row, scrollable, space, text, tooltip};
use iced::{Alignment, Element, Length, Padding};

use crate::app::{App, Message, Tab};
use crate::ui::icons::{self, Icon};
use crate::ui::layout::UNDERLINE;
use crate::ui::theme::{self, palette};
use crate::ui::{fit, sidebar, strings, widgets};

/// A tab's name is cut to this width (the full name shows on hover).
const NAME_MAX_WIDTH: f32 = 200.0;

/// The room a tab's name gets: its estimated width, up to [`NAME_MAX_WIDTH`] (with a
/// little slack, since Inter's advance is estimated).
fn name_width(name: &str) -> f32 {
    let natural = name.chars().count() as f32 * theme::TEXT_SM * fit::INTER_ADVANCE;
    (natural + 2.0).min(NAME_MAX_WIDTH)
}

/// The id of the tab strip's scrollable ([`crate::app::Effect::ScrollTabs`]).
pub fn strip_id() -> Id {
    Id::new("tab-strip")
}

/// The tab strip, or the "No open sessions" placeholder.
pub fn view(app: &App) -> Element<'_, Message> {
    if app.tabs.is_empty() {
        return container(
            text(strings::no_sessions())
                .size(theme::TEXT_XS)
                .color(palette().muted),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(Padding::ZERO.left(theme::SPACE_MD))
        .align_y(Alignment::Center)
        .into();
    }
    let strip = row(app
        .tabs
        .iter()
        .enumerate()
        .map(|(i, tab)| session_tab(app, i, tab)))
    .spacing(theme::SPACE_3XS)
    .height(Length::Fill);
    // A thin bar over the underlines, shown only when the tabs overflow.
    let bar = Scrollbar::new()
        .width(theme::SPACE_3XS)
        .scroller_width(theme::SPACE_3XS);
    let strip = scrollable(strip)
        .id(strip_id())
        .direction(Direction::Horizontal(bar))
        .width(Length::Fill)
        .height(Length::Fill)
        .style(theme::scrollbar);
    container(row![strip, session_actions(app)].align_y(Alignment::Center))
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(Padding::ZERO.top(theme::SPACE_2XS).left(theme::SPACE_XS))
        .into()
}

/// The protocol tag of one tab: an SSH site's terminal is `ssh`, its files `sftp`; an FTP
/// site's tab has the site's tag.
pub fn tab_tag(site: &Site, kind: TabKind) -> &'static str {
    match (site.protocol, kind) {
        (Protocol::Ssh, TabKind::Terminal) => "ssh",
        (Protocol::Ssh, TabKind::Files) => "sftp",
        _ => sidebar::protocol_tag(site),
    }
}

/// The icon of one tab: a terminal's window, or the drive of a files tab.
///
/// By tab kind, not [`sidebar::site_icon`]: an SSH site's files tab is a drive too.
pub fn tab_icon(_site: &Site, kind: TabKind) -> Icon {
    match kind {
        TabKind::Terminal => Icon::TerminalWindow,
        TabKind::Files => Icon::HardDrives,
    }
}

/// "New terminal tab" and "New files tab", for the active tab when it can have more.
fn session_actions(app: &App) -> Option<Element<'_, Message>> {
    let index = app.active_tab.filter(|&i| app.can_open_session_tab(i))?;
    let tab = app.tabs[index].id;
    let action = |icon, label, kind| {
        widgets::icon_button(icon, label, None, Some(Message::NewTab { tab, kind }))
    };
    Some(
        row![
            action(
                Icon::TerminalWindow,
                strings::new_terminal_tab(),
                TabKind::Terminal
            ),
            action(Icon::FolderPlus, strings::new_files_tab(), TabKind::Files),
        ]
        .spacing(theme::SPACE_3XS)
        .padding(Padding::ZERO.left(theme::SPACE_XS).right(theme::SPACE_XS))
        .into(),
    )
}

fn session_tab<'a>(app: &'a App, index: usize, tab: &'a Tab) -> Element<'a, Message> {
    let active = app.active_tab == Some(index);
    let site = app.site(&tab.site);
    let (name, tag) = match site {
        Some(site) => (site.name.as_str(), tab_tag(site, tab.kind())),
        None => (strings::site_missing(), ""),
    };
    let p = palette();
    let kind_icon = icons::icon(
        site.map_or(Icon::HardDrives, |s| tab_icon(s, tab.kind())),
        icons::SMALL,
    )
    .color(if active { p.ink } else { p.muted });
    let close = widgets::icon_button(
        Icon::X,
        strings::close_tab(),
        None,
        Some(Message::CloseTab(tab.id)),
    );
    // The site's transfers stopped on a lost connection, until they run again.
    let paused = app.queue.site_paused(&tab.site).map(|_| {
        text(strings::site_paused())
            .size(theme::TEXT_TAG)
            .color(p.error)
            .wrapping(text::Wrapping::None)
    });
    let label = row![
        kind_icon,
        widgets::fitted(name, name_width(name), theme::TEXT_SM, theme::INTER),
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
    // A terminal's remote title (OSC 0/2) is a hint over the tab, never its name.
    let button: Element<'a, Message> = match tab.terminal().and_then(|t| t.title.as_deref()) {
        Some(title) => tooltip(
            button,
            container(
                text(title)
                    .font(theme::MONO)
                    .size(theme::TEXT_MONO)
                    .wrapping(text::Wrapping::None),
            )
            .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_XS]))
            .style(theme::dialog_box),
            tooltip::Position::Bottom,
        )
        .into(),
        None => button.into(),
    };
    let underline = container(space()).width(Length::Fill).height(UNDERLINE);
    let underline = if active {
        underline.style(theme::underline)
    } else {
        underline
    };
    column![button, underline].width(Length::Shrink).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_name_the_tab_kind() {
        let ssh = Site::new("s", "h", "u", Protocol::Ssh);
        assert_eq!(tab_tag(&ssh, TabKind::Terminal), "ssh");
        assert_eq!(tab_tag(&ssh, TabKind::Files), "sftp");
        let ftp = Site::new("f", "h", "u", Protocol::FtpsExplicit);
        assert_eq!(tab_tag(&ftp, TabKind::Files), "ftps");
    }

    #[test]
    fn tab_icons_by_kind() {
        let ssh = Site::new("s", "h", "u", Protocol::Ssh);
        assert_eq!(tab_icon(&ssh, TabKind::Terminal), Icon::TerminalWindow);
        assert_eq!(tab_icon(&ssh, TabKind::Files), Icon::HardDrives);
        let ftp = Site::new("f", "h", "u", Protocol::Ftp);
        assert_eq!(tab_icon(&ftp, TabKind::Files), Icon::HardDrives);
    }
}
