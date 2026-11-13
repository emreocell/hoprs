//! The questions before closing with remote edits (Review Focus 4): the last tab of a
//! session (inline in the tab), the window and "Close workspace" (in the center).
//!
//! Each asks the backend first what is unsent ([`Effect::CheckEdits`]); nothing unsent,
//! nothing is asked and the close goes on (it ends the edits and removes their files).
//! Otherwise the question lists the unsent files and where kept copies go. Its answers
//! run as one [`Effect::SettleEdits`], and only its result closes. While "Upload" runs,
//! "Stop and keep local copies" switches to keeping: the backend's `keep` stops the flush
//! between syncs, never waits for an upload under way (it flags that file's server copy
//! as possibly incomplete, shown in the kept notice), and no sync writes an edit it ended;
//! the upload's late result is ignored. The window's question takes priority: a workspace
//! question never replaces it, and a window close during a busy workspace question is
//! asked once that one ends. A site or workspace form open when a question takes the center is parked
//! and comes back when the question ends without closing the window.

use std::path::PathBuf;

use hoprs_core::{Error, WorkspaceId};

use super::{
    CenterAsk, CloseAnswer, EditState, EditSummary, EditsFor, Question, QuestionFile, Settle,
    Settled,
};
use crate::app::tabs::TabId;
use crate::app::{App, Effect, Form};
use crate::backend::SessionId;

impl App {
    /// Takes the center for the question, parking an open site or workspace form.
    fn show_center_question(&mut self) {
        self.park_form();
        self.form = Some(Form::QuitEdits);
    }

    #[cfg(test)]
    pub(crate) fn park_form_for_tests(&mut self) {
        self.park_form();
    }

    #[cfg(test)]
    pub(crate) fn unpark_form_for_tests(&mut self) {
        self.unpark_form();
    }

    /// An open form steps aside (onto the stack: a second question never loses the
    /// first one's form).
    fn park_form(&mut self) {
        if !matches!(self.form, None | Some(Form::QuitEdits))
            && let Some(form) = self.form.take()
        {
            self.parked_forms.push(form);
        }
    }

    /// A question ended without closing the window: the parked form comes back (unless
    /// the user opened another one meanwhile).
    fn unpark_form(&mut self) {
        // Unless the user opened another form meanwhile (the parked ones then wait).
        if matches!(self.form, None | Some(Form::QuitEdits)) {
            self.form = self.parked_forms.pop();
        }
    }

    /// The sessions with active edits among `sites`' connections.
    fn edit_sessions(&self, sites: &[hoprs_core::SiteId]) -> Vec<SessionId> {
        let mut sessions: Vec<SessionId> = self
            .edits
            .iter()
            .filter(|edit| sites.is_empty() || sites.contains(&edit.site))
            .map(|edit| edit.session)
            .collect();
        sessions.sort();
        sessions.dedup();
        sessions
    }

    /// The × of the last tab of a session with edits: the backend is asked what is
    /// unsent. `None`: the tab closes as usual.
    pub(in crate::app) fn ask_before_closing_tab(
        &mut self,
        index: usize,
    ) -> Option<Vec<super::Effect>> {
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
        // The question is in the tab: an open form steps aside until it ends.
        self.park_form();
        Some(vec![Effect::CheckEdits {
            purpose: EditsFor::Tab(id),
            sessions: vec![session],
        }])
    }

    /// The window asks to close: with active edits, the backend first says what is
    /// unsent. `None`: the usual close flow.
    pub(in crate::app) fn ask_before_quitting(
        &mut self,
        window: iced::window::Id,
    ) -> Option<Vec<Effect>> {
        // The window comes first: an idle workspace question gives way to it; a busy one
        // finishes, then the window's question follows.
        if let Some(ask) = &self.center_ask
            && matches!(ask.purpose, EditsFor::Workspace(_))
        {
            if ask.question.busy.is_some() {
                self.queued_quit = Some(window);
                return Some(Vec::new());
            }
            self.center_ask = None;
        }
        if let Some(ask) = &self.center_ask
            && ask.purpose == EditsFor::Quit(window)
        {
            if ask.question.files.is_some() && ask.question.busy.is_none() {
                // Asked already: show it again.
                self.show_center_question();
                return Some(Vec::new());
            }
            // Asked again while the backend checks or settles: the user insists. The
            // usual close flow runs; its shutdown first keeps every unsent edit as a
            // recovered copy (and a forced exit does too), never deleting one.
            tracing::warn!("close requested again while the remote edits were settled; closing");
            return None;
        }
        let sessions = self.edit_sessions(&[]);
        if sessions.is_empty() {
            return None;
        }
        self.center_ask = Some(CenterAsk {
            purpose: EditsFor::Quit(window),
            question: Question::checking(sessions.clone()),
        });
        Some(vec![Effect::CheckEdits {
            purpose: EditsFor::Quit(window),
            sessions,
        }])
    }

    /// "Close workspace" with edits on its sites: the backend first says what is unsent.
    /// `None`: it closes as usual.
    pub(in crate::app) fn ask_before_closing_workspace(
        &mut self,
        id: &WorkspaceId,
    ) -> Option<Vec<Effect>> {
        let sites: Vec<hoprs_core::SiteId> = self
            .workspace(id)?
            .sites
            .iter()
            .map(|entry| entry.site.clone())
            .collect();
        let sessions = self.edit_sessions(&sites);
        if sessions.is_empty() {
            return None;
        }
        // Another question in the center (the window's, or a busy one) is not replaced:
        // it shows again and this request is dropped.
        if let Some(ask) = &self.center_ask
            && (ask.question.busy.is_some() || !matches!(ask.purpose, EditsFor::Workspace(_)))
        {
            if ask.question.files.is_some() {
                self.show_center_question();
            }
            return Some(Vec::new());
        }
        let purpose = EditsFor::Workspace(id.clone());
        self.center_ask = Some(CenterAsk {
            purpose: purpose.clone(),
            question: Question::checking(sessions.clone()),
        });
        Some(vec![Effect::CheckEdits { purpose, sessions }])
    }

    /// The question of `purpose`, if it is still open.
    fn question_mut(&mut self, purpose: &EditsFor) -> Option<&mut Question> {
        match purpose {
            EditsFor::Tab(tab) => self.close_asks.get_mut(tab),
            EditsFor::Quit(_) | EditsFor::Workspace(_) => self
                .center_ask
                .as_mut()
                .filter(|ask| &ask.purpose == purpose)
                .map(|ask| &mut ask.question),
        }
    }

    pub(in crate::app) fn edits_checked(
        &mut self,
        purpose: EditsFor,
        unsent: Vec<EditSummary>,
        kept_in: PathBuf,
    ) -> Vec<Effect> {
        let Some(question) = self.question_mut(&purpose) else {
            return Vec::new();
        };
        let sessions = question.sessions.clone();
        let files = self.question_files(&sessions, &unsent);
        if files.is_empty() {
            // Everything is on the server: nothing to ask, the close goes on (and ends
            // the edits, removing their files).
            return self.close_for(purpose);
        }
        if let Some(question) = self.question_mut(&purpose) {
            question.files = Some(files);
            question.kept_in = Some(kept_in);
        }
        if !matches!(purpose, EditsFor::Tab(_)) {
            self.show_center_question();
        }
        Vec::new()
    }

    /// The unsent files among `sessions`' edits (and unsent edits the UI does not list:
    /// started by a click whose result is late).
    fn question_files(&self, sessions: &[SessionId], unsent: &[EditSummary]) -> Vec<QuestionFile> {
        unsent
            .iter()
            .filter(|summary| sessions.contains(&summary.session))
            .map(|summary| QuestionFile {
                remote: summary.remote.clone(),
                unsent: true,
                conflict: summary.state == EditState::Conflict,
            })
            .collect()
    }

    /// The question of `purpose` is settled (or moot): the tab, the workspace or the
    /// window closes now.
    fn close_for(&mut self, purpose: EditsFor) -> Vec<Effect> {
        match purpose {
            EditsFor::Tab(tab) => {
                self.close_asks.remove(&tab);
                self.unpark_form();
                match self.tab_index(tab) {
                    Some(index) => self.close_tab(index),
                    None => Vec::new(),
                }
            }
            EditsFor::Workspace(id) => {
                self.center_ask = None;
                self.unpark_form();
                let mut effects = self.close_workspace_now(&id);
                effects.extend(self.run_queued_quit());
                effects
            }
            EditsFor::Quit(window) => {
                self.center_ask = None;
                self.begin_close(window)
            }
        }
    }

    /// An answer to `purpose`'s question.
    fn answer(&mut self, purpose: EditsFor, answer: CloseAnswer) -> Vec<Effect> {
        let Some(question) = self.question_mut(&purpose) else {
            return Vec::new();
        };
        if question.files.is_none() {
            return Vec::new();
        }
        let how = match (question.busy, answer) {
            (None, CloseAnswer::Cancel) => {
                match purpose {
                    EditsFor::Tab(tab) => {
                        self.close_asks.remove(&tab);
                    }
                    EditsFor::Quit(_) | EditsFor::Workspace(_) => self.center_ask = None,
                }
                self.unpark_form();
                return self.run_queued_quit();
            }
            (None, CloseAnswer::Upload) => Settle::Upload,
            // "Keep", or "Stop and keep local copies" while the upload runs.
            (None | Some(CloseAnswer::Upload), CloseAnswer::Keep) => Settle::Keep,
            _ => return Vec::new(),
        };
        question.busy = Some(answer);
        question.error = None;
        vec![Effect::SettleEdits {
            purpose,
            sessions: question.sessions.clone(),
            how,
        }]
    }

    /// A window close that waited for a busy workspace question: asked now.
    fn run_queued_quit(&mut self) -> Vec<Effect> {
        match self.queued_quit.take() {
            Some(window) => self.close_requested(window),
            None => Vec::new(),
        }
    }

    pub(in crate::app) fn close_tab_answer(
        &mut self,
        tab: TabId,
        answer: CloseAnswer,
    ) -> Vec<Effect> {
        self.answer(EditsFor::Tab(tab), answer)
    }

    pub(in crate::app) fn center_answer(&mut self, answer: CloseAnswer) -> Vec<Effect> {
        match self.center_ask.as_ref().map(|ask| ask.purpose.clone()) {
            Some(purpose) => self.answer(purpose, answer),
            None => Vec::new(),
        }
    }

    /// An answer ran. Everything sent, or kept: the tab or workspace closes (with their
    /// sessions), or the window's close flow starts. Unsent files left (or a copy that
    /// failed): the question stays, saying so. A result for an answer that was replaced
    /// ("Stop and keep" during the upload) is ignored.
    pub(in crate::app) fn edits_settled(
        &mut self,
        purpose: EditsFor,
        how: Settle,
        result: Result<Settled, Error>,
    ) -> Vec<Effect> {
        let expected = self
            .question_mut(&purpose)
            .map(|question| match question.busy {
                Some(CloseAnswer::Keep) => Some(Settle::Keep),
                Some(CloseAnswer::Upload) => Some(Settle::Upload),
                _ => None,
            });
        if let Ok(settled) = &result {
            self.note_kept(&settled.kept, settled.kept_in.clone());
        }
        // Cancelled meanwhile, or superseded by another answer.
        if expected != Some(Some(how)) {
            return Vec::new();
        }
        let sessions = match self.question_mut(&purpose) {
            Some(question) => {
                question.busy = None;
                question.sessions.clone()
            }
            None => return Vec::new(),
        };
        match result {
            Ok(settled) if settled.unsent.is_empty() || how == Settle::Keep => {
                self.close_for(purpose)
            }
            Ok(settled) => {
                let files = self.question_files(&sessions, &settled.unsent);
                if let Some(question) = self.question_mut(&purpose) {
                    question.not_uploaded = files.len();
                    question.files = Some(files);
                }
                Vec::new()
            }
            Err(error) => {
                if let Some(question) = self.question_mut(&purpose) {
                    question.error = Some(error);
                }
                Vec::new()
            }
        }
    }
}
