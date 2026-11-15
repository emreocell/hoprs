//! The status bar (mockup v2): the active tab's connection state, `user@host:port` in
//! mono, how the session logged in, and the open workspace's "3 of 3 sites in Shop"; on
//! the right, "Log" opens the technical log panel.

use iced::widget::{button, column, container, row, rule, space, text};
use iced::{Alignment, Color, Element, Length, Padding};

use crate::app::{App, ConnStatus, LogMsg, Message};
use crate::ui::layout::STATUS_HEIGHT;
use crate::ui::strings;
use crate::ui::theme::{self, palette};

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
    /// "ed25519 via ssh-agent", "password", "FTPS explicit"...
    pub auth: Option<String>,
    /// "3 of 3 sites in Shop", while a workspace is open.
    pub workspace: Option<String>,
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
            (strings::disconnected(&error.message), Tone::Error)
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
    let mut items = row![text(line.state).size(theme::TEXT_XS).color(color)]
        .spacing(theme::SPACE_LG)
        .align_y(Alignment::Center);
    if let Some(endpoint) = line.endpoint {
        items = items.push(text(endpoint).font(theme::MONO).size(theme::TEXT_XS));
    }
    for extra in [line.auth, line.workspace].into_iter().flatten() {
        items = items.push(text(extra).size(theme::TEXT_XS));
    }
    // The technical log panel opens and closes here.
    let items = items
        .push(space::horizontal())
        .push(
            button(text(strings::log()).size(theme::TEXT_XS))
                .padding(Padding::from([0.0, theme::SPACE_XS]))
                .style(theme::queue_tab(app.log.open))
                .on_press(Message::Log(LogMsg::Toggle)),
        )
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
