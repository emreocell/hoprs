//! The 256 px sidebar of mockup v2: filter, Workspaces (expandable, with "Open
//! workspace"), All sites, the unreadable entries and the footer.

use hoprs_core::{OpenAs, Protocol, Site, Workspace};
use iced::widget::{button, column, container, row, rule, scrollable, space, text, text_input};
use iced::{Alignment, Element, Length, Padding};

use crate::app::{App, Load, Message};
use crate::ui::strings;
use crate::ui::theme::{self, palette};

pub const WIDTH: f32 = 256.0;
pub const SEARCH_HEIGHT: f32 = 32.0;
/// The "Open workspace" button.
const OPEN_HEIGHT: f32 = 32.0;

/// Whether `site` matches the filter: name, host or user contains it, ignoring case.
pub fn site_matches(site: &Site, filter: &str) -> bool {
    let filter = filter.trim().to_lowercase();
    filter.is_empty()
        || [&site.name, &site.host, &site.user]
            .iter()
            .any(|field| field.to_lowercase().contains(&filter))
}

/// The protocol tag shown next to a site, as in the mockup: what the site opens.
pub fn protocol_tag(site: &Site) -> &'static str {
    match site.protocol {
        Protocol::Ssh => match site.ssh.open_as {
            OpenAs::Both => "ssh + sftp",
            OpenAs::Files => "sftp",
            OpenAs::Terminal => "ssh",
        },
        Protocol::Ftp => "ftp",
        Protocol::FtpsExplicit | Protocol::FtpsImplicit => "ftps",
    }
}

pub fn view(app: &App) -> Element<'_, Message> {
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
            .style(theme::button_text)
            .on_press(Message::OpenImport),
        button(text(strings::SETTINGS).size(theme::TEXT_XS))
            .padding(0)
            .style(theme::button_text)
            .on_press(Message::OpenSettings),
    ]
    .spacing(theme::SPACE_MD);

    let body: Element<'_, Message> = match &app.load {
        Load::Loading => skeleton(),
        Load::Failed(error) => column![
            group_rule(),
            container(
                column![
                    text(strings::CANNOT_LOAD_SITES).color(palette().error),
                    text(&error.message)
                        .size(theme::TEXT_XS)
                        .color(palette().muted),
                    action(strings::RETRY, Message::ReloadProfiles),
                ]
                .spacing(theme::SPACE_2XS)
            )
            .padding(Padding::from([theme::SPACE_XS, theme::SPACE_MD])),
        ]
        .into(),
        Load::Loaded => {
            let mut lists = column![workspaces(app), all_sites(app)];
            if !app.unreadable.is_empty() || app.sidebar_error.is_some() {
                lists = lists.push(unreadable(app));
            }
            scrollable(lists).height(Length::Fill).into()
        }
    };

    let content = column![
        search,
        container(body).height(Length::Fill),
        group_rule(),
        container(footer).padding(Padding::from([theme::SPACE_SM, theme::SPACE_MD])),
    ]
    .height(Length::Fill);

    container(content)
        .width(WIDTH - theme::HAIRLINE)
        .height(Length::Fill)
        .style(theme::sidebar)
        .into()
}

fn group_rule<'a>() -> Element<'a, Message> {
    rule::horizontal(theme::HAIRLINE)
        .style(theme::rule_1px)
        .into()
}

/// A muted line in the group's padding.
fn note(label: &str) -> Element<'_, Message> {
    container(text(label).size(theme::TEXT_SM).color(palette().muted))
        .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_MD]))
        .into()
}

fn action(label: &str, on_press: Message) -> Element<'_, Message> {
    button(text(label).size(theme::TEXT_XS))
        .padding(0)
        .style(theme::button_text)
        .on_press(on_press)
        .into()
}

/// A sidebar group: rule, heading with "+ New", then its body.
fn group<'a>(title: &'a str, on_new: Message, body: Element<'a, Message>) -> Element<'a, Message> {
    let head = row![
        text(title).size(theme::TEXT_XS).color(palette().muted),
        space::horizontal(),
        button(text(strings::NEW).size(theme::TEXT_XS))
            .padding(Padding::from([0.0, theme::SPACE_2XS]))
            .style(theme::button_text)
            .on_press(on_new),
    ]
    .align_y(Alignment::Center)
    .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_MD]));

    column![
        group_rule(),
        container(column![head, body].spacing(theme::SPACE_2XS))
            .padding(Padding::from([theme::SPACE_XS, 0.0])),
    ]
    .into()
}

/// A site row: name on the left, protocol tag in mono on the right; a click connects.
/// With `edit`, a muted "Edit" after it opens the site form without connecting.
fn site_row(site: &Site, edit: bool) -> Element<'_, Message> {
    let open = button(
        // The tag never wraps: a long name wraps instead.
        row![
            text(&site.name).size(theme::TEXT_SM).width(Length::Fill),
            text(protocol_tag(site))
                .font(theme::MONO)
                .size(theme::TEXT_TAG)
                .color(palette().muted)
                .wrapping(text::Wrapping::None),
        ]
        .spacing(theme::SPACE_XS)
        .align_y(Alignment::Center),
    )
    .width(Length::Fill)
    .padding(Padding::from([theme::SPACE_3XS, theme::SPACE_XS]))
    .style(theme::row)
    .on_press(Message::ConnectSite(site.id.clone()));
    if !edit {
        return open.into();
    }
    row![
        open,
        button(text(strings::EDIT).size(theme::TEXT_XS))
            .padding(Padding::from([theme::SPACE_3XS, theme::SPACE_2XS]))
            .style(theme::button_text)
            .on_press(Message::EditSite(site.id.clone())),
    ]
    .spacing(theme::SPACE_3XS)
    .align_y(Alignment::Center)
    .into()
}

/// The loading state: grey bars where the groups' rows will be.
fn skeleton<'a>() -> Element<'a, Message> {
    let bar = |width: f32| {
        container(space())
            .width(width)
            .height(theme::TEXT_SM)
            .style(theme::skeleton)
    };
    let rows = [120.0, 168.0, 96.0, 144.0, 112.0];
    column![
        group_rule(),
        container(column(rows.into_iter().map(|w| bar(w).into())).spacing(theme::SPACE_SM))
            .padding(Padding::from([theme::SPACE_SM, theme::SPACE_MD])),
    ]
    .into()
}

fn workspaces(app: &App) -> Element<'_, Message> {
    let visible: Vec<&Workspace> = app
        .workspaces
        .iter()
        .filter(|ws| {
            app.site_filter.trim().is_empty()
                || ws
                    .name
                    .to_lowercase()
                    .contains(&app.site_filter.trim().to_lowercase())
                || ws.sites.iter().any(|entry| {
                    app.site(&entry.site)
                        .is_some_and(|site| site_matches(site, &app.site_filter))
                })
        })
        .collect();
    let body: Element<'_, Message> = if app.workspaces.is_empty() {
        note(strings::NO_WORKSPACES)
    } else if visible.is_empty() {
        note(strings::NO_MATCHES)
    } else {
        column(visible.into_iter().map(|ws| workspace(app, ws)))
            .spacing(theme::SPACE_3XS)
            .into()
    };
    group(strings::WORKSPACES, Message::NewWorkspace, body)
}

fn workspace<'a>(app: &'a App, ws: &'a Workspace) -> Element<'a, Message> {
    let open = app.expanded.contains(&ws.id);
    let p = palette();
    let head = button(
        row![
            text(&ws.name)
                .font(theme::INTER_MEDIUM)
                .color(if open { p.ink } else { p.ink_2 })
                .width(Length::Fill),
            text(strings::workspace_site_count(
                ws.sites
                    .iter()
                    .filter(|entry| app.site(&entry.site).is_some())
                    .count(),
                ws.sites.len(),
            ))
            .size(theme::TEXT_XS)
            .color(p.muted)
            .wrapping(text::Wrapping::None),
        ]
        .spacing(theme::SPACE_XS)
        .align_y(Alignment::Center),
    )
    .width(Length::Fill)
    .padding(theme::SPACE_XS)
    .style(if open {
        theme::row_selected
    } else {
        theme::row
    })
    .on_press(Message::ToggleWorkspace(ws.id.clone()));
    let head = container(head).padding(Padding::from([0.0, theme::SPACE_XS]));
    if !open {
        return head.into();
    }

    let sites = column(ws.sites.iter().map(|entry| {
        match app.site(&entry.site) {
            Some(site) => site_row(site, false),
            None => container(
                text(strings::SITE_MISSING)
                    .size(theme::TEXT_SM)
                    .color(palette().error),
            )
            .padding(Padding::from([theme::SPACE_3XS, theme::SPACE_XS]))
            .into(),
        }
    }));
    // The 1 px `rule_2` line on the left of the nested list.
    let nested = row![
        rule::vertical(theme::HAIRLINE).style(theme::rule_strong),
        container(sites).padding(Padding::from([0.0, theme::SPACE_XS])),
    ]
    .height(Length::Shrink);
    let nested = container(nested).padding(
        Padding::ZERO
            .top(theme::SPACE_2XS)
            .left(theme::SPACE_LG)
            .right(theme::SPACE_XS),
    );

    let open_button = button(
        container(text(strings::OPEN_WORKSPACE).font(theme::INTER_SEMIBOLD))
            .center_x(Length::Fill)
            .center_y(Length::Fill),
    )
    .width(Length::Fill)
    .height(OPEN_HEIGHT)
    .padding(0)
    .style(theme::button_primary)
    .on_press(Message::OpenWorkspace(ws.id.clone()));
    let actions = row![
        open_button,
        button(text(strings::EDIT).size(theme::TEXT_XS))
            .padding(Padding::from([0.0, theme::SPACE_2XS]))
            .style(theme::button_text)
            .on_press(Message::EditWorkspace(ws.id.clone())),
    ]
    .spacing(theme::SPACE_XS)
    .align_y(Alignment::Center);
    // The open workspace can be closed: its tabs are saved, then closed.
    let actions = if app.open_workspace.as_ref() == Some(&ws.id) {
        actions.push(
            button(text(strings::CLOSE_WORKSPACE).size(theme::TEXT_XS))
                .padding(Padding::from([0.0, theme::SPACE_2XS]))
                .style(theme::button_text)
                .on_press(Message::CloseWorkspace(ws.id.clone())),
        )
    } else {
        actions
    };
    let actions = container(actions).padding(
        Padding::ZERO
            .top(theme::SPACE_SM)
            .bottom(theme::SPACE_XS)
            .left(theme::SPACE_MD)
            .right(theme::SPACE_MD),
    );

    column![head, nested, actions].into()
}

fn all_sites(app: &App) -> Element<'_, Message> {
    let visible = app.visible_sites();
    let body: Element<'_, Message> = if app.sites.is_empty() {
        container(
            column![
                text(strings::NO_SITES)
                    .size(theme::TEXT_SM)
                    .color(palette().muted),
                action(strings::NEW_SITE, Message::NewSite),
            ]
            .spacing(theme::SPACE_2XS),
        )
        .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_MD]))
        .into()
    } else if visible.is_empty() {
        note(strings::NO_MATCHES)
    } else {
        container(column(visible.into_iter().map(|site| site_row(site, true))))
            .padding(Padding::from([0.0, theme::SPACE_XS]))
            .into()
    };
    group(strings::ALL_SITES, Message::NewSite, body)
}

/// Entries kept on disk but unreadable here, and the way to look at the file.
fn unreadable(app: &App) -> Element<'_, Message> {
    let mut list = column![].spacing(theme::SPACE_XS);
    for entry in &app.unreadable {
        let name = strings::unreadable_name(entry.name.as_deref(), entry.file, entry.index);
        list = list.push(
            column![
                text(name).font(theme::MONO).size(theme::TEXT_MONO),
                text(strings::UNREADABLE_ENTRY)
                    .size(theme::TEXT_XS)
                    .color(palette().error),
            ]
            .spacing(theme::SPACE_3XS),
        );
    }
    list = list.push(action(
        strings::OPEN_PROFILE_FOLDER,
        Message::OpenProfileFolder,
    ));
    if let Some(error) = &app.sidebar_error {
        list = list.push(
            text(strings::cannot_open_folder(&error.message))
                .size(theme::TEXT_XS)
                .color(palette().error),
        );
    }
    column![
        group_rule(),
        container(list).padding(Padding::from([theme::SPACE_SM, theme::SPACE_MD])),
    ]
    .into()
}
