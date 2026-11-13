//! Remote editing, as drawn ([`crate::app::edit`]): the thin strip of a files tab (one
//! line per active edit, the inline conflict choice), a tab's close question, the
//! window's close question in the center, and the toast. Nothing here is a modal.

use std::time::SystemTime;

use iced::widget::{button, column, container, row, scrollable, space, text};
use iced::{Alignment, Element, Length, Padding};

use crate::app::edit::{CloseAnswer, EditMsg, EditStatus, Question, QuestionFile, RemoteEdit};
use crate::app::{App, Message, Tab};
use crate::backend::EditChoice;
use crate::ui::strings;
use crate::ui::theme::{self, palette};

/// `14:02`, local time.
pub fn clock(at: SystemTime) -> String {
    chrono::DateTime::<chrono::Local>::from(at)
        .format("%H:%M")
        .to_string()
}

/// What an edit's line says (its buttons aside).
pub fn edit_line(edit: &RemoteEdit) -> String {
    let name = edit.name();
    match &edit.status {
        EditStatus::Editing { saved: None } => strings::editing(name),
        EditStatus::Editing { saved: Some(at) } => strings::editing_saved(name, &clock(*at)),
        EditStatus::Waiting { truncated } => {
            let mut line = format!(
                "{} — {}",
                strings::editing(name),
                strings::WAITING_FOR_CONNECTION
            );
            if *truncated {
                line.push(' ');
                line.push_str(strings::REMOTE_MAY_BE_TRUNCATED);
            }
            line
        }
        EditStatus::Failed { error, truncated } => {
            let mut line = format!("{name}: {}", strings::upload_failed(&error.message));
            if *truncated {
                line.push(' ');
                line.push_str(strings::REMOTE_MAY_BE_TRUNCATED);
            }
            line
        }
        EditStatus::Conflict { .. } => format!("{name}: {}", strings::CHANGED_ON_SERVER),
        EditStatus::Replaced { .. } => {
            format!("{} — {}", strings::editing(name), strings::RELOAD_IN_EDITOR)
        }
    }
}

fn small_button<'a>(
    label: &'a str,
    on_press: Option<Message>,
    primary: bool,
) -> Element<'a, Message> {
    button(text(label).size(theme::TEXT_XS))
        .padding(Padding::from([theme::SPACE_3XS, theme::SPACE_XS]))
        .style(if primary {
            theme::button_primary
        } else {
            theme::button_outline
        })
        .on_press_maybe(on_press)
        .into()
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
    let mut head = row![
        text(edit_line(edit))
            .size(theme::TEXT_XS)
            .color(color)
            .width(Length::Fill)
    ]
    .spacing(theme::SPACE_XS)
    .align_y(Alignment::Center);
    let mut details = column![];
    let id = edit.id;
    match &edit.status {
        EditStatus::Failed { .. } => {
            head = head.push(small_button(
                strings::RETRY,
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
                    strings::OVERWRITE,
                    choose(EditChoice::Overwrite),
                    false,
                ))
                .push(small_button(
                    strings::KEEP_REMOTE,
                    choose(EditChoice::KeepRemote),
                    false,
                ))
                .push(small_button(
                    strings::SAVE_A_COPY,
                    choose(EditChoice::SaveCopy),
                    false,
                ));
            if let Some(error) = error {
                details = details.push(
                    text(error.message.clone())
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
        lines = lines.push(
            text(strings::opening_edit(name))
                .size(theme::TEXT_XS)
                .color(palette().muted),
        );
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
            Some(strings::EDIT_CONFLICT)
        } else if file.unsent {
            Some(strings::EDIT_UNSENT)
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
                text(strings::CHECKING_EDITS)
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
            text(error.message.clone())
                .size(theme::TEXT_XS)
                .color(p.error),
        );
    }
    let idle = question.busy.is_none();
    let on = |a: CloseAnswer| idle.then(|| answer(a));
    let mut actions = row![
        small_button(labels[0], on(CloseAnswer::Upload), true),
        small_button(labels[1], on(CloseAnswer::Keep), false),
        small_button(strings::CANCEL, on(CloseAnswer::Cancel), false),
    ]
    .spacing(theme::SPACE_XS)
    .align_y(Alignment::Center);
    match question.busy {
        Some(CloseAnswer::Upload) => {
            actions = actions.push(
                text(strings::UPLOADING_EDITS)
                    .size(theme::TEXT_XS)
                    .color(p.muted),
            );
        }
        Some(CloseAnswer::Keep) => {
            actions = actions.push(
                text(strings::KEEPING_COPIES)
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
            strings::CLOSE_EDITS_TITLE,
            [strings::UPLOAD_AND_CLOSE, strings::CLOSE_KEEP_COPIES],
            move |answer| edit_msg(EditMsg::CloseTab { tab: id, answer }),
        ))
        .width(Length::Fill)
        .padding(Padding::from([theme::SPACE_XS, theme::SPACE_MD]))
        .style(theme::pane_bar)
        .into(),
    )
}

/// The window's close question, in the center (like the settings page).
pub fn quit_view(app: &App) -> Element<'_, Message> {
    let Some(quit) = &app.quit else {
        return space().into();
    };
    let content = column![
        question_body(
            &quit.question,
            strings::QUIT_EDITS_TITLE,
            [strings::UPLOAD_AND_QUIT, strings::QUIT_KEEP_COPIES],
            |answer| edit_msg(EditMsg::Quit(answer)),
        ),
        text(strings::QUIT_EDITS_HINT)
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
    let notice = container(text(toast.text.as_str()).size(theme::TEXT_XS))
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
