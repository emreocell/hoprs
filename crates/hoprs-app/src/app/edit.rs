//! Remote editing in the UI (Plan 6, Task 8; Review Focus 4). The backend does the work
//! ([`crate::backend::edit`]); this is what the user sees and decides:
//!
//! - **Edit (F4)** on the remote pane opens the selected file in the editor of the
//!   settings ([`crate::settings::Settings::editor_command`]). Folders, links, files over
//!   [`EDIT_MAX_BYTES`] and a selection of several entries are refused, with the reason
//!   inline ([`edit_refusal`]).
//! - **Notices, never modals.** A thin strip in the session's files and terminal tabs has
//!   one line per active edit ([`EditStatus`], the file name in mono); a change on the
//!   server since the file was opened shows the inline choice "Overwrite / Keep remote /
//!   Save a copy" there. Each upload shows a [`Toast`] for [`TOAST_TIME`] at the bottom
//!   right. Copies a close kept are said by a [`KeptNotice`] above the center, until
//!   dismissed.
//! - **Questions before closing** (the last tab of a session, "Close workspace", the
//!   window), in [`close`]: asked only when something is unsent, before anything closes.
//!
//! A session closed any other way (its site deleted or moved, "Close and reconnect")
//! keeps its unsent edits as recovered copies after the close ([`Effect::CloseSession`]),
//! deleting nothing; the window's shutdown (and a forced exit) keeps them first.

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use hoprs_core::{EntryKind, Error, RemoteEntry, RemotePath, SiteId, WorkspaceId};

use super::files::Row;
use super::tabs::{SiteConn, Tab, TabId};
use super::{App, Effect, Message};
use crate::backend::{
    EditChoice, EditEvent, EditId, EditState, EditSummary, KeptEdit, SessionId, Side,
};
use crate::ui::strings;

mod close;

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

/// The small notice at the bottom right: `text` then the file `name` (in mono).
#[derive(Clone, Debug, PartialEq)]
pub struct Toast {
    /// Which expiry ends it (a newer toast is not ended by an older one's).
    pub id: u64,
    pub text: &'static str,
    pub name: String,
}

/// Unsent edits a close kept as recovered copies, said until dismissed (above the
/// center, whatever it shows).
#[derive(Clone, Debug, PartialEq)]
pub struct KeptNotice {
    /// How many copies, since the notice was last dismissed.
    pub count: usize,
    /// The recovered copies' root.
    pub dir: PathBuf,
    /// Remote files whose server copy may be incomplete: an upload was under way when
    /// they were kept (never left unsaid).
    pub incomplete: Vec<RemotePath>,
}

/// What a close question is for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditsFor {
    /// The last tab of a session.
    Tab(TabId),
    /// The window.
    Quit(iced::window::Id),
    /// "Close workspace".
    Workspace(WorkspaceId),
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
    /// The recovered copies' root.
    pub kept_in: PathBuf,
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

/// The question shown in the center ([`Form::QuitEdits`]): the window's, or "Close
/// workspace"'s.
#[derive(Clone, Debug, PartialEq)]
pub struct CenterAsk {
    /// [`EditsFor::Quit`] or [`EditsFor::Workspace`].
    pub purpose: EditsFor,
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
    /// An answer in the question in the center (window or workspace).
    Center(CloseAnswer),
    /// "Dismiss" on the notice of kept copies.
    DismissKept,
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
    /// A session closed some other way kept these unsent edits as recovered copies,
    /// under `kept_in`.
    Kept {
        kept: Vec<KeptEdit>,
        kept_in: PathBuf,
    },
}

impl EditMsg {
    /// Whether the user sent it (ignored while a dialog is open or the window closes).
    pub fn is_user(&self) -> bool {
        matches!(
            self,
            Self::Resolve { .. }
                | Self::Retry(_)
                | Self::CloseTab { .. }
                | Self::Center(_)
                | Self::DismissKept
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
        // Several selected: one file at a time (the cursor's would be a guess).
        if pane.selected_entries().len() > 1 {
            return Some(Err(strings::EDIT_ONE_FILE));
        }
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
                    strings::EDIT_ONE_FILE,
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
            EditMsg::Center(answer) => self.center_answer(answer),
            EditMsg::DismissKept => {
                self.kept_notice = None;
                Vec::new()
            }
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
            EditMsg::Kept { kept, kept_in } => {
                self.note_kept(&kept, kept_in);
                Vec::new()
            }
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
                let name = edit.name().to_owned();
                return self.show_toast(strings::UPLOADED, name);
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
                let Some(edit) = find(&mut self.edits, id) else {
                    return Vec::new();
                };
                edit.status = EditStatus::Waiting {
                    truncated: remote_may_be_truncated,
                };
                // A save in the editor is the user acting on the site: a session lost to
                // the network reconnects (never one refused for good; once).
                let (site, session) = (edit.site.clone(), edit.session);
                if self
                    .conns
                    .get(&site)
                    .is_none_or(|conn| conn.session() != Some(session))
                {
                    return Vec::new();
                }
                return self.reconnect_on_demand(&site).into_iter().collect();
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

    /// Shows `text` + `name` as the toast, ending it after [`TOAST_TIME`].
    fn show_toast(&mut self, text: &'static str, name: String) -> Vec<Effect> {
        let id = self.next_id();
        self.toast = Some(Toast { id, text, name });
        vec![Effect::Later {
            after: TOAST_TIME,
            message: Box::new(Message::Edit(EditMsg::ToastExpired(id))),
        }]
    }

    /// Recovered copies were made by a close: the notice above the center says how many
    /// and where, until dismissed (an upload's toast never replaces it); logged too.
    pub(super) fn note_kept(&mut self, kept: &[KeptEdit], dir: PathBuf) {
        if kept.is_empty() {
            return;
        }
        tracing::info!(count = kept.len(), dir = %dir.display(), "unsent remote edits kept as recovered copies");
        let mut notice = self.kept_notice.take().unwrap_or(KeptNotice {
            count: 0,
            dir: dir.clone(),
            incomplete: Vec::new(),
        });
        notice.count += kept.len();
        notice.dir = dir;
        for edit in kept.iter().filter(|edit| edit.remote_may_be_truncated) {
            tracing::warn!(remote = %edit.remote, "kept during an upload; the server's copy may be incomplete");
            if !notice.incomplete.contains(&edit.remote) {
                notice.incomplete.push(edit.remote.clone());
            }
        }
        self.kept_notice = Some(notice);
    }

    /// The delete confirmation's note when the site has files open for editing.
    pub fn site_delete_note(&self, site: &SiteId) -> Option<String> {
        let n = self.edits.iter().filter(|edit| &edit.site == site).count();
        (n > 0).then(|| strings::delete_site_edits(n))
    }
}
