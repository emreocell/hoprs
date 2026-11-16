//! The status bar (mockup v2): the active tab's connection state, `user@host:port` in
//! mono, how the session logged in, and the open workspace's "3 of 3 sites in Shop"; on
//! the right, "Log" opens the technical log panel.

use hoprs_core::{Protocol, Site};
use iced::widget::{column, container, row, rule, space, text};
use iced::{Alignment, Color, Element, Length, Padding};

use crate::app::{App, ConnStatus, LogMsg, Message};
use crate::ui::icons::{self, Icon};
use crate::ui::layout::STATUS_HEIGHT;
use crate::ui::strings;
use crate::ui::theme::{self, palette};
use crate::ui::widgets;

/// How the state reads: Connected in ink, a lost connection in the error colour, the
/// rest muted.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Tone {
    Ok,
    Error,
    Muted,
}

/// What the status bar shows, as text (pure: the view only draws it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusLine {
    pub state: String,
    pub tone: Tone,
    /// `user@host:port` of the active tab's site.
    pub endpoint: Option<String>,
    /// The lock before `auth`: `None` for plain FTP.
    pub encryption: Option<Icon>,
    /// "ed25519 via ssh-agent", "password", "FTPS explicit"...
    pub auth: Option<String>,
    /// "3 of 3 sites in Shop", while a workspace is open.
    pub workspace: Option<String>,
}

/// The lock for a site that is encrypted on the wire (SSH, FTPS); none for plain FTP.
pub fn encryption_icon(site: &Site) -> Option<Icon> {
    match site.protocol {
        Protocol::Ssh | Protocol::FtpsExplicit | Protocol::FtpsImplicit => Some(Icon::LockSimple),
        Protocol::Ftp => None,
    }
}

pub fn line(app: &App) -> StatusLine {
    let workspace = app.open_workspace.as_ref().and_then(|id| {
        let (connected, total) = app.workspace_connected(id)?;
        let name = &app.workspace(id)?.name;
        Some(strings::workspace_status(connected, total, name))
    });
    let tab = app.active_tab.and_then(|i| app.tabs.get(i));
    let site = tab.and_then(|t| app.site(&t.site));
    let (state, tone) = match tab.map(|t| app.conn(&t.site).conn_status()) {
        _ if app.closing => (strings::closing().to_owned(), Tone::Muted),
        None => (strings::not_connected().to_owned(), Tone::Muted),
        Some(ConnStatus::Connecting) => (strings::connecting().to_owned(), Tone::Muted),
        Some(ConnStatus::Connected) => (strings::connected().to_owned(), Tone::Ok),
        Some(ConnStatus::Reconnecting { attempt, of }) => {
            (strings::reconnecting(attempt, of), Tone::Muted)
        }
        Some(ConnStatus::Waiting) => (strings::waiting_for_you().to_owned(), Tone::Muted),
        Some(ConnStatus::Disconnected(error)) => {
            (strings::disconnected(&strings::error(error)), Tone::Error)
        }
    };
    let auth = match (tab, site) {
        (Some(tab), Some(site)) => match strings::ftp_auth(site.protocol) {
            Some(label) => Some(label.to_owned()),
            None => app.conn(&tab.site).auth.as_ref().map(strings::ssh_auth),
        },
        _ => None,
    };
    StatusLine {
        state,
        tone,
        endpoint: site.map(|s| strings::endpoint(&s.user, &s.host, s.port)),
        encryption: site.and_then(encryption_icon),
        auth,
        workspace,
    }
}

pub fn view(app: &App) -> Element<'_, Message> {
    let line = line(app);
    let p = palette();
    let color: Color = match line.tone {
        Tone::Ok => p.ink,
        Tone::Error => p.error,
        Tone::Muted => p.muted,
    };
    let (state_icon, state_color) = match line.tone {
        Tone::Ok => (Icon::PlugsConnected, p.success),
        Tone::Error => (Icon::Warning, p.error),
        Tone::Muted => (Icon::Plug, p.muted),
    };
    let mut items = row![
        row![
            icons::icon(state_icon, icons::SMALL).color(state_color),
            text(line.state).size(theme::TEXT_XS).color(color),
        ]
        .spacing(theme::SPACE_2XS)
        .align_y(Alignment::Center)
    ]
    .spacing(theme::SPACE_LG)
    .align_y(Alignment::Center);
    if let Some(endpoint) = line.endpoint {
        items = items.push(text(endpoint).font(theme::MONO).size(theme::TEXT_XS));
    }
    if let Some(auth) = line.auth {
        let lock = line
            .encryption
            .map(|icon| icons::icon(icon, icons::SMALL).color(p.muted));
        items = items.push(
            row![lock, text(auth).size(theme::TEXT_XS)]
                .spacing(theme::SPACE_2XS)
                .align_y(Alignment::Center),
        );
    }
    if let Some(workspace) = line.workspace {
        items = items.push(text(workspace).size(theme::TEXT_XS));
    }
    // The technical log panel opens and closes here.
    let items = items
        .push(space::horizontal())
        .push(widgets::icon_toggle(
            Icon::Scroll,
            strings::log(),
            None,
            Some(Message::Log(LogMsg::Toggle)),
            app.log.open,
        ))
        .width(Length::Fill);
    column![
        rule::horizontal(theme::HAIRLINE).style(theme::rule_1px),
        container(items)
            .width(Length::Fill)
            .height(STATUS_HEIGHT - theme::HAIRLINE)
            .padding(Padding::ZERO.left(theme::SPACE_MD).right(theme::SPACE_XS))
            .align_y(Alignment::Center)
            .clip(true)
            .style(theme::status_bar),
    ]
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use hoprs_core::Protocol;

    #[test]
    fn encryption_icon_per_protocol() {
        let icon = |protocol| encryption_icon(&Site::new("s", "h", "u", protocol));
        assert_eq!(icon(Protocol::Ssh), Some(Icon::LockSimple));
        assert_eq!(icon(Protocol::FtpsExplicit), Some(Icon::LockSimple));
        assert_eq!(icon(Protocol::FtpsImplicit), Some(Icon::LockSimple));
        assert_eq!(icon(Protocol::Ftp), None);
    }
}
