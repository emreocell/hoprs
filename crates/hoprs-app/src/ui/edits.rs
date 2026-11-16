//! Remote editing, as drawn ([`crate::app::edit`]): the thin strip of a files tab (one
//! line per active edit, the inline conflict choice), a tab's close question, the
//! window's close question in the center, and the toast. Nothing here is a modal.

use std::time::SystemTime;

use iced::widget::{button, column, container, row, scrollable, space, text};
use iced::{Alignment, Element, Length, Padding};

use crate::app::edit::{
    CloseAnswer, EditMsg, EditStatus, EditsFor, Question, QuestionFile, RemoteEdit,
};
use crate::app::{App, Message, Tab};
use crate::backend::EditChoice;
use crate::ui::strings;
use crate::ui::theme::{self, palette};
use crate::ui::widgets;

/// `14:02`, local time.
pub fn clock(at: SystemTime) -> String {
    chrono::DateTime::<chrono::Local>::from(at)
        .format("%H:%M")
        .to_string()
}

/// What an edit's line says (its buttons aside), as the text before the file name, the
/// name (drawn in mono) and the text after it.
pub fn edit_line_parts(edit: &RemoteEdit) -> (String, String, String) {
    let name = edit.name().to_owned();
    let editing = |after: String| (format!("{} ", strings::editing()), after);
    let (before, after) = match &edit.status {
        EditStatus::Editing { saved: None } => editing(String::new()),
        EditStatus::Editing { saved: Some(at) } => {
            editing(format!(" — {}", strings::saved_at(&clock(*at))))
        }
        EditStatus::Waiting { truncated } => {
            let mut after = format!(" — {}", strings::waiting_for_connection());
            if *truncated {
                after.push(' ');
                after.push_str(strings::remote_may_be_truncated());
            }
            editing(after)
        }
        EditStatus::Failed { error, truncated } => {
            let mut after = format!(": {}", strings::upload_failed(&strings::error(error)));
            if *truncated {
                after.push(' ');
                after.push_str(strings::remote_may_be_truncated());
            }
            (String::new(), after)
        }
        EditStatus::Conflict { .. } => {
            (String::new(), format!(": {}", strings::changed_on_server()))
        }
        EditStatus::Replaced { .. } => editing(format!(" — {}", strings::reload_in_editor())),
    };
    (before, name, after)
}

/// The whole line as one string (what [`edit_line_parts`] draws).
#[cfg(test)]
pub fn edit_line(edit: &RemoteEdit) -> String {
    let (before, name, after) = edit_line_parts(edit);
    format!("{before}{name}{after}")
}

/// Text, a file name in mono, text: one line of a notice.
fn with_name<'a>(
    before: String,
    name: String,
    after: String,
    color: iced::Color,
) -> Element<'a, Message> {
    row![
        text(before).size(theme::TEXT_XS).color(color),
        text(name)
            .font(theme::MONO)
            .size(theme::TEXT_MONO)
            .color(color),
        text(after).size(theme::TEXT_XS).color(color),
    ]
    .align_y(Alignment::Center)
    .into()
}

fn small_button<'a>(
    label: &'a str,
    on_press: Option<Message>,
    primary: bool,
) -> Element<'a, Message> {
    let enabled = on_press.is_some();
    let button = button(text(label).size(theme::TEXT_XS))
        .padding(Padding::from([theme::SPACE_3XS, theme::SPACE_XS]))
        .on_press_maybe(on_press);
    if primary {
        button.style(theme::button_primary).into()
    } else {
        widgets::outlined(button, enabled)
    }
}

fn edit_msg(msg: EditMsg) -> Message {
    Message::Edit(msg)
}

/// A path, as data (mono, muted).
fn path_text<'a>(path: String) -> Element<'a, Message> {
    text(path)
        .font(theme::MONO)
        .size(theme::TEXT_MONO)
        .color(palette().muted)
        .into()
}

/// One edit's line, with its buttons and, below, its details (an error, where the user's
/// version was saved).
fn line(edit: &RemoteEdit) -> Element<'_, Message> {
    let p = palette();
    let color = match edit.status {
        EditStatus::Failed { .. } | EditStatus::Conflict { .. } => p.ink,
        _ => p.ink_2,
    };
    let (before, name, after) = edit_line_parts(edit);
    let mut head = row![container(with_name(before, name, after, color)).width(Length::Fill)]
        .spacing(theme::SPACE_XS)
        .align_y(Alignment::Center);
    let mut details = column![];
    let id = edit.id;
    match &edit.status {
        EditStatus::Failed { .. } => {
            head = head.push(small_button(
                strings::retry(),
                Some(edit_msg(EditMsg::Retry(id))),
                false,
            ));
        }
        EditStatus::Conflict {
            busy,
            error,
            recovered,
        } => {
            let choose = |choice| (!busy).then(|| edit_msg(EditMsg::Resolve { id, choice }));
            head = head
                .push(small_button(
                    strings::overwrite(),
                    choose(EditChoice::Overwrite),
                    false,
                ))
                .push(small_button(
                    strings::keep_remote(),
                    choose(EditChoice::KeepRemote),
                    false,
                ))
                .push(small_button(
                    strings::save_a_copy(),
                    choose(EditChoice::SaveCopy),
                    false,
                ));
            if let Some(error) = error {
                details = details.push(
                    text(strings::error(error))
                        .size(theme::TEXT_XS)
                        .color(p.error),
                );
            }
            if let Some(path) = recovered {
                details = details.push(path_text(strings::your_version_at(
                    &path.display().to_string(),
                )));
            }
        }
        EditStatus::Replaced {
            recovered: Some(path),
        } => {
            details = details.push(path_text(strings::your_version_at(
                &path.display().to_string(),
            )));
        }
        _ => {}
    }
    column![head, details].spacing(theme::SPACE_3XS).into()
}

/// The strip across a files tab: a line per edit of its session (and per file being
/// opened). `None` without any.
pub fn strip<'a>(app: &'a App, tab: &'a Tab) -> Option<Element<'a, Message>> {
    let session = app.conn(&tab.site).session()?;
    let edits = app.session_edits(session);
    let opening: Vec<_> = app
        .edit_opening
        .iter()
        .filter(|o| o.session == session)
        .collect();
    if edits.is_empty() && opening.is_empty() {
        return None;
    }
    let mut lines = column![].spacing(theme::SPACE_2XS);
    for edit in edits {
        lines = lines.push(line(edit));
    }
    for o in opening {
        let name = o.remote.file_name().unwrap_or(o.remote.as_str());
        lines = lines.push(with_name(
            format!("{} ", strings::opening()),
            name.to_owned(),
            "…".to_owned(),
            palette().muted,
        ));
    }
    Some(
        container(lines)
            .width(Length::Fill)
            .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_MD]))
            .style(theme::pane_bar)
            .into(),
    )
}

/// The files a question lists, each with its state.
fn files_list(files: &[QuestionFile]) -> Element<'_, Message> {
    let mut list = column![].spacing(theme::SPACE_3XS);
    for file in files {
        let state = if file.conflict {
            Some(strings::edit_conflict())
        } else if file.unsent {
            Some(strings::edit_unsent())
        } else {
            None
        };
        let mut entry = row![
            text(file.remote.as_str().to_owned())
                .font(theme::MONO)
                .size(theme::TEXT_MONO)
        ]
        .spacing(theme::SPACE_XS);
        if let Some(state) = state {
            entry = entry.push(text(state).size(theme::TEXT_XS).color(palette().muted));
        }
        list = list.push(entry);
    }
    list.into()
}

/// A question's body: the files, where kept copies go, what failed, and the answers.
fn question_body<'a>(
    question: &'a Question,
    title: &'a str,
    labels: [&'a str; 2],
    answer: impl Fn(CloseAnswer) -> Message + 'a,
) -> Element<'a, Message> {
    let p = palette();
    let mut body = column![text(title).font(theme::INTER_SEMIBOLD)].spacing(theme::SPACE_XS);
    let Some(files) = &question.files else {
        return body
            .push(
                text(strings::checking_edits())
                    .size(theme::TEXT_XS)
                    .color(p.muted),
            )
            .into();
    };
    body = body.push(files_list(files));
    if let Some(dir) = &question.kept_in {
        body = body.push(path_text(strings::copies_go_to(&dir.display().to_string())));
    }
    if question.not_uploaded > 0 {
        body = body.push(
            text(strings::not_uploaded(question.not_uploaded))
                .size(theme::TEXT_XS)
                .color(p.error),
        );
    }
    if let Some(error) = &question.error {
        body = body.push(
            text(strings::error(error))
                .size(theme::TEXT_XS)
                .color(p.error),
        );
    }
    let idle = question.busy.is_none();
    let on = |a: CloseAnswer| idle.then(|| answer(a));
    let mut actions = if question.busy == Some(CloseAnswer::Upload) {
        // The upload may take its full limit: keeping instead stays one click away.
        row![small_button(
            strings::stop_keep_copies(),
            Some(answer(CloseAnswer::Keep)),
            false
        )]
    } else {
        row![
            small_button(labels[0], on(CloseAnswer::Upload), true),
            small_button(labels[1], on(CloseAnswer::Keep), false),
            small_button(strings::cancel(), on(CloseAnswer::Cancel), false),
        ]
    }
    .spacing(theme::SPACE_XS)
    .align_y(Alignment::Center);
    match question.busy {
        Some(CloseAnswer::Upload) => {
            actions = actions.push(
                text(strings::uploading_edits())
                    .size(theme::TEXT_XS)
                    .color(p.muted),
            );
        }
        Some(CloseAnswer::Keep) => {
            actions = actions.push(
                text(strings::keeping_copies())
                    .size(theme::TEXT_XS)
                    .color(p.muted),
            );
        }
        _ => {}
    }
    body.push(actions).into()
}

/// A tab's close question, inline at its top.
pub fn close_ask<'a>(app: &'a App, tab: &'a Tab) -> Option<Element<'a, Message>> {
    let question = app.close_asks.get(&tab.id)?;
    let id = tab.id;
    Some(
        container(question_body(
            question,
            strings::close_edits_title(),
            [strings::upload_and_close(), strings::close_keep_copies()],
            move |answer| edit_msg(EditMsg::CloseTab { tab: id, answer }),
        ))
        .width(Length::Fill)
        .padding(Padding::from([theme::SPACE_XS, theme::SPACE_MD]))
        .style(theme::pane_bar)
        .into(),
    )
}

/// The question of the window or of "Close workspace", in the center (like the
/// settings page).
pub fn quit_view(app: &App) -> Element<'_, Message> {
    let Some(ask) = &app.center_ask else {
        return space().into();
    };
    let (title, labels) = match ask.purpose {
        EditsFor::Workspace(_) => (
            strings::close_workspace_edits_title(),
            [strings::upload_and_close(), strings::close_keep_copies()],
        ),
        _ => (
            strings::quit_edits_title(),
            [strings::upload_and_quit(), strings::quit_keep_copies()],
        ),
    };
    let content = column![
        question_body(&ask.question, title, labels, |answer| {
            edit_msg(EditMsg::Center(answer))
        }),
        text(strings::quit_edits_hint())
            .size(theme::TEXT_XS)
            .color(palette().muted),
    ]
    .spacing(theme::SPACE_MD)
    .max_width(640);
    scrollable(
        container(content)
            .width(Length::Fill)
            .padding(theme::SPACE_LG),
    )
    .height(Length::Fill)
    .into()
}

/// The toast, bottom right of `base` (no animation: it appears and goes).
pub fn with_toast<'a>(app: &'a App, base: Element<'a, Message>) -> Element<'a, Message> {
    let Some(toast) = &app.toast else {
        return base;
    };
    let notice = container(with_name(
        format!("{} ", toast.text),
        toast.name.clone(),
        String::new(),
        palette().ink,
    ))
    .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_SM]))
    .style(theme::dialog_box);
    let layer = container(notice)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(theme::SPACE_MD)
        .align_x(Alignment::End)
        .align_y(Alignment::End);
    iced::widget::stack![base, layer].into()
}

/// The notice of unsent edits a close kept as recovered copies: above the center, from
/// any tab, until dismissed.
pub fn kept_notice(app: &App) -> Option<Element<'_, Message>> {
    let notice = app.kept_notice.as_ref()?;
    let mut lines = column![
        row![
            text(strings::kept_notice(notice.count)).size(theme::TEXT_XS),
            container(path_text(notice.dir.display().to_string())).width(Length::Fill),
            small_button(
                strings::dismiss(),
                Some(edit_msg(EditMsg::DismissKept)),
                false
            ),
        ]
        .spacing(theme::SPACE_XS)
        .align_y(Alignment::Center)
    ]
    .spacing(theme::SPACE_3XS);
    if notice.closed_unasked {
        lines = lines.push(text(strings::kept_saves_stopped()).size(theme::TEXT_XS));
    }
    for remote in &notice.incomplete {
        lines = lines.push(with_name(
            format!("{} ", strings::server_copy_of()),
            remote.as_str().to_owned(),
            format!(" {}", strings::may_be_incomplete()),
            palette().error,
        ));
    }
    Some(
        container(lines)
            .width(Length::Fill)
            .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_MD]))
            .style(theme::pane_bar)
            .into(),
    )
}
