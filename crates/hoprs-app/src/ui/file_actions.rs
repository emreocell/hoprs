//! The file actions of a pane, as drawn: the toolbar (New folder, Rename, Delete and, on
//! the remote side, Permissions…), the thin bars above the table (the delete
//! confirmation, the permissions dialog, a name the editor refused, an operation's error
//! with "Dismiss") and the inline name editor. Nothing here is a modal.

use hoprs_core::{EntryKind, Error};
use iced::widget::{button, checkbox, column, container, row, space, text, text_input, tooltip};
use iced::{Alignment, Element, Length, Padding};

use crate::app::actions::{BusyKind, Edit, EditKind, PERMISSION_BITS, PaneAction, Permissions};
use crate::app::files::Pane;
use crate::app::{Message, PaneMsg, SiteConn, Tab, TabId};
use crate::backend::Side;
use crate::ui::files::edit_id;
use crate::ui::fit::Toolbar;
use crate::ui::icons::{self, Icon};
use crate::ui::strings;
use crate::ui::theme::{self, palette};
use crate::ui::virtual_list::ROW_HEIGHT;
use crate::ui::widgets;

/// The toolbar under the path bar: the actions, each with its key, and on the right the
/// operation in flight.
/// `form` ([`crate::ui::fit::toolbar`]): below Full no key hints, and while an operation
/// runs only its progress (the actions are all dim then); Narrow shows icons only.
pub fn toolbar<'a>(
    tab: &'a Tab,
    conn: &SiteConn,
    side: Side,
    form: Toolbar,
) -> Element<'a, Message> {
    let p = palette();
    let compact = !form.hints();
    // The narrowest panes show the icons only; the tooltips carry label and key.
    let short = form.icons_only();
    let Some(pane) = tab.pane(side) else {
        return space().into();
    };
    let msg = move |msg: PaneMsg| Message::Pane {
        tab: tab.id,
        side,
        msg,
    };
    let can = tab.can_act(conn, side);
    let some = can && !pane.selected_entries().is_empty();
    let chmod = can && !pane.permission_targets().is_empty();
    let action =
        |icon: Icon, label: &'a str, key: Option<&'a str>, enabled: bool, pane_msg: PaneMsg| {
            let on_press = enabled.then(|| msg(pane_msg));
            if short {
                widgets::icon_button(icon, label, key, on_press)
            } else {
                widgets::tool_button(icon, label, key.filter(|_| !compact), on_press).into()
            }
        };
    let (transfer_icon, transfer) = match side {
        Side::Local => (Icon::UploadSimple, strings::upload()),
        Side::Remote => (Icon::DownloadSimple, strings::download()),
    };
    let mut bar = row![];
    // Edit (remote): the selected file, or why it can't be (the hint, on the right).
    let edit = tab.edit_target(conn, side);
    // A narrow pane while an operation runs: only its progress (every action is dim).
    if !(compact && pane.busy.is_some()) {
        bar = bar.push(action(
            transfer_icon,
            transfer,
            Some(strings::key_transfer()),
            tab.can_transfer(conn, side),
            PaneMsg::Transfer,
        ));
        if side == Side::Remote {
            bar = bar.push(edit_action(edit, compact, short, msg(PaneMsg::Edit)));
        }
        bar = bar
            .push(action(
                Icon::FolderSimplePlus,
                strings::new_folder(),
                Some(strings::key_new_folder()),
                can,
                PaneMsg::NewFolder,
            ))
            .push(action(
                Icon::Textbox,
                strings::rename(),
                Some(strings::key_rename()),
                some,
                PaneMsg::Rename,
            ))
            .push(action(
                Icon::Trash,
                strings::delete(),
                Some(strings::key_delete()),
                some,
                PaneMsg::Delete,
            ));
        if side == Side::Remote {
            bar = bar.push(action(
                Icon::LockKey,
                strings::permissions(),
                None,
                chmod,
                PaneMsg::Permissions,
            ));
        }
    }
    bar = bar.push(space().width(Length::Fill));
    // Folder compare: one button for the tab, at the right of the local toolbar.
    if side == Side::Local && !(compact && pane.busy.is_some()) {
        let start = tab.can_act(conn, Side::Remote).then_some(Message::Compare(
            crate::app::CompareMsg::Start { tab: tab.id },
        ));
        bar = bar.push(if short {
            widgets::icon_button(Icon::GitDiff, strings::compare_button(), None, start)
        } else {
            widgets::tool_button(Icon::GitDiff, strings::compare_button(), None, start).into()
        });
    }
    if let Some(busy) = &pane.busy {
        let label = match busy.kind {
            BusyKind::NewFolder => strings::creating_folder().to_owned(),
            BusyKind::Rename => strings::renaming().to_owned(),
            BusyKind::Delete => strings::deleting(busy.done),
            BusyKind::Permissions => strings::changing_permissions().to_owned(),
        };
        bar = bar.push(
            text(label)
                .size(theme::TEXT_XS)
                .color(p.muted)
                .wrapping(text::Wrapping::None),
        );
    }
    bar.spacing(theme::SPACE_3XS)
        .align_y(Alignment::Center)
        .height(ROW_HEIGHT)
        .padding(Padding::ZERO.left(theme::SPACE_XS).right(theme::SPACE_XS))
        .into()
}

/// "Edit F4" (remote). Off without a selection. On a folder, a link or a file over
/// 50 MB it looks off too, but a click (like F4) still answers: the reason shows inline
/// in the bar above the table; hovering shows it as well.
fn edit_action<'a>(
    edit: Option<Result<hoprs_core::RemotePath, &'static str>>,
    compact: bool,
    icon_only: bool,
    on_press: Message,
) -> Element<'a, Message> {
    let p = palette();
    let mut content =
        row![icons::icon(Icon::PencilSimpleLine, icons::ROW)].spacing(theme::SPACE_2XS);
    if !icon_only {
        content = content.push(
            text(strings::edit_file())
                .size(theme::TEXT_XS)
                .wrapping(text::Wrapping::None),
        );
    }
    if !compact && !icon_only {
        content = content.push(
            text(strings::key_edit())
                .font(theme::MONO)
                .size(theme::TEXT_TAG)
                .color(p.disabled),
        );
    }
    let refused = matches!(edit, Some(Err(_)));
    let button = button(content.align_y(Alignment::Center))
        .padding(Padding::from([theme::SPACE_3XS, theme::SPACE_2XS]))
        .style(move |t, status| {
            theme::button_text(
                t,
                if refused {
                    button::Status::Disabled
                } else {
                    status
                },
            )
        })
        .on_press_maybe(edit.is_some().then_some(on_press));
    match edit {
        Some(Err(reason)) => tooltip(
            button,
            container(text(reason).size(theme::TEXT_XS))
                .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_XS]))
                .style(theme::dialog_box),
            tooltip::Position::Bottom,
        )
        .into(),
        _ if icon_only => widgets::tip(button, strings::edit_file(), Some(strings::key_edit())),
        _ => button.into(),
    }
}

/// The bars above the table, top to bottom: the open confirmation or dialog (or the
/// editor's complaint), then the last operation's error.
pub fn bars<'a>(tab: TabId, side: Side, pane: &'a Pane) -> Vec<Element<'a, Message>> {
    let msg = move |msg: PaneMsg| Message::Pane { tab, side, msg };
    let mut bars = Vec::new();
    match &pane.action {
        Some(PaneAction::ConfirmDelete(entries)) => bars.push(confirm_delete(entries, msg)),
        Some(PaneAction::Permissions(dialog)) => bars.push(permissions(dialog, msg)),
        Some(PaneAction::Edit(Edit {
            error: Some(error), ..
        })) => bars.push(bar(text(*error)
            .size(theme::TEXT_XS)
            .color(palette().error)
            .into())),
        Some(PaneAction::Edit(_)) | None => {}
    }
    if let Some(error) = &pane.action_error {
        bars.push(error_bar(error, msg(PaneMsg::DismissError)));
    }
    if let Some(note) = &pane.note {
        bars.push(note_bar(note, msg(PaneMsg::DismissError)));
    }
    bars
}

/// A muted note above the table (a saved folder that is gone) and Dismiss.
fn note_bar(note: &str, dismiss: Message) -> Element<'_, Message> {
    bar(row![
        text(note)
            .size(theme::TEXT_XS)
            .color(palette().muted)
            .width(Length::Fill),
        button(text(strings::dismiss()).size(theme::TEXT_XS))
            .padding(Padding::from([theme::SPACE_3XS, theme::SPACE_XS]))
            .style(theme::button_text)
            .on_press(dismiss),
    ]
    .spacing(theme::SPACE_XS)
    .align_y(Alignment::Center)
    .into())
}

/// The row above the list where a new folder's name is typed.
pub fn new_folder_row<'a>(tab: TabId, side: Side, pane: &'a Pane) -> Option<Element<'a, Message>> {
    let Some(PaneAction::Edit(edit)) = &pane.action else {
        return None;
    };
    if edit.kind != EditKind::NewFolder {
        return None;
    }
    Some(
        container(name_editor(tab, side, edit))
            .width(Length::Fill)
            .height(ROW_HEIGHT)
            .padding(Padding::ZERO.left(theme::SPACE_MD).right(theme::SPACE_MD))
            .align_y(Alignment::Center)
            .style(theme::file_row(true, false, false, false))
            .into(),
    )
}

/// The name editor: Enter commits, Escape cancels (through the pane keys).
pub fn name_editor<'a>(tab: TabId, side: Side, edit: &'a Edit) -> Element<'a, Message> {
    let msg = move |msg: PaneMsg| Message::Pane { tab, side, msg };
    text_input("", &edit.value)
        .id(edit_id(tab, side))
        .width(Length::Fill)
        .on_input(move |value| msg(PaneMsg::EditChanged(value)))
        .on_submit(msg(PaneMsg::EditCommit))
        .font(theme::MONO)
        .size(theme::TEXT_MONO)
        .padding(Padding::from([theme::SPACE_3XS, theme::SPACE_XS]))
        .style(theme::search)
        .into()
}

/// A thin bar on `paper_2`.
fn bar(content: Element<'_, Message>) -> Element<'_, Message> {
    container(content)
        .width(Length::Fill)
        .padding(Padding::from([theme::SPACE_XS, theme::SPACE_MD]))
        .style(theme::pane_bar)
        .into()
}

/// The operation's error: its message, its detail in mono, "Dismiss".
pub fn error_bar(error: &Error, dismiss: Message) -> Element<'_, Message> {
    let p = palette();
    let mut lines = column![
        text(crate::ui::strings::error(error))
            .size(theme::TEXT_XS)
            .color(p.error)
    ]
    .spacing(theme::SPACE_3XS)
    .width(Length::Fill);
    if let Some(detail) = &error.detail {
        lines = lines.push(
            text(detail)
                .font(theme::MONO)
                .size(theme::TEXT_TAG)
                .color(p.muted),
        );
    }
    bar(row![
        lines,
        button(text(strings::dismiss()).size(theme::TEXT_XS))
            .padding(Padding::from([theme::SPACE_3XS, theme::SPACE_XS]))
            .style(theme::button_text)
            .on_press(dismiss),
    ]
    .spacing(theme::SPACE_XS)
    .align_y(Alignment::Center)
    .into())
}

/// "Delete these?": the names (folders with their `/`), Delete and Keep.
fn confirm_delete<'a>(
    entries: &'a [hoprs_core::RemoteEntry],
    msg: impl Fn(PaneMsg) -> Message,
) -> Element<'a, Message> {
    let p = palette();
    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    let mut listed = row![].spacing(theme::SPACE_XS);
    for entry in entries.iter().take(strings::LISTED_NAMES) {
        let mut name = row![
            text(&entry.name)
                .font(theme::MONO)
                .size(theme::TEXT_MONO)
                .wrapping(text::Wrapping::None)
        ];
        if entry.kind == EntryKind::Dir {
            name = name.push(
                text(strings::path_separator())
                    .font(theme::MONO)
                    .size(theme::TEXT_MONO)
                    .color(p.muted),
            );
        }
        listed = listed.push(name);
    }
    if entries.len() > strings::LISTED_NAMES {
        listed = listed.push(
            text(strings::and_more(entries.len() - strings::LISTED_NAMES))
                .size(theme::TEXT_XS)
                .color(p.muted),
        );
    }
    let mut lines = column![
        text(strings::delete_question(&names)).size(theme::TEXT_SM),
        listed.wrap(),
    ]
    .spacing(theme::SPACE_2XS)
    .width(Length::Fill);
    if entries.iter().any(|e| e.kind == EntryKind::Dir) {
        lines = lines.push(
            text(strings::delete_folders_note())
                .size(theme::TEXT_XS)
                .color(p.muted),
        );
    }
    bar(row![
        lines,
        button(text(strings::delete()).font(theme::INTER_SEMIBOLD))
            .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_MD]))
            .style(theme::button_primary)
            .on_press(msg(PaneMsg::ConfirmDelete)),
        button(text(strings::keep()))
            .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_XS]))
            .style(theme::button_text)
            .on_press(msg(PaneMsg::CancelAction)),
    ]
    .spacing(theme::SPACE_XS)
    .align_y(Alignment::Center)
    .into())
}

/// The permissions dialog, in place: the rwx grid, the octal value, Apply and Cancel.
fn permissions<'a>(
    dialog: &'a Permissions,
    msg: impl Fn(PaneMsg) -> Message + Copy + 'a,
) -> Element<'a, Message> {
    let p = palette();
    let label = |s: &'a str, width: f32| {
        container(text(s).size(theme::TEXT_XS).color(p.muted)).width(width)
    };
    const WHO: f32 = 64.0;
    const BOX: f32 = 72.0;
    let mut grid = column![row![
        label("", WHO),
        label(strings::perm_read(), BOX),
        label(strings::perm_write(), BOX),
        label(strings::perm_execute(), BOX),
    ]]
    .spacing(theme::SPACE_2XS);
    for (who, bits) in [
        strings::perm_owner(),
        strings::perm_group(),
        strings::perm_others(),
    ]
    .into_iter()
    .zip(PERMISSION_BITS)
    {
        let mut line = row![label(who, WHO)].align_y(Alignment::Center);
        for bit in bits {
            line = line.push(
                container(
                    checkbox(dialog.has(bit))
                        .style(theme::checkbox)
                        .on_toggle(move |_| msg(PaneMsg::PermissionToggled(bit))),
                )
                .width(BOX),
            );
        }
        grid = grid.push(line);
    }
    let mut names = row![text(strings::permissions_title()).size(theme::TEXT_SM)]
        .spacing(theme::SPACE_XS)
        .align_y(Alignment::Center);
    for name in dialog.names.iter().take(strings::LISTED_NAMES) {
        names = names.push(
            text(name)
                .font(theme::MONO)
                .size(theme::TEXT_MONO)
                .wrapping(text::Wrapping::None),
        );
    }
    if dialog.names.len() > strings::LISTED_NAMES {
        names = names.push(
            text(strings::and_more(
                dialog.names.len() - strings::LISTED_NAMES,
            ))
            .size(theme::TEXT_XS)
            .color(p.muted),
        );
    }
    if dialog.mixed {
        names = names.push(
            text(strings::mixed_permissions())
                .size(theme::TEXT_XS)
                .color(p.muted),
        );
    }
    let mut octal = column![
        text(strings::perm_octal())
            .size(theme::TEXT_XS)
            .color(p.muted),
        text_input("", &dialog.octal)
            .on_input(move |v| msg(PaneMsg::PermissionOctal(v)))
            .on_submit_maybe((!dialog.invalid).then(|| msg(PaneMsg::ApplyPermissions)))
            .font(theme::MONO)
            .size(theme::TEXT_MONO)
            .width(64)
            .padding(Padding::from([theme::SPACE_3XS, theme::SPACE_XS]))
            .style(theme::search),
    ]
    .spacing(theme::SPACE_2XS);
    if dialog.invalid {
        octal = octal.push(
            text(strings::octal_invalid())
                .size(theme::TEXT_XS)
                .color(p.error),
        );
    }
    bar(column![
        row![
            container(names.wrap()).width(Length::Fill),
            button(text(strings::apply()).font(theme::INTER_SEMIBOLD))
                .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_MD]))
                .style(theme::button_primary)
                .on_press_maybe((!dialog.invalid).then(|| msg(PaneMsg::ApplyPermissions))),
            button(text(strings::cancel()))
                .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_XS]))
                .style(theme::button_text)
                .on_press(msg(PaneMsg::CancelAction)),
        ]
        .spacing(theme::SPACE_XS)
        .align_y(Alignment::Center),
        row![grid, octal]
            .spacing(theme::SPACE_LG)
            .align_y(Alignment::End),
    ]
    .spacing(theme::SPACE_XS)
    .into())
}
