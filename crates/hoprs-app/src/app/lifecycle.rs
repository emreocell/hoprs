//! Sessions over time and the app's own lifecycle: the connection state each session's
//! feed reports (status bar, reconnect banner), the tabs a workspace saves when it
//! closes, and the orderly shutdown when the window closes (Review Focus 5).
//!
//! Closing the window: the app asks for one [`Effect::Shutdown`], which saves the open
//! workspace's tabs, shuts the transfer engine down (its running items keep their stored
//! state and reopen paused on the next start), then closes the sessions, all within
//! 5 s. Only its [`Message::ShutdownDone`] closes the window. A failed save is logged and
//! the shutdown goes on; a backend that hangs is abandoned when the 5 s run out.

use std::path::PathBuf;

use hoprs_core::{ConnectionState, Error, ReconnectPolicy, SavedTab, TabKind, WorkspaceId};

use super::{App, Effect, TabState};
use crate::app::files::{Listing, TabId};
use crate::backend::{AuthMethod, SessionId, Side, WorkspaceTabs};

/// What the status bar says about a tab's connection.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ConnStatus<'a> {
    Connecting,
    Connected,
    /// Attempt `attempt` of `of`.
    Reconnecting {
        attempt: u32,
        of: u32,
    },
    /// A prompt holds the connection until the user answers.
    Waiting,
    Disconnected(&'a Error),
}

/// The files tab among a workspace site's saved tabs.
pub fn files_tab(tabs: &[SavedTab]) -> Option<&SavedTab> {
    tabs.iter().find(|t| t.kind == TabKind::Files)
}

impl super::Tab {
    /// The connection as the status bar shows it: the connect's outcome, then what the
    /// session's feed reports.
    pub fn conn_status(&self) -> ConnStatus<'_> {
        match &self.state {
            TabState::Connecting => ConnStatus::Connecting,
            TabState::Failed(failure) => ConnStatus::Disconnected(&failure.error),
            TabState::Connected(_) => match &self.conn {
                None | Some(ConnectionState::Connected) => ConnStatus::Connected,
                Some(ConnectionState::Connecting) => ConnStatus::Connecting,
                Some(ConnectionState::Reconnecting { attempt }) => ConnStatus::Reconnecting {
                    attempt: *attempt,
                    // Sessions use the default policy (see `backend::mapping`).
                    of: ReconnectPolicy::default().max_attempts,
                },
                Some(ConnectionState::WaitingForUser) => ConnStatus::Waiting,
                Some(ConnectionState::Disconnected(error)) => ConnStatus::Disconnected(error),
            },
        }
    }

    /// Why a connected session is lost for good (it gave up reconnecting, or was refused):
    /// the tab shows it in a banner with "Reconnect".
    pub fn lost_connection(&self) -> Option<&Error> {
        match (&self.state, &self.conn) {
            (TabState::Connected(_), Some(ConnectionState::Disconnected(error))) => Some(error),
            _ => None,
        }
    }

    /// The tab as a workspace saves it: a files tab at its two folders.
    fn saved(&self) -> SavedTab {
        SavedTab {
            kind: TabKind::Files,
            remote_dir: Some(self.remote.path.as_str().to_owned()),
            local_dir: Some(PathBuf::from(self.local.path.as_str())),
        }
    }
}

impl App {
    /// The sessions whose state the app follows (one feed each): those of the connected
    /// tabs. A tab that closes or fails drops out, which ends its feed.
    pub fn watched_sessions(&self) -> Vec<SessionId> {
        self.tabs.iter().filter_map(|tab| tab.session()).collect()
    }

    /// How many of the workspace's sites have a connected tab, of all its sites.
    pub fn workspace_connected(&self, id: &WorkspaceId) -> Option<(usize, usize)> {
        let workspace = self.workspace(id)?;
        let connected = workspace
            .sites
            .iter()
            .filter(|entry| {
                self.tab_of(&entry.site)
                    .is_some_and(|i| self.tabs[i].conn_status() == ConnStatus::Connected)
            })
            .count();
        Some((connected, workspace.sites.len()))
    }

    /// A session's feed: its state and, once logged in, how. Only a connected tab takes
    /// it (a failed one follows its connect's outcome). Back to `Connected` after a loss,
    /// the site's transfers run again and a listing that failed meanwhile is retried.
    pub(super) fn session_state(
        &mut self,
        session: SessionId,
        state: ConnectionState,
        auth: Option<AuthMethod>,
    ) -> Vec<Effect> {
        let Some(index) = self.tabs.iter().position(|t| t.session() == Some(session)) else {
            return Vec::new();
        };
        let tab = &mut self.tabs[index];
        if auth.is_some() {
            tab.auth = auth;
        }
        let back = state == ConnectionState::Connected
            && tab
                .conn
                .as_ref()
                .is_some_and(|c| *c != ConnectionState::Connected);
        tab.conn = Some(state);
        if !back {
            return Vec::new();
        }
        tab.reconnecting = false;
        let retry = matches!(tab.remote.listing, Listing::Failed(_));
        let site = tab.site.clone();
        self.queue.site_restored(&site);
        if retry {
            self.request_listing(index, Side::Remote)
        } else {
            Vec::new()
        }
    }

    /// "Reconnect" in the banner: the backend's reconnect (which also settles the secrets
    /// typed with "Save in the system keyring"), one at a time.
    pub(super) fn reconnect_session(&mut self, tab: TabId) -> Vec<Effect> {
        let Some(index) = self.tab_index(tab) else {
            return Vec::new();
        };
        let tab_ref = &mut self.tabs[index];
        let Some(session) = tab_ref.session() else {
            return Vec::new();
        };
        if tab_ref.reconnecting || tab_ref.lost_connection().is_none() {
            return Vec::new();
        }
        tab_ref.reconnecting = true;
        vec![Effect::Reconnect { tab, session }]
    }

    /// The reconnect finished: connected, the remote folder is listed again; refused, the
    /// banner shows the new reason. A result for a tab (or session) that is gone is
    /// dropped.
    pub(super) fn reconnected(
        &mut self,
        tab: TabId,
        session: SessionId,
        result: Result<(), Error>,
    ) -> Vec<Effect> {
        let Some(index) = self.tab_index(tab) else {
            return Vec::new();
        };
        if self.tabs[index].session() != Some(session) {
            return Vec::new();
        }
        let connected = result.is_ok();
        let mut effects = self.settle_saving(session, connected);
        let tab = &mut self.tabs[index];
        tab.reconnecting = false;
        match result {
            Ok(()) => {
                tab.conn = Some(ConnectionState::Connected);
                let site = tab.site.clone();
                self.queue.site_restored(&site);
                effects.extend(self.request_listing(index, Side::Remote));
            }
            Err(error) => tab.conn = Some(ConnectionState::Disconnected(error)),
        }
        effects
    }

    /// The tabs of the workspace's sites as it saves them: each site with a tab gets its
    /// files tab replaced by the current folders (other saved tabs of the site are kept).
    /// Sites without a tab are not named, so they keep what they had. `None` when there is
    /// nothing to save.
    fn workspace_tabs(&self, id: &WorkspaceId) -> Option<WorkspaceTabs> {
        let workspace = self.workspace(id)?;
        let tabs: WorkspaceTabs = workspace
            .sites
            .iter()
            .filter_map(|entry| {
                let tab = &self.tabs[self.tab_of(&entry.site)?];
                let mut saved = entry.tabs.clone();
                match saved.iter_mut().find(|t| t.kind == TabKind::Files) {
                    Some(slot) => *slot = tab.saved(),
                    None => saved.push(tab.saved()),
                }
                Some((entry.site.clone(), saved))
            })
            .collect();
        (!tabs.is_empty()).then_some(tabs)
    }

    pub(super) fn save_tabs_effect(&self, id: &WorkspaceId) -> Option<Effect> {
        let tabs = self.workspace_tabs(id)?;
        Some(Effect::SaveTabs {
            workspace: id.clone(),
            tabs,
        })
    }

    /// "Close workspace": saves its tabs, then closes them and their sessions. Tabs of
    /// other sites stay.
    pub(super) fn close_workspace(&mut self, id: &WorkspaceId) -> Vec<Effect> {
        if self.open_workspace.as_ref() != Some(id) {
            return Vec::new();
        }
        let mut effects: Vec<Effect> = self.save_tabs_effect(id).into_iter().collect();
        self.open_workspace = None;
        let sites: Vec<_> = self
            .workspace(id)
            .map(|w| w.sites.iter().map(|e| e.site.clone()).collect())
            .unwrap_or_default();
        for site in sites {
            if let Some(index) = self.tab_of(&site) {
                effects.extend(self.close_tab(index));
            }
        }
        effects
    }

    /// Saved: the profiles are read again, so the next "Open workspace" restores these
    /// folders. A failure is logged only (the tabs are not worth an error in the UI).
    pub(super) fn tabs_saved(&mut self, result: Result<(), Error>) -> Vec<Effect> {
        match result {
            Ok(()) => vec![Effect::LoadProfiles],
            Err(error) => {
                tracing::warn!(%error, "cannot save the workspace's tabs");
                Vec::new()
            }
        }
    }

    /// The window asks to close: one orderly shutdown (see the module docs). A second
    /// request meanwhile changes nothing: the shutdown ends within 5 s anyway.
    pub(super) fn close_requested(&mut self, window: iced::window::Id) -> Vec<Effect> {
        if self.closing {
            return Vec::new();
        }
        self.closing = true;
        let save = self
            .open_workspace
            .clone()
            .and_then(|id| self.workspace_tabs(&id).map(|tabs| (id, tabs)));
        vec![Effect::Shutdown { save, window }]
    }
}
