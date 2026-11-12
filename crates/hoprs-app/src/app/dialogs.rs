//! The security and credential dialogs (host key, certificate, password, passphrase,
//! keyboard-interactive): the only modals of the app.
//!
//! - **One at a time, in arrival order.** The prompt broker sends a request per dialog;
//!   the first is shown, the others wait in a FIFO queue. A withdrawn request (its
//!   session closed, its connection attempt ended) disappears, shown or queued, without
//!   an answer.
//! - **Closing is refusing.** Esc, Cancel and the close of a dialog answer
//!   [`PromptAnswer::Cancel`].
//! - **The safe button is the default.** iced 0.14 buttons take no keyboard focus, so
//!   Enter in a host key or certificate dialog means Cancel (the one drawn as the default
//!   button); trusting needs a click on "Trust…". Enter in a credential dialog submits it.
//! - **Typed secrets leave the UI on submit.** The fields are taken out of the state into
//!   the [`Effect::AnswerPrompt`]'s `SecretString`s; nothing typed stays in `App`. The
//!   fields' `Debug` never shows them.
//! - **"Save in the system keyring"** goes with the answer (`save: true`): the backend
//!   stores the secret only once the session connects with it. The app reloads the
//!   profiles after that connect, so the site shows its saved password.
//!
//! A *changed* host key is not a dialog: the connection fails with it, and the tab shows a
//! blocking explanation with no way to accept the new key, only "Remove the old entry…"
//! (see [`KnownHostsFix`]).

use std::collections::{HashSet, VecDeque};
use std::fmt;

use hoprs_core::Error;
use hoprs_ssh::HostKeyChange;
use secrecy::SecretString;

use super::{App, Effect, FileKey, Tab, TabId, TabState};
use crate::backend::broker::{PromptKind, PromptRequest};
use crate::backend::{BrokerEvent, KnownHostEntry, PromptAnswer, SessionId};

/// The dialog shown and the ones waiting.
#[derive(Debug, Default)]
pub struct Dialogs {
    pub current: Option<Dialog>,
    queue: VecDeque<PromptRequest>,
    /// Sessions whose typed secret was answered with "save": their next successful
    /// connect stored it, so the profiles are reloaded then.
    saving: HashSet<SessionId>,
}

impl Dialogs {
    pub fn is_open(&self) -> bool {
        self.current.is_some()
    }

    /// The requests waiting behind the current dialog, in order.
    pub fn queued(&self) -> impl Iterator<Item = &PromptRequest> {
        self.queue.iter()
    }
}

/// The open dialog: its request and what the user typed so far.
#[derive(Debug)]
pub struct Dialog {
    pub request: PromptRequest,
    /// One per input: one for a password or passphrase, one per keyboard-interactive
    /// prompt, none for a decision.
    pub fields: Vec<Typed>,
    /// "Save in the system keyring" (password and passphrase only).
    pub save: bool,
}

impl Dialog {
    fn new(request: PromptRequest) -> Self {
        let inputs = match &request.kind {
            PromptKind::HostKey { .. } | PromptKind::Certificate { .. } => 0,
            PromptKind::Password { .. } | PromptKind::Passphrase { .. } => 1,
            PromptKind::KeyboardInteractive { prompts, .. } => prompts.len(),
        };
        Self {
            request,
            fields: vec![Typed::default(); inputs],
            save: false,
        }
    }

    /// A trust decision (host key, certificate) rather than a credential.
    pub fn is_decision(&self) -> bool {
        matches!(
            self.request.kind,
            PromptKind::HostKey { .. } | PromptKind::Certificate { .. }
        )
    }

    /// Whether the dialog offers "Save in the system keyring".
    pub fn can_save(&self) -> bool {
        matches!(
            self.request.kind,
            PromptKind::Password { .. } | PromptKind::Passphrase { .. }
        )
    }
}

/// A dialog field as typed. Its `Debug` never shows it.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Typed(pub String);

impl fmt::Debug for Typed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Typed(***)")
    }
}

/// Something in the open dialog.
#[derive(Debug, Clone)]
pub enum DialogMsg {
    /// Field `index` changed.
    Input(usize, Typed),
    /// "Save in the system keyring" toggled.
    Save(bool),
    /// Send the typed credentials.
    Submit,
    /// "Trust and connect" / "Trust this certificate".
    Trust,
    /// Cancel, Esc or the dialog's close: a refusal.
    Cancel,
}

/// "Remove the old entry…" of a tab whose host key changed.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum KnownHostsFix {
    /// The explanation, with the action.
    #[default]
    Idle,
    /// Reading the `known_hosts` lines that would be removed.
    Checking,
    /// Asking to remove these lines (empty: none is left).
    Confirm(Vec<KnownHostEntry>),
    Removing,
    /// Reading or removing failed.
    Failed(Error),
}

#[derive(Debug, Clone)]
pub enum KnownHostsMsg {
    /// "Remove the old entry…": look up what would go.
    Start,
    /// Remove the lines shown.
    Confirm,
    /// Keep them.
    Cancel,
}

/// What a failed tab offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureAction {
    /// Connect again (the sidebar's connect path).
    Reconnect,
    EditSite,
    /// The host key changed: remove the old `known_hosts` entry (never accept the key).
    RemoveOldEntry,
}

impl Tab {
    /// The host key changed: why this tab failed, if so.
    pub fn host_key_change(&self) -> Option<HostKeyChange> {
        match &self.state {
            TabState::Failed(failure) => hoprs_ssh::host_key_change(&failure.error),
            TabState::Connecting | TabState::Connected(_) => None,
        }
    }
}

impl App {
    /// A request or withdrawal from the broker.
    pub(super) fn prompt_event(&mut self, event: BrokerEvent) -> Vec<Effect> {
        match event {
            BrokerEvent::Request(request) => {
                if self.dialogs.current.is_some() {
                    self.dialogs.queue.push_back(request);
                    return Vec::new();
                }
                self.show(request)
            }
            BrokerEvent::Withdrawn(id) => {
                if self.dialogs.current.as_ref().map(|d| d.request.id) == Some(id) {
                    self.dialogs.current = None;
                    return self.show_next();
                }
                self.dialogs.queue.retain(|r| r.id != id);
                Vec::new()
            }
        }
    }

    /// Shows `request`, focusing its first field if it has one.
    fn show(&mut self, request: PromptRequest) -> Vec<Effect> {
        let dialog = Dialog::new(request);
        let focus = !dialog.fields.is_empty();
        self.dialogs.current = Some(dialog);
        if focus {
            vec![Effect::FocusDialog]
        } else {
            Vec::new()
        }
    }

    fn show_next(&mut self) -> Vec<Effect> {
        match self.dialogs.queue.pop_front() {
            Some(request) => self.show(request),
            None => Vec::new(),
        }
    }

    /// Closes the open dialog with `answer` and shows the next one.
    fn close_dialog(&mut self, answer: impl FnOnce(&mut Dialog) -> PromptAnswer) -> Vec<Effect> {
        let Some(mut dialog) = self.dialogs.current.take() else {
            return Vec::new();
        };
        let answer = answer(&mut dialog);
        let mut effects = vec![Effect::AnswerPrompt {
            id: dialog.request.id,
            answer,
        }];
        effects.extend(self.show_next());
        effects
    }

    /// Something in the open dialog.
    pub(super) fn dialog_msg(&mut self, msg: DialogMsg) -> Vec<Effect> {
        let Some(dialog) = &mut self.dialogs.current else {
            return Vec::new();
        };
        match msg {
            DialogMsg::Input(index, typed) => {
                if let Some(field) = dialog.fields.get_mut(index) {
                    *field = typed;
                }
                Vec::new()
            }
            DialogMsg::Save(save) => {
                dialog.save = save && dialog.can_save();
                Vec::new()
            }
            DialogMsg::Submit if !dialog.is_decision() => {
                if dialog.save && dialog.can_save() {
                    self.dialogs.saving.insert(dialog.request.session);
                }
                self.close_dialog(answer_of)
            }
            DialogMsg::Trust if dialog.is_decision() => {
                self.close_dialog(|_| PromptAnswer::Decision(true))
            }
            DialogMsg::Cancel => self.close_dialog(|_| PromptAnswer::Cancel),
            // Trust on a credential, Submit on a decision: not this dialog's.
            DialogMsg::Submit | DialogMsg::Trust => Vec::new(),
        }
    }

    /// A key while a dialog is open: it belongs to the dialog, never to the panes. Esc
    /// refuses; Enter is the default button (Cancel for a trust decision, since iced
    /// buttons take no focus; submit for credentials); Tab moves between fields.
    pub(super) fn dialog_key(&mut self, key: FileKey) -> Vec<Effect> {
        let Some(dialog) = &self.dialogs.current else {
            return Vec::new();
        };
        match key {
            FileKey::Escape => self.dialog_msg(DialogMsg::Cancel),
            FileKey::Enter if dialog.is_decision() => self.dialog_msg(DialogMsg::Cancel),
            FileKey::Enter => self.dialog_msg(DialogMsg::Submit),
            FileKey::SwitchPane => vec![Effect::FocusNext],
            _ => Vec::new(),
        }
    }

    /// A session's connect finished: a secret answered with "save" was stored if it
    /// connected, so the profiles are reloaded.
    pub(super) fn settle_saving(&mut self, session: SessionId, connected: bool) -> Vec<Effect> {
        if self.dialogs.saving.remove(&session) && connected {
            vec![Effect::LoadProfiles]
        } else {
            Vec::new()
        }
    }

    /// What the failed tab offers: never a way to accept a changed host key.
    pub fn failure_actions(&self, tab: &Tab) -> Vec<FailureAction> {
        let mut actions = Vec::new();
        if self.site(&tab.site).is_none() {
            return actions;
        }
        if tab.host_key_change().is_some() {
            actions.push(FailureAction::RemoveOldEntry);
        } else {
            actions.push(FailureAction::Reconnect);
        }
        actions.push(FailureAction::EditSite);
        actions
    }

    /// The `host:port` of the tab whose host key changed, if it did.
    fn changed_endpoint(&self, index: usize) -> Option<(String, u16)> {
        let tab = &self.tabs[index];
        tab.host_key_change()?;
        self.site(&tab.site).map(|s| (s.host.clone(), s.port))
    }

    /// "Remove the old entry…" and its confirmation, in the tab.
    pub(super) fn known_hosts_msg(&mut self, tab: TabId, msg: KnownHostsMsg) -> Vec<Effect> {
        let Some(index) = self.tab_index(tab) else {
            return Vec::new();
        };
        let Some((host, port)) = self.changed_endpoint(index) else {
            return Vec::new();
        };
        let fix = &mut self.tabs[index].known_hosts;
        match (msg, &*fix) {
            (KnownHostsMsg::Start, KnownHostsFix::Idle | KnownHostsFix::Failed(_)) => {
                *fix = KnownHostsFix::Checking;
                vec![Effect::KnownHostEntries { tab, host, port }]
            }
            (KnownHostsMsg::Confirm, KnownHostsFix::Confirm(_)) => {
                *fix = KnownHostsFix::Removing;
                vec![Effect::RemoveKnownHost { tab, host, port }]
            }
            (KnownHostsMsg::Cancel, KnownHostsFix::Confirm(_)) => {
                *fix = KnownHostsFix::Idle;
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    pub(super) fn known_host_entries(
        &mut self,
        tab: TabId,
        result: Result<Vec<KnownHostEntry>, Error>,
    ) -> Vec<Effect> {
        if let Some(index) = self.tab_index(tab) {
            let fix = &mut self.tabs[index].known_hosts;
            if *fix == KnownHostsFix::Checking {
                *fix = match result {
                    Ok(entries) => KnownHostsFix::Confirm(entries),
                    Err(error) => KnownHostsFix::Failed(error),
                };
            }
        }
        Vec::new()
    }

    /// Removed: the tab connects again, and that connect asks about the new key.
    pub(super) fn known_host_removed(
        &mut self,
        tab: TabId,
        result: Result<usize, Error>,
    ) -> Vec<Effect> {
        let Some(index) = self.tab_index(tab) else {
            return Vec::new();
        };
        if self.tabs[index].known_hosts != KnownHostsFix::Removing {
            return Vec::new();
        }
        match result {
            Ok(_) => {
                self.tabs[index].known_hosts = KnownHostsFix::Idle;
                let site = self.tabs[index].site.clone();
                self.open_tabs(vec![site])
            }
            Err(error) => {
                self.tabs[index].known_hosts = KnownHostsFix::Failed(error);
                Vec::new()
            }
        }
    }
}

/// The answer a dialog's fields make, taking them out of the dialog.
fn take_secret(field: &mut Typed) -> SecretString {
    SecretString::from(std::mem::take(&mut field.0))
}

/// The answer of a credential dialog: its fields, taken out of it (nothing typed stays
/// in the state).
fn answer_of(dialog: &mut Dialog) -> PromptAnswer {
    match &dialog.request.kind {
        PromptKind::KeyboardInteractive { .. } => {
            PromptAnswer::Responses(dialog.fields.iter_mut().map(take_secret).collect())
        }
        _ => PromptAnswer::Secret {
            secret: dialog
                .fields
                .first_mut()
                .map(take_secret)
                .unwrap_or_default(),
            save: dialog.save,
        },
    }
}
