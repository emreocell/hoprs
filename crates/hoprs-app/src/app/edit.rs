//! Remote editing in the UI (Plan 6, Task 8; Review Focus 4). The backend does the work
//! ([`crate::backend::edit`]); this is what the user sees and decides:
//!
//! - **Edit (F4)** on the remote pane opens the entry under the cursor (else the first
//!   selected one) in the editor of the settings ([`crate::settings::Settings::editor_command`]).
//!   Folders, links and files over [`EDIT_MAX_BYTES`] are refused, with the reason inline
//!   ([`edit_refusal`]).
//! - **Notices, never modals.** A thin strip in the site's files tabs has one line per
//!   active edit ([`EditStatus`]); a change on the server since the file was opened shows
//!   the inline choice "Overwrite / Keep remote / Save a copy" there. Each upload shows a
//!   [`Toast`] for [`TOAST_TIME`] at the bottom right.
//! - **Closing the last tab of a session** that has edits asks inline in that tab, listing
//!   the files: "Upload pending changes and close", "Close and keep local copies" or
//!   "Cancel" ([`CloseAsk`]).
//! - **Closing the window** with unsent edits asks the same in the center, *before* the
//!   Plan 5 close flow (save → engine → sessions): "Upload and quit" waits up to
//!   [`FLUSH_LIMIT`] for the uploads, "Quit and keep local copies" says where the copies go
//!   and deletes nothing ([`QuitAsk`]).
//!
//! Both questions first ask the backend what is unsent ([`Effect::CheckEdits`]): a save
//! the debounce has not seen yet counts. Their answers run as one [`Effect::SettleEdits`];
//! only its result closes the tab (and so its session) or starts the window's close flow.
//! An upload that could not finish keeps the question open with the files left, so
//! nothing is closed over unsent work. "Keep" saves each unsent edit as a recovered copy
//! (durable, see the backend) and leaves the local edit files where they are. A session
//! closed any other way (its site deleted, moved, restarted) ends its edits the same way
//! after the close ([`Effect::CloseSession`]).

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use hoprs_core::{EntryKind, Error, RemoteEntry, RemotePath, SiteId};

use super::files::Row;
use super::tabs::{SiteConn, Tab, TabId};
use super::{App, Effect, Form, Message};
use crate::backend::{
    EditChoice, EditEvent, EditId, EditState, EditSummary, KeptEdit, SessionId, Side,
};
use crate::ui::strings;

/// Largest file the Edit action opens (50 MB, base 1024 as the size column shows it).
pub const EDIT_MAX_BYTES: u64 = 50 * 1024 * 1024;
/// How long "Upload … and close/quit" waits for the uploads.
pub const FLUSH_LIMIT: Duration = Duration::from_secs(10);
/// How long a toast stays. It appears and goes without animation (iced reports no
/// "reduce motion" preference, so none is ever played).
pub const TOAST_TIME: Duration = Duration::from_secs(3);

/// Why Edit cannot open `entry`, if it cannot.
pub fn edit_refusal(entry: &RemoteEntry) -> Option<&'static str> {
    match entry.kind {
        EntryKind::Dir => Some(strings::EDIT_NO_FOLDERS),
        EntryKind::Symlink => Some(strings::EDIT_NO_LINKS),
        EntryKind::File if entry.size > EDIT_MAX_BYTES => Some(strings::EDIT_TOO_LARGE),
        EntryKind::File => None,
    }
}

/// What a line of the edits strip says about one edit.
#[derive(Clone, Debug, PartialEq)]
pub enum EditStatus {
    /// Open in the editor; `saved`: the last upload.
    Editing { saved: Option<SystemTime> },
    /// A save is held until the session is back (`truncated`: a cut write may have left
    /// the server's copy incomplete meanwhile).
    Waiting { truncated: bool },
    /// The last upload failed ("Retry").
    Failed { error: Error, truncated: bool },
    /// The server's copy changed: the inline choice. `busy`: a choice runs. `error`,
    /// `recovered`: a choice that failed half way, and where the user's version was saved.
    Conflict {
        busy: bool,
        error: Option<Error>,
        recovered: Option<PathBuf>,
    },
    /// "Keep remote" / "Save a copy" done: the local file is the server's version (the
    /// editor must reload it); the user's version is at `recovered`.
    Replaced { recovered: Option<PathBuf> },
}

/// One file being edited, as the UI knows it.
#[derive(Clone, Debug, PartialEq)]
pub struct RemoteEdit {
    pub id: EditId,
    pub session: SessionId,
    pub site: SiteId,
    pub remote: RemotePath,
    pub status: EditStatus,
}

impl RemoteEdit {
    /// The file's name, for the notices.
    pub fn name(&self) -> &str {
        self.remote.file_name().unwrap_or(self.remote.as_str())
    }
}

/// A file whose download and editor are on their way.
#[derive(Clone, Debug, PartialEq)]
pub struct Opening {
    pub session: SessionId,
    pub remote: RemotePath,
}

/// The small notice at the bottom right.
#[derive(Clone, Debug, PartialEq)]
pub struct Toast {
    /// Which expiry ends it (a newer toast is not ended by an older one's).
    pub id: u64,
    pub text: String,
}

/// What a close question is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditsFor {
    /// The last tab of a session.
    Tab(TabId),
    /// The window.
    Quit(iced::window::Id),
}

/// An answer to a close question.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseAnswer {
    /// "Upload pending changes and close" / "Upload and quit".
    Upload,
    /// "Close and keep local copies" / "Quit and keep local copies".
    Keep,
    Cancel,
}

/// How [`Effect::SettleEdits`] ends the edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Settle {
    /// Upload what is unsent (at most `FLUSH_LIMIT`), then end the edits that are all
    /// sent; report the rest.
    Upload,
    /// Save the unsent edits as recovered copies, end them, delete nothing.
    Keep,
}

/// What [`Effect::SettleEdits`] did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Settled {
    /// Still not on the server (an upload that did not finish, a conflict).
    pub unsent: Vec<EditSummary>,
    /// Saved as recovered copies.
    pub kept: Vec<KeptEdit>,
}

/// A close question's state, in the tab or in the center.
#[derive(Clone, Debug, PartialEq)]
pub struct Question {
    pub sessions: Vec<SessionId>,
    /// `None` while the backend is asked what is unsent.
    pub files: Option<Vec<QuestionFile>>,
    /// Where "keep" saves the copies.
    pub kept_in: Option<PathBuf>,
    /// The answer running, if any.
    pub busy: Option<CloseAnswer>,
    /// The upload left these files unsent (the question stays).
    pub not_uploaded: usize,
    pub error: Option<Error>,
}

/// One file a close question lists.
#[derive(Clone, Debug, PartialEq)]
pub struct QuestionFile {
    pub remote: RemotePath,
    /// Changes not on the server yet (else only open in the editor).
    pub unsent: bool,
    pub conflict: bool,
}

impl Question {
    fn checking(sessions: Vec<SessionId>) -> Self {
        Self {
            sessions,
            files: None,
            kept_in: None,
            busy: None,
            not_uploaded: 0,
            error: None,
        }
    }
}

/// The question of a tab whose close waits for its edits.
pub type CloseAsk = Question;

/// The window's question (shown in the center as [`Form::QuitEdits`]).
#[derive(Clone, Debug, PartialEq)]
pub struct QuitAsk {
    pub window: iced::window::Id,
    pub question: Question,
}

/// What the edits' widgets, feeds and effects send.
#[derive(Clone, Debug)]
pub enum EditMsg {
    /// The inline choice on a conflict.
    Resolve { id: EditId, choice: EditChoice },
    /// "Retry" on a failed upload.
    Retry(EditId),
    /// An answer in a tab's close question.
    CloseTab { tab: TabId, answer: CloseAnswer },
    /// An answer in the window's close question.
    Quit(CloseAnswer),
    /// The toast whose id this is may go.
    ToastExpired(u64),
    /// [`Effect::StartEdit`] finished.
    Started {
        tab: TabId,
        session: SessionId,
        remote: RemotePath,
        result: Result<EditId, Error>,
    },
    /// A resolve or retry finished.
    Done {
        id: EditId,
        result: Result<(), Error>,
    },
    /// An event of the backend's edits.
    Event(EditEvent),
    /// [`Effect::CheckEdits`] finished: the unsent edits, and where kept copies go.
    Checked {
        purpose: EditsFor,
        unsent: Vec<EditSummary>,
        kept_in: PathBuf,
    },
    /// [`Effect::SettleEdits`] finished.
    Settled {
        purpose: EditsFor,
        how: Settle,
        result: Result<Settled, Error>,
    },
    /// A session closed some other way kept these unsent edits as recovered copies.
    Kept(Vec<KeptEdit>),
}

impl EditMsg {
    /// Whether the user sent it (ignored while a dialog is open or the window closes).
    pub fn is_user(&self) -> bool {
        matches!(
            self,
            Self::Resolve { .. } | Self::Retry(_) | Self::CloseTab { .. } | Self::Quit(_)
        )
    }
}

/// The edit of `id` in `edits`.
fn find(edits: &mut [RemoteEdit], id: EditId) -> Option<&mut RemoteEdit> {
    edits.iter_mut().find(|edit| edit.id == id)
}

impl App {
    /// The active edits of `session`, in the order they started.
    pub fn session_edits(&self, session: SessionId) -> Vec<&RemoteEdit> {
        self.edits
            .iter()
            .filter(|edit| edit.session == session)
            .collect()
    }

    /// See [`Tab::edit_target`].
    pub fn edit_target(
        &self,
        index: usize,
        side: Side,
    ) -> Option<Result<RemotePath, &'static str>> {
        self.tabs[index].edit_target(self.tab_conn(index), side)
    }
}

impl Tab {
    /// The entry Edit (F4) acts on in pane `side`, given the site's connection `conn`:
    /// the cursor's entry if selected, else the first selected one; or why it cannot be
    /// edited. `None`: nothing to act on (no selection, the local side, not ready).
    pub fn edit_target(
        &self,
        conn: &SiteConn,
        side: Side,
    ) -> Option<Result<RemotePath, &'static str>> {
        if side != Side::Remote || !self.can_act(conn, side) {
            return None;
        }
        let pane = self.pane(side)?;
        let cursor = pane
            .selection
            .cursor
            .filter(|&c| pane.selection.contains(c))
            .and_then(|c| match pane.row(c) {
                Some(Row::Entry(entry)) => Some(entry),
                _ => None,
            });
        let entry = cursor.or_else(|| pane.selected_entries().first().copied())?;
        Some(match edit_refusal(entry) {
            Some(reason) => Err(reason),
            None => Ok(pane.path.join(&entry.name)),
        })
    }
}

impl App {
    /// F4 / "Edit": downloads the entry and opens the editor; a refused entry says why
    /// inline (the pane's note).
    pub(super) fn start_edit(&mut self, index: usize, side: Side) -> Vec<Effect> {
        let Some(target) = self.edit_target(index, side) else {
            return Vec::new();
        };
        let remote = match target {
            Ok(remote) => remote,
            Err(reason) => {
                if let Some(pane) = self.tabs[index].pane_mut(side) {
                    pane.note = Some(reason.to_owned());
                }
                return Vec::new();
            }
        };
        let Some(session) = self.tab_session(index) else {
            return Vec::new();
        };
        // An earlier refusal no longer applies.
        if let Some(pane) = self.tabs[index].pane_mut(side)
            && pane.note.as_deref().is_some_and(|note| {
                [
                    strings::EDIT_NO_FOLDERS,
                    strings::EDIT_NO_LINKS,
                    strings::EDIT_TOO_LARGE,
                ]
                .contains(&note)
            })
        {
            pane.note = None;
        }
        if !self
            .edit_opening
            .iter()
            .any(|o| o.session == session && o.remote == remote)
        {
            self.edit_opening.push(Opening {
                session,
                remote: remote.clone(),
            });
        }
        vec![Effect::StartEdit {
            tab: self.tabs[index].id,
            session,
            remote,
            editor: self.settings.editor_command.clone(),
        }]
    }

    pub(super) fn edit_msg(&mut self, msg: EditMsg) -> Vec<Effect> {
        match msg {
            EditMsg::Resolve { id, choice } => {
                let Some(edit) = find(&mut self.edits, id) else {
                    return Vec::new();
                };
                let EditStatus::Conflict { busy, error, .. } = &mut edit.status else {
                    return Vec::new();
                };
                if *busy {
                    return Vec::new();
                }
                *busy = true;
                *error = None;
                vec![Effect::ResolveEdit { id, choice }]
            }
            EditMsg::Retry(id) => match find(&mut self.edits, id) {
                Some(edit) if matches!(edit.status, EditStatus::Failed { .. }) => {
                    vec![Effect::RetryEdit(id)]
                }
                _ => Vec::new(),
            },
            EditMsg::CloseTab { tab, answer } => self.close_tab_answer(tab, answer),
            EditMsg::Quit(answer) => self.quit_answer(answer),
            EditMsg::ToastExpired(id) => {
                if self.toast.as_ref().is_some_and(|t| t.id == id) {
                    self.toast = None;
                }
                Vec::new()
            }
            EditMsg::Started {
                tab,
                session,
                remote,
                result,
            } => self.edit_started(tab, session, remote, result),
            EditMsg::Done { id, result } => {
                self.edit_done(id, result);
                Vec::new()
            }
            EditMsg::Event(event) => self.edit_event(event),
            EditMsg::Checked {
                purpose,
                unsent,
                kept_in,
            } => self.edits_checked(purpose, unsent, kept_in),
            EditMsg::Settled {
                purpose,
                how,
                result,
            } => self.edits_settled(purpose, how, result),
            EditMsg::Kept(kept) => self.kept_toast(&kept),
        }
    }

    /// The download and editor of [`Effect::StartEdit`] finished. A failure shows inline
    /// in the remote pane that asked (if it is still there).
    pub(super) fn edit_started(
        &mut self,
        tab: TabId,
        session: SessionId,
        remote: RemotePath,
        result: Result<EditId, Error>,
    ) -> Vec<Effect> {
        self.edit_opening
            .retain(|o| !(o.session == session && o.remote == remote));
        match result {
            Ok(id) => {
                // The same file opened again: same id, same line.
                if self.edits.iter().any(|edit| edit.id == id) {
                    return Vec::new();
                }
                let Some(site) = self.site_of_session(session) else {
                    // The session is gone meanwhile: its close ended the edits.
                    return Vec::new();
                };
                self.edits.push(RemoteEdit {
                    id,
                    session,
                    site,
                    remote,
                    status: EditStatus::Editing { saved: None },
                });
                self.edits.sort_by_key(|edit| edit.id);
            }
            Err(error) => {
                if let Some(pane) = self
                    .tab_index(tab)
                    .and_then(|i| self.tabs[i].pane_mut(Side::Remote))
                {
                    pane.action_error = Some(error);
                }
            }
        }
        Vec::new()
    }

    /// A resolve or retry ended. A failure shows on the edit's line (a conflict keeps its
    /// choice); a success is shown by the events that follow it.
    fn edit_done(&mut self, id: EditId, result: Result<(), Error>) {
        let Some(edit) = find(&mut self.edits, id) else {
            return;
        };
        match (&mut edit.status, result) {
            (EditStatus::Conflict { busy, error, .. }, Err(failure)) => {
                *busy = false;
                *error = Some(failure);
            }
            // Settled before its event arrived: the line leaves the choice.
            (EditStatus::Conflict { .. }, Ok(())) => {
                edit.status = EditStatus::Editing { saved: None };
            }
            (EditStatus::Failed { error, .. }, Err(failure)) => *error = failure,
            (_, Err(failure)) => {
                edit.status = EditStatus::Failed {
                    error: failure,
                    truncated: false,
                };
            }
            (_, Ok(())) => {}
        }
    }

    fn edit_event(&mut self, event: EditEvent) -> Vec<Effect> {
        match event {
            EditEvent::Uploaded { id, at } => {
                let Some(edit) = find(&mut self.edits, id) else {
                    return Vec::new();
                };
                edit.status = EditStatus::Editing { saved: Some(at) };
                let text = strings::uploaded(edit.name());
                return self.show_toast(text);
            }
            EditEvent::RemoteChanged { id } => {
                if let Some(edit) = find(&mut self.edits, id) {
                    edit.status = EditStatus::Conflict {
                        busy: false,
                        error: None,
                        recovered: None,
                    };
                }
            }
            EditEvent::Pending {
                id,
                remote_may_be_truncated,
            } => {
                if let Some(edit) = find(&mut self.edits, id) {
                    edit.status = EditStatus::Waiting {
                        truncated: remote_may_be_truncated,
                    };
                }
            }
            EditEvent::Failed {
                id,
                error,
                remote_may_be_truncated,
                recovered,
            } => {
                if let Some(edit) = find(&mut self.edits, id) {
                    edit.status = match &edit.status {
                        // A choice failed half way: the choice stays, with where the
                        // user's version went.
                        EditStatus::Conflict { .. } => EditStatus::Conflict {
                            busy: false,
                            error: Some(error),
                            recovered,
                        },
                        _ => EditStatus::Failed {
                            error,
                            truncated: remote_may_be_truncated,
                        },
                    };
                }
            }
            EditEvent::ReplacedByRemote { id, recovered, .. } => {
                if let Some(edit) = find(&mut self.edits, id) {
                    edit.status = EditStatus::Replaced { recovered };
                }
            }
            EditEvent::Closed { id } => self.edits.retain(|edit| edit.id != id),
        }
        Vec::new()
    }

    /// Shows `text` as the toast, ending it after [`TOAST_TIME`].
    fn show_toast(&mut self, text: String) -> Vec<Effect> {
        let id = self.next_id();
        self.toast = Some(Toast { id, text });
        vec![Effect::Later {
            after: TOAST_TIME,
            message: Box::new(Message::Edit(EditMsg::ToastExpired(id))),
        }]
    }

    /// Recovered copies were made: the toast says where.
    fn kept_toast(&mut self, kept: &[KeptEdit]) -> Vec<Effect> {
        let Some(first) = kept.first() else {
            return Vec::new();
        };
        let dir = first
            .recovered
            .parent()
            .map_or_else(|| first.recovered.clone(), PathBuf::from);
        self.show_toast(strings::kept_copies(kept.len(), &dir.display().to_string()))
    }

    /// The × of the last tab of a session with edits: the question, once the backend
    /// said what is unsent. `None`: the tab closes as usual.
    pub(super) fn ask_before_closing_tab(&mut self, index: usize) -> Option<Vec<Effect>> {
        let tab = &self.tabs[index];
        let id = tab.id;
        if self.close_asks.contains_key(&id) {
            // Asked already: the question is on screen.
            self.active_tab = Some(index);
            return Some(Vec::new());
        }
        if self.tabs_of(&tab.site).len() > 1 {
            return None;
        }
        let session = self.conn(&tab.site).session()?;
        if self.session_edits(session).is_empty() {
            return None;
        }
        self.close_asks
            .insert(id, Question::checking(vec![session]));
        self.active_tab = Some(index);
        self.form = None;
        Some(vec![Effect::CheckEdits {
            purpose: EditsFor::Tab(id),
            sessions: vec![session],
        }])
    }

    /// The window asks to close: with active edits, the backend first says what is unsent
    /// (the question, or the usual close flow if nothing is). `None`: no edits.
    pub(super) fn ask_before_quitting(&mut self, window: iced::window::Id) -> Option<Vec<Effect>> {
        if let Some(quit) = &self.quit {
            if quit.question.files.is_some() && quit.question.busy.is_none() {
                // Asked already: show it again.
                self.form = Some(Form::QuitEdits);
                return Some(Vec::new());
            }
            // Asked again while the backend checks or settles: the user insists. The
            // usual close flow runs; the shutdown keeps what is unsent as recovered
            // copies (it never deletes them).
            tracing::warn!("close requested again while the remote edits were settled; closing");
            return None;
        }
        let mut sessions: Vec<SessionId> = self.edits.iter().map(|edit| edit.session).collect();
        sessions.sort();
        sessions.dedup();
        if sessions.is_empty() {
            return None;
        }
        self.quit = Some(QuitAsk {
            window,
            question: Question::checking(sessions.clone()),
        });
        Some(vec![Effect::CheckEdits {
            purpose: EditsFor::Quit(window),
            sessions,
        }])
    }

    fn edits_checked(
        &mut self,
        purpose: EditsFor,
        unsent: Vec<EditSummary>,
        kept_in: PathBuf,
    ) -> Vec<Effect> {
        match purpose {
            EditsFor::Tab(tab) => {
                let Some(index) = self.tab_index(tab) else {
                    self.close_asks.remove(&tab);
                    return Vec::new();
                };
                let Some(question) = self.close_asks.get(&tab) else {
                    return Vec::new();
                };
                let files = self.question_files(&question.sessions, &unsent);
                if files.is_empty() {
                    // The edits ended meanwhile: nothing to ask.
                    self.close_asks.remove(&tab);
                    return self.close_tab(index);
                }
                if let Some(question) = self.close_asks.get_mut(&tab) {
                    question.files = Some(files);
                    question.kept_in = Some(kept_in);
                }
                Vec::new()
            }
            EditsFor::Quit(window) => {
                let Some(quit) = self.quit.as_ref().filter(|q| q.window == window) else {
                    return Vec::new();
                };
                if unsent.is_empty() {
                    // Everything is on the server: the usual close flow (which ends the
                    // edits and removes their files).
                    self.quit = None;
                    return self.begin_close(window);
                }
                let files = self.question_files(&quit.question.sessions, &unsent);
                if let Some(quit) = &mut self.quit {
                    quit.question.files = Some(files);
                    quit.question.kept_in = Some(kept_in);
                }
                self.form = Some(Form::QuitEdits);
                Vec::new()
            }
        }
    }

    /// The files a question lists: the active edits of `sessions` (the window lists only
    /// the unsent ones), each marked unsent or not.
    fn question_files(&self, sessions: &[SessionId], unsent: &[EditSummary]) -> Vec<QuestionFile> {
        let mut files: Vec<QuestionFile> = self
            .edits
            .iter()
            .filter(|edit| sessions.contains(&edit.session))
            .map(|edit| {
                let summary = unsent.iter().find(|s| s.id == edit.id);
                QuestionFile {
                    remote: edit.remote.clone(),
                    unsent: summary.is_some(),
                    conflict: summary.is_some_and(|s| s.state == EditState::Conflict),
                }
            })
            .collect();
        // Unsent edits the UI does not list (started by a click whose result is late).
        for summary in unsent {
            if !self.edits.iter().any(|edit| edit.id == summary.id) {
                files.push(QuestionFile {
                    remote: summary.remote.clone(),
                    unsent: true,
                    conflict: summary.state == EditState::Conflict,
                });
            }
        }
        files
    }

    fn close_tab_answer(&mut self, tab: TabId, answer: CloseAnswer) -> Vec<Effect> {
        let Some(question) = self.close_asks.get_mut(&tab) else {
            return Vec::new();
        };
        if question.busy.is_some() || question.files.is_none() {
            return Vec::new();
        }
        let how = match answer {
            CloseAnswer::Cancel => {
                self.close_asks.remove(&tab);
                return Vec::new();
            }
            CloseAnswer::Upload => Settle::Upload,
            CloseAnswer::Keep => Settle::Keep,
        };
        question.busy = Some(answer);
        question.error = None;
        vec![Effect::SettleEdits {
            purpose: EditsFor::Tab(tab),
            sessions: question.sessions.clone(),
            how,
        }]
    }

    fn quit_answer(&mut self, answer: CloseAnswer) -> Vec<Effect> {
        let Some(quit) = &mut self.quit else {
            return Vec::new();
        };
        if quit.question.busy.is_some() || quit.question.files.is_none() {
            return Vec::new();
        }
        let how = match answer {
            CloseAnswer::Cancel => {
                self.quit = None;
                if matches!(self.form, Some(Form::QuitEdits)) {
                    self.form = None;
                }
                return Vec::new();
            }
            CloseAnswer::Upload => Settle::Upload,
            CloseAnswer::Keep => Settle::Keep,
        };
        quit.question.busy = Some(answer);
        quit.question.error = None;
        vec![Effect::SettleEdits {
            purpose: EditsFor::Quit(quit.window),
            sessions: quit.question.sessions.clone(),
            how,
        }]
    }

    /// An answer ran. Everything sent, or kept: the tab closes (its session with it), or
    /// the window's close flow starts. Unsent files left (or a copy that failed): the
    /// question stays, saying so.
    fn edits_settled(
        &mut self,
        purpose: EditsFor,
        how: Settle,
        result: Result<Settled, Error>,
    ) -> Vec<Effect> {
        let mut effects = match &result {
            Ok(settled) => self.kept_toast(&settled.kept),
            Err(_) => Vec::new(),
        };
        let unsent = match result {
            Ok(settled) => Ok(settled.unsent),
            Err(error) => Err(error),
        };
        let question = match purpose {
            EditsFor::Tab(tab) => self.close_asks.get_mut(&tab),
            EditsFor::Quit(window) => self
                .quit
                .as_mut()
                .filter(|q| q.window == window)
                .map(|q| &mut q.question),
        };
        let Some(question) = question else {
            // Cancelled meanwhile (the window's question left the center).
            return effects;
        };
        question.busy = None;
        match unsent {
            Ok(unsent) if unsent.is_empty() || how == Settle::Keep => {}
            Ok(unsent) => {
                question.not_uploaded = unsent.len();
                let sessions = question.sessions.clone();
                let files = self.question_files(&sessions, &unsent);
                let question = match purpose {
                    EditsFor::Tab(tab) => self.close_asks.get_mut(&tab),
                    EditsFor::Quit(_) => self.quit.as_mut().map(|q| &mut q.question),
                };
                if let Some(question) = question {
                    question.files = Some(files.into_iter().filter(|f| f.unsent).collect());
                }
                return effects;
            }
            Err(error) => {
                question.error = Some(error);
                return effects;
            }
        }
        match purpose {
            EditsFor::Tab(tab) => {
                self.close_asks.remove(&tab);
                if let Some(index) = self.tab_index(tab) {
                    effects.extend(self.close_tab(index));
                }
            }
            EditsFor::Quit(window) => {
                self.quit = None;
                effects.extend(self.begin_close(window));
            }
        }
        effects
    }
}
