//! The security and credential dialogs (the app's only modals) and the changed host key
//! explanation shown in a tab.
//!
//! Each dialog is described by a [`Content`] (what it says and asks), built from its
//! request by [`content`]; [`view`] draws it. The tests check the content.

use iced::widget::{
    button, checkbox, column, container, opaque, row, space, stack, text, text_input,
};
use iced::{Alignment, Element, Length, Padding};

use crate::app::{
    App, Dialog, DialogMsg, FailureAction, KnownHostsFix, KnownHostsMsg, Message, Tab, Typed,
};
use crate::backend::KnownHostEntry;
use crate::backend::broker::PromptKind;
use crate::ui::strings;
use crate::ui::theme::{self, palette};
use hoprs_remote::CertProblem;

/// The dialog box's width.
const WIDTH: f32 = 460.0;

/// The id of a dialog's field `index` (the first one gets the focus when shown).
pub fn field_id(index: usize) -> iced::widget::Id {
    iced::widget::Id::from(format!("dialog-field-{index}"))
}

/// What a dialog says and asks.
#[derive(Debug, Clone, PartialEq)]
pub struct Content {
    pub title: String,
    pub intro: Option<String>,
    /// Label and value; values are data (host, fingerprint, path), shown in mono.
    pub rows: Vec<(&'static str, String)>,
    /// Shown in the error colour.
    pub warning: Option<String>,
    /// The inputs: label and whether the answer may be shown.
    pub fields: Vec<(String, bool)>,
    /// Offers "Save in the system keyring".
    pub save: bool,
    /// The action besides Cancel: "Trust and connect", "Connect"…
    pub action: &'static str,
    /// A trust decision: Cancel is the default (white) button and Enter means Cancel.
    pub decision: bool,
}

/// The content of `dialog`.
pub fn content(dialog: &Dialog) -> Content {
    let request = &dialog.request;
    let base = Content {
        title: String::new(),
        intro: None,
        rows: Vec::new(),
        warning: None,
        fields: Vec::new(),
        save: dialog.can_save(),
        action: strings::CONNECT,
        decision: dialog.is_decision(),
    };
    match &request.kind {
        PromptKind::HostKey {
            key_type,
            fingerprint,
            other_key_types,
            ..
        } => Content {
            title: strings::HOST_KEY_TITLE.to_owned(),
            intro: Some(strings::HOST_KEY_INTRO.to_owned()),
            rows: vec![
                (strings::LABEL_HOST, request.key.clone()),
                (strings::LABEL_KEY_TYPE, key_type.clone()),
                (strings::LABEL_FINGERPRINT, fingerprint.clone()),
            ],
            warning: (!other_key_types.is_empty())
                .then(|| strings::other_key_types(other_key_types)),
            action: strings::TRUST_AND_CONNECT,
            ..base
        },
        PromptKind::Certificate {
            fingerprint,
            problem,
            ..
        } => {
            let host = (strings::LABEL_HOST, request.key.clone());
            match problem {
                CertProblem::Untrusted { reason } => Content {
                    title: strings::CERT_UNTRUSTED_TITLE.to_owned(),
                    intro: Some(strings::CERT_UNTRUSTED_INTRO.to_owned()),
                    rows: vec![
                        host,
                        (strings::LABEL_REASON, strings::cert_reason(reason)),
                        (strings::LABEL_FINGERPRINT, fingerprint.clone()),
                    ],
                    action: strings::TRUST_CERTIFICATE,
                    ..base
                },
                CertProblem::Changed { pinned, reason } => Content {
                    title: strings::CERT_CHANGED_TITLE.to_owned(),
                    intro: None,
                    rows: vec![
                        host,
                        (strings::LABEL_REASON, strings::cert_reason(reason)),
                        (strings::LABEL_TRUSTED_BEFORE, pinned.clone()),
                        (strings::LABEL_PRESENTED_NOW, fingerprint.clone()),
                    ],
                    warning: Some(strings::CERT_CHANGED_INTRO.to_owned()),
                    action: strings::TRUST_CERTIFICATE,
                    ..base
                },
            }
        }
        PromptKind::Password { user, .. } => Content {
            title: strings::PASSWORD_TITLE.to_owned(),
            rows: vec![(strings::LABEL_ACCOUNT, format!("{user}@{}", request.key))],
            fields: vec![(strings::FIELD_PASSWORD.to_owned(), false)],
            ..base
        },
        PromptKind::Passphrase { path } => Content {
            title: strings::PASSPHRASE_TITLE.to_owned(),
            rows: vec![
                (strings::LABEL_HOST, request.key.clone()),
                (strings::LABEL_KEY_FILE, path.display().to_string()),
            ],
            fields: vec![(strings::FIELD_PASSPHRASE.to_owned(), false)],
            action: strings::UNLOCK,
            ..base
        },
        PromptKind::KeyboardInteractive {
            name,
            instructions,
            prompts,
        } => Content {
            title: if name.trim().is_empty() {
                strings::KBD_TITLE.to_owned()
            } else {
                name.clone()
            },
            intro: (!instructions.trim().is_empty()).then(|| instructions.clone()),
            rows: vec![(strings::LABEL_HOST, request.key.clone())],
            fields: prompts
                .iter()
                .map(|p| (p.text.trim().to_owned(), p.echo))
                .collect(),
            action: strings::CONTINUE,
            ..base
        },
    }
}

/// `base` with the open dialog over it, if any: a backdrop that takes every click, and
/// the dialog box in the middle.
pub fn overlay<'a>(app: &'a App, base: Element<'a, Message>) -> Element<'a, Message> {
    let Some(dialog) = &app.dialogs.current else {
        return base;
    };
    let waiting = app.dialogs.queued().count();
    let mut boxed = column![view(dialog)]
        .spacing(theme::SPACE_XS)
        .align_x(Alignment::Center);
    if waiting > 0 {
        boxed = boxed.push(
            text(strings::dialogs_waiting(waiting))
                .size(theme::TEXT_XS)
                .color(palette().muted),
        );
    }
    let layer = opaque(container(boxed).center(Length::Fill).style(theme::backdrop));
    stack![base, layer].into()
}

fn msg(m: DialogMsg) -> Message {
    Message::Dialog(m)
}

/// The dialog box.
pub fn view(dialog: &Dialog) -> Element<'_, Message> {
    let p = palette();
    let content = content(dialog);
    let mut body = column![
        text(content.title)
            .size(theme::TEXT_MD)
            .font(theme::INTER_SEMIBOLD)
    ]
    .spacing(theme::SPACE_SM);
    if let Some(intro) = content.intro {
        body = body.push(text(intro).size(theme::TEXT_SM).color(p.ink_2));
    }
    for (label, value) in content.rows {
        body = body.push(
            column![
                text(label).size(theme::TEXT_XS).color(p.muted),
                text(value)
                    .font(theme::MONO)
                    .size(theme::TEXT_MONO)
                    .wrapping(text::Wrapping::WordOrGlyph),
            ]
            .spacing(theme::SPACE_3XS),
        );
    }
    if let Some(warning) = content.warning {
        body = body.push(text(warning).size(theme::TEXT_SM).color(p.error));
    }
    for (index, ((label, echo), typed)) in
        content.fields.into_iter().zip(&dialog.fields).enumerate()
    {
        let input = text_input("", &typed.0)
            .id(field_id(index))
            .on_input(move |v| msg(DialogMsg::Input(index, Typed(v))))
            .on_submit(msg(DialogMsg::Submit))
            .secure(!echo)
            .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_XS]))
            .size(theme::TEXT_SM)
            .style(theme::search);
        body = body.push(
            column![text(label).size(theme::TEXT_XS).color(p.muted), input]
                .spacing(theme::SPACE_2XS),
        );
    }
    if content.save {
        body = body.push(
            column![
                checkbox(dialog.save)
                    .label(strings::SAVE_IN_KEYRING)
                    .text_size(theme::TEXT_SM)
                    .style(theme::checkbox)
                    .on_toggle(|v| msg(DialogMsg::Save(v))),
                text(strings::SAVE_IN_KEYRING_HINT)
                    .size(theme::TEXT_XS)
                    .color(p.muted),
            ]
            .spacing(theme::SPACE_3XS),
        );
    }
    body = body.push(buttons(content.action, content.decision));
    container(body)
        .width(WIDTH)
        .padding(theme::SPACE_LG)
        .style(theme::dialog_box)
        .into()
}

/// A decision: "Trust…" outlined, then Cancel as the default (white) button. A
/// credential: Cancel as text, then the action in white.
fn buttons<'a>(action: &'a str, decision: bool) -> Element<'a, Message> {
    let pad = Padding::from([theme::SPACE_2XS, theme::SPACE_MD]);
    let cancel = button(text(strings::CANCEL).font(theme::INTER_SEMIBOLD))
        .padding(pad)
        .on_press(msg(DialogMsg::Cancel));
    let row = if decision {
        row![
            space::horizontal(),
            button(text(action))
                .padding(pad)
                .style(theme::button_outline)
                .on_press(msg(DialogMsg::Trust)),
            cancel.style(theme::button_primary),
        ]
    } else {
        row![
            space::horizontal(),
            cancel.style(theme::button_text),
            button(text(action).font(theme::INTER_SEMIBOLD))
                .padding(pad)
                .style(theme::button_primary)
                .on_press(msg(DialogMsg::Submit)),
        ]
    };
    row.spacing(theme::SPACE_XS)
        .align_y(Alignment::Center)
        .into()
}

/// The warning to show before removing `entries`: some also name other hosts.
pub fn known_hosts_warning(entries: &[KnownHostEntry]) -> Option<String> {
    let lines: Vec<usize> = entries
        .iter()
        .filter(|e| e.covers_other_hosts)
        .map(|e| e.line)
        .collect();
    (!lines.is_empty()).then(|| strings::also_covers_other_hosts(&lines))
}

/// The tab's blocking explanation of a changed host key: no way to accept the key, only
/// "Remove the old entry…" (with its confirmation in place) or "Edit site".
pub fn host_key_changed<'a>(app: &'a App, tab: &'a Tab) -> Element<'a, Message> {
    let p = palette();
    let fix = |m: KnownHostsMsg| Message::KnownHosts {
        tab: tab.id,
        msg: m,
    };
    let pad = Padding::from([theme::SPACE_2XS, theme::SPACE_XS]);
    let line = tab.host_key_change().and_then(|change| change.line);
    let explanation = if line.is_some() {
        strings::HOST_KEY_CHANGED_BODY
    } else {
        strings::HOST_KEY_CHANGED_SESSION_BODY
    };
    let mut body = column![
        text(strings::HOST_KEY_CHANGED_TITLE)
            .font(theme::INTER_SEMIBOLD)
            .color(p.error),
        text(explanation).size(theme::TEXT_SM).color(p.ink_2),
    ]
    .spacing(theme::SPACE_XS)
    .max_width(WIDTH);
    if let Some(line) = line {
        body = body.push(
            text(strings::known_hosts_line(line))
                .font(theme::MONO)
                .size(theme::TEXT_MONO)
                .color(p.muted),
        );
    }
    let host = tab
        .failed_endpoint
        .as_ref()
        .map(|(host, port)| format!("{host}:{port}"))
        .unwrap_or_default();
    let mut actions = row![].spacing(theme::SPACE_XS);
    for action in app.failure_actions(tab) {
        let (label, style, on_press): (_, fn(&iced::Theme, _) -> _, _) = match action {
            FailureAction::RemoveOldEntry => (
                strings::REMOVE_OLD_ENTRY,
                theme::button_outline,
                fix(KnownHostsMsg::Start),
            ),
            FailureAction::CloseAndReconnect => (
                strings::CLOSE_AND_RECONNECT,
                theme::button_outline,
                Message::RestartSession(tab.id),
            ),
            FailureAction::EditSite => (
                strings::EDIT_SITE,
                theme::button_text,
                Message::EditSite(tab.site.clone()),
            ),
            FailureAction::Reconnect => continue,
        };
        actions = actions.push(
            button(text(label))
                .padding(pad)
                .style(style)
                .on_press(on_press),
        );
    }
    let muted = |label: &'a str| text(label).size(theme::TEXT_SM).color(p.muted);
    body = match &tab.known_hosts {
        KnownHostsFix::Idle => body.push(actions),
        KnownHostsFix::Checking => body.push(muted(strings::READING_KNOWN_HOSTS)),
        KnownHostsFix::Removing => body.push(muted(strings::REMOVING_KNOWN_HOSTS)),
        KnownHostsFix::Failed(error) => body
            .push(crate::ui::widgets::failure(Some(error)))
            .push(actions),
        KnownHostsFix::Confirm(entries) => body.push(confirm(entries, host, fix)),
    };
    container(body).center(Length::Fill).into()
}

/// "Remove N lines for host from known_hosts?", the lines, the warning, Remove / Keep.
fn confirm<'a>(
    entries: &'a [KnownHostEntry],
    host: String,
    fix: impl Fn(KnownHostsMsg) -> Message,
) -> Element<'a, Message> {
    let p = palette();
    let pad = Padding::from([theme::SPACE_2XS, theme::SPACE_MD]);
    let (question, yes) = if entries.is_empty() {
        (
            strings::NO_KNOWN_HOSTS_ENTRY.to_owned(),
            strings::CONNECT_AGAIN,
        )
    } else {
        (
            strings::remove_known_hosts_question(entries.len(), &host),
            strings::REMOVE,
        )
    };
    let mut body =
        column![text(question).size(theme::TEXT_SM).color(p.ink_2)].spacing(theme::SPACE_XS);
    for entry in entries {
        body = body.push(
            text(format!(
                "{}  {}",
                strings::known_hosts_line(entry.line),
                entry.key_type
            ))
            .font(theme::MONO)
            .size(theme::TEXT_MONO)
            .color(p.ink_2),
        );
    }
    if let Some(warning) = known_hosts_warning(entries) {
        body = body.push(text(warning).size(theme::TEXT_SM).color(p.error));
    }
    body = body.push(
        row![
            button(text(yes).font(theme::INTER_SEMIBOLD))
                .padding(pad)
                .style(theme::button_primary)
                .on_press(fix(KnownHostsMsg::Confirm)),
            button(text(strings::KEEP))
                .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_XS]))
                .style(theme::button_text)
                .on_press(fix(KnownHostsMsg::Cancel)),
        ]
        .spacing(theme::SPACE_XS),
    );
    container(body)
        .padding(theme::SPACE_SM)
        .style(theme::outline_box)
        .into()
}
