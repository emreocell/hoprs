//! The file actions of a pane, as drawn: the toolbar (New folder, Rename, Delete and, on
//! the remote side, Permissions…), the thin bars above the table (the delete
//! confirmation, the permissions dialog, a name the editor refused, an operation's error
//! with "Dismiss") and the inline name editor. Nothing here is a modal.

use hoprs_core::{EntryKind, Error};
use iced::widget::{button, checkbox, column, container, row, space, text, text_input};
use iced::{Alignment, Element, Length, Padding};

use crate::app::actions::{BusyKind, Edit, EditKind, PERMISSION_BITS, PaneAction, Permissions};
use crate::app::files::Pane;
use crate::app::{Message, PaneMsg, Tab, TabId};
use crate::backend::Side;
use crate::ui::files::edit_id;
use crate::ui::strings;
use crate::ui::theme::{self, palette};
use crate::ui::virtual_list::ROW_HEIGHT;

/// The toolbar under the path bar: the actions, each with its key, and on the right the
/// operation in flight.
pub fn toolbar<'a>(tab: &'a Tab, side: Side) -> Element<'a, Message> {
    let p = palette();
    let pane = tab.pane(side);
    let msg = move |msg: PaneMsg| Message::Pane {
        tab: tab.id,
        side,
        msg,
    };
    let can = tab.can_act(side);
    let some = can && !pane.selected_entries().is_empty();
    let chmod = can && !pane.permission_targets().is_empty();
    let action = |label: &'a str, key: Option<&'a str>, enabled: bool, pane_msg: PaneMsg| {
        let mut content = row![text(label).size(theme::TEXT_XS)].spacing(theme::SPACE_2XS);
        if let Some(key) = key {
            content = content.push(
                text(key)
                    .font(theme::MONO)
                    .size(theme::TEXT_TAG)
                    .color(p.rule_2),
            );
        }
        button(content.align_y(Alignment::Center))
            .padding(Padding::from([theme::SPACE_3XS, theme::SPACE_XS]))
            .style(theme::button_text)
            .on_press_maybe(enabled.then(|| msg(pane_msg)))
    };
    let (transfer, transfer_key) = match side {
        Side::Local => (strings::UPLOAD, strings::KEY_TRANSFER),
        Side::Remote => (strings::DOWNLOAD, strings::KEY_TRANSFER),
    };
    let mut bar = row![
        action(
            transfer,
            Some(transfer_key),
            tab.can_transfer(side),
            PaneMsg::Transfer
        ),
        action(
            strings::NEW_FOLDER,
            Some(strings::KEY_NEW_FOLDER),
            can,
            PaneMsg::NewFolder
        ),
        action(
            strings::RENAME,
            Some(strings::KEY_RENAME),
            some,
            PaneMsg::Rename
        ),
        action(
            strings::DELETE,
            Some(strings::KEY_DELETE),
            some,
            PaneMsg::Delete
        ),
    ];
    if side == Side::Remote {
        bar = bar.push(action(
            strings::PERMISSIONS,
            None,
            chmod,
            PaneMsg::Permissions,
        ));
    }
    bar = bar.push(space().width(Length::Fill));
    if let Some(busy) = &pane.busy {
        let label = match busy.kind {
            BusyKind::NewFolder => strings::CREATING_FOLDER.to_owned(),
            BusyKind::Rename => strings::RENAMING.to_owned(),
            BusyKind::Delete => strings::deleting(busy.done),
            BusyKind::Permissions => strings::CHANGING_PERMISSIONS.to_owned(),
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
        .padding(Padding::ZERO.left(theme::SPACE_XS).right(theme::SPACE_MD))
        .into()
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
    bars
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
            .style(theme::file_row(true, false, false))
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
    let mut lines = column![text(&error.message).size(theme::TEXT_XS).color(p.error)]
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
        button(text(strings::DISMISS).size(theme::TEXT_XS))
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
                text(strings::PATH_SEPARATOR)
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
            text(strings::DELETE_FOLDERS_NOTE)
                .size(theme::TEXT_XS)
                .color(p.muted),
        );
    }
    bar(row![
        lines,
        button(text(strings::DELETE).font(theme::INTER_SEMIBOLD))
            .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_MD]))
            .style(theme::button_primary)
            .on_press(msg(PaneMsg::ConfirmDelete)),
        button(text(strings::KEEP))
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
        label(strings::PERM_READ, BOX),
        label(strings::PERM_WRITE, BOX),
        label(strings::PERM_EXECUTE, BOX),
    ]]
    .spacing(theme::SPACE_2XS);
    for (who, bits) in [
        strings::PERM_OWNER,
        strings::PERM_GROUP,
        strings::PERM_OTHERS,
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
    let mut names = row![text(strings::PERMISSIONS_TITLE).size(theme::TEXT_SM)]
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
            text(strings::MIXED_PERMISSIONS)
                .size(theme::TEXT_XS)
                .color(p.muted),
        );
    }
    let mut octal = column![
        text(strings::PERM_OCTAL)
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
            text(strings::OCTAL_INVALID)
                .size(theme::TEXT_XS)
                .color(p.error),
        );
    }
    bar(column![
        row![
            container(names.wrap()).width(Length::Fill),
            button(text(strings::APPLY).font(theme::INTER_SEMIBOLD))
                .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_MD]))
                .style(theme::button_primary)
                .on_press_maybe((!dialog.invalid).then(|| msg(PaneMsg::ApplyPermissions))),
            button(text(strings::CANCEL))
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
