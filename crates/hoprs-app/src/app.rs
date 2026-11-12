//! The application state and its pure update: no I/O here, only state
//! changes and the [`Effect`]s for `effects::run` to perform.

use std::collections::HashSet;

use hoprs_core::{Error, RemoteEntry, RemotePath, Site, SiteId, Workspace, WorkspaceId};
use hoprs_vault::UnreadableEntry;

use crate::backend::{ConnectError, ConnectResult, Connected, ProfileSnapshot, SessionId, Side};
use crate::ui::sidebar;
use crate::ui::site_form::{PasswordChange, SiteForm, SiteFormMsg, SiteFormOutcome};
use crate::ui::workspace_form::{WorkspaceForm, WorkspaceFormMsg, WorkspaceFormOutcome};

/// Where the saved sites are.
#[derive(Debug, Clone, PartialEq)]
pub enum Load {
    Loading,
    Loaded,
    Failed(Error),
}

/// The form shown in the center, in place of the tabs.
#[derive(Debug)]
// At most one form exists, so its size doesn't matter.
#[allow(clippy::large_enum_variant)]
pub enum Form {
    Site(SiteForm),
    Workspace(WorkspaceForm),
}

pub mod actions;
pub mod files;

pub use actions::FileAction;
pub use files::{FileKey, Mods, PaneMsg, RequestId, Tab, TabId};

#[derive(Debug, Clone, PartialEq)]
pub enum TabState {
    Connecting,
    Connected(Connected),
    /// This site failed alone: the error shows in its tab.
    Failed(ConnectError),
}

/// A workspace's entry as the sidebar lists it.
#[derive(Debug, PartialEq)]
pub enum WorkspaceRow<'a> {
    Site(&'a Site),
    /// The site is no longer in the vault (or this version can't read it): shown as
    /// "Site missing" and skipped by "Open workspace".
    Missing(SiteId),
}

/// The whole UI state.
#[derive(Debug)]
pub struct App {
    /// Whether the transfer queue is expanded.
    pub queue_open: bool,
    /// The sidebar's site filter, as typed.
    pub site_filter: String,
    pub load: Load,
    /// Saved sites and workspaces, in file order.
    pub sites: Vec<Site>,
    pub workspaces: Vec<Workspace>,
    /// Entries `sites.toml` / `workspaces.toml` hold but this version can't read.
    pub unreadable: Vec<UnreadableEntry>,
    /// Workspaces expanded in the sidebar.
    pub expanded: HashSet<WorkspaceId>,
    pub form: Option<Form>,
    /// One tab per site, created by "Open workspace" or a click on a site.
    pub tabs: Vec<Tab>,
    pub active_tab: Option<usize>,
    /// A sidebar action failed (opening the profile folder).
    pub sidebar_error: Option<Error>,
    /// Where a local pane starts when its site names no folder.
    pub home: Option<RemotePath>,
    /// The modifiers held now, for clicks in the file lists.
    pub mods: Mods,
    /// The last tab or listing id handed out.
    next_id: u64,
}

#[derive(Debug, Clone)]
pub enum Message {
    /// Collapse or expand the transfer queue.
    ToggleQueue,
    /// The sidebar filter changed.
    FilterSites(String),
    ReloadProfiles,
    ProfilesLoaded(Result<ProfileSnapshot, Error>),
    ToggleWorkspace(WorkspaceId),
    OpenWorkspace(WorkspaceId),
    /// A site was clicked: open (or retry) its tab.
    ConnectSite(SiteId),
    /// One site's connect finished (each site of a batch on its own).
    SiteConnected {
        site: SiteId,
        result: ConnectResult,
    },
    SelectTab(usize),
    NewSite,
    EditSite(SiteId),
    NewWorkspace,
    EditWorkspace(WorkspaceId),
    SiteForm(SiteFormMsg),
    WorkspaceForm(WorkspaceFormMsg),
    /// A form's save or delete finished. `op` says which; a result whose form is no
    /// longer the one open still updates the profiles and tabs, never the open form.
    FormDone {
        op: ProfileOp,
        result: Result<(), Error>,
    },
    OpenProfileFolder,
    ProfileFolderOpened(Result<(), Error>),
    /// The × of a tab: closes it and its session.
    CloseTab(TabId),
    /// Something in one pane of a file tab.
    Pane {
        tab: TabId,
        side: Side,
        msg: PaneMsg,
    },
    /// A listing finished; dropped unless `request_id` is the pane's latest request.
    Listed {
        tab: TabId,
        side: Side,
        request_id: RequestId,
        result: Result<Vec<RemoteEntry>, Error>,
    },
    /// A key from the keyboard subscription. Unless it is Escape, it reaches the panes
    /// (as [`Message::Key`]) only if no text field has the focus.
    KeyPressed {
        key: FileKey,
        shift: bool,
    },
    /// A key for the focused pane: no widget took it and no text field has the focus.
    Key {
        key: FileKey,
        shift: bool,
    },
    Modifiers(Mods),
    /// A delete's running count of removed items.
    FileOpProgress {
        tab: TabId,
        side: Side,
        op: RequestId,
        done: u64,
    },
    /// A file operation finished; dropped unless it is the pane's operation in flight.
    FileOpDone {
        tab: TabId,
        side: Side,
        op: RequestId,
        result: Result<(), Error>,
    },
}

/// The profile change a form asked for, carried back with its result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileOp {
    SaveSite(SiteId),
    DeleteSite(SiteId),
    SaveWorkspace(WorkspaceId),
    DeleteWorkspace(WorkspaceId),
}

/// Work for the backend.
#[derive(Debug)]
pub enum Effect {
    LoadProfiles,
    /// Saves the site, then applies the password change (the only holder of a new
    /// password once the form emitted it).
    SaveSite {
        site: Site,
        password: PasswordChange,
    },
    DeleteSite(SiteId),
    SaveWorkspace(Workspace),
    DeleteWorkspace(WorkspaceId),
    /// Connects the sites in parallel with one secret batch; one
    /// [`Message::SiteConnected`] per site, as each finishes.
    ConnectSites(Vec<SiteId>),
    CloseSession(SessionId),
    /// Opens the directory of `sites.toml` with the system file manager.
    OpenProfileFolder,
    /// Lists a pane's folder; the result comes back as [`Message::Listed`]. The remote
    /// side needs `session`; the local side has none.
    List {
        tab: TabId,
        side: Side,
        request_id: RequestId,
        session: Option<SessionId>,
        path: RemotePath,
    },
    /// Scrolls a pane's list to `offset_y` (a new folder, the keyboard cursor, a tab
    /// shown again).
    ScrollPane {
        tab: TabId,
        side: Side,
        offset_y: f32,
    },
    /// A new folder, rename, delete or chmod in one pane; progress comes back as
    /// [`Message::FileOpProgress`], the end as [`Message::FileOpDone`]. The remote side
    /// needs `session`.
    FileOp {
        tab: TabId,
        side: Side,
        op: RequestId,
        session: Option<SessionId>,
        action: FileAction,
    },
    /// Focuses the pane's inline name editor and selects its first `select_end`
    /// characters.
    FocusEdit {
        tab: TabId,
        side: Side,
        select_end: usize,
    },
    /// Sends [`Message::Key`] unless a text field has the focus (the sidebar filter keeps
    /// its arrows, Tab, F2, F7…).
    KeyUnlessTyping {
        key: FileKey,
        shift: bool,
    },
}

impl App {
    pub fn new() -> Self {
        Self {
            queue_open: true,
            site_filter: String::new(),
            load: Load::Loading,
            sites: Vec::new(),
            workspaces: Vec::new(),
            unreadable: Vec::new(),
            expanded: HashSet::new(),
            form: None,
            tabs: Vec::new(),
            active_tab: None,
            sidebar_error: None,
            home: dirs::home_dir().as_deref().and_then(files::local_path),
            mods: Mods::default(),
            next_id: 0,
        }
    }

    /// The app and the effects to run at startup.
    pub fn boot() -> (Self, Vec<Effect>) {
        (Self::new(), vec![Effect::LoadProfiles])
    }

    pub fn site(&self, id: &SiteId) -> Option<&Site> {
        self.sites.iter().find(|s| &s.id == id)
    }

    pub fn workspace(&self, id: &WorkspaceId) -> Option<&Workspace> {
        self.workspaces.iter().find(|w| &w.id == id)
    }

    /// The sites the sidebar lists under the current filter, in file order.
    pub fn visible_sites(&self) -> Vec<&Site> {
        self.sites
            .iter()
            .filter(|s| sidebar::site_matches(s, &self.site_filter))
            .collect()
    }

    /// The workspace's sites in its order, missing ones included.
    pub fn workspace_sites<'a>(&'a self, workspace: &Workspace) -> Vec<WorkspaceRow<'a>> {
        workspace
            .sites
            .iter()
            .map(|entry| match self.site(&entry.site) {
                Some(site) => WorkspaceRow::Site(site),
                None => WorkspaceRow::Missing(entry.site.clone()),
            })
            .collect()
    }

    pub fn tab_of(&self, site: &SiteId) -> Option<usize> {
        self.tabs.iter().position(|t| &t.site == site)
    }

    /// Applies `msg`; returns the effects to run. Never blocks.
    pub fn update(&mut self, msg: Message) -> Vec<Effect> {
        let shown = self.shown_tab();
        let mut effects = self.apply(msg);
        // The center now shows another tab (or a tab instead of a form): its lists'
        // scrollables must match the offsets the panes remember.
        if self.shown_tab() != shown && self.shown_tab().is_some() {
            effects.extend(self.sync_scroll());
        }
        effects
    }

    /// The tab the center shows, if any.
    fn shown_tab(&self) -> Option<TabId> {
        if self.form.is_some() {
            return None;
        }
        self.active_tab
            .and_then(|i| self.tabs.get(i))
            .map(|tab| tab.id)
    }

    fn apply(&mut self, msg: Message) -> Vec<Effect> {
        match msg {
            Message::ToggleQueue => self.queue_open = !self.queue_open,
            Message::FilterSites(filter) => self.site_filter = filter,
            Message::ReloadProfiles => {
                self.load = Load::Loading;
                return vec![Effect::LoadProfiles];
            }
            Message::ProfilesLoaded(Ok(snapshot)) => {
                self.sites = snapshot.sites;
                self.workspaces = snapshot.workspaces;
                self.unreadable = snapshot.unreadable;
                self.load = Load::Loaded;
            }
            Message::ProfilesLoaded(Err(error)) => self.load = Load::Failed(error),
            Message::ToggleWorkspace(id) => {
                if !self.expanded.remove(&id) {
                    self.expanded.insert(id);
                }
            }
            Message::OpenWorkspace(id) => return self.open_workspace(&id),
            Message::ConnectSite(site) => {
                self.form = None;
                return self.open_tabs(vec![site]);
            }
            Message::SiteConnected { site, result } => return self.site_connected(site, result),
            Message::SelectTab(index) => {
                if index < self.tabs.len() {
                    self.active_tab = Some(index);
                    self.form = None;
                }
            }
            Message::NewSite => self.form = Some(Form::Site(SiteForm::new_site())),
            Message::EditSite(id) => {
                if let Some(site) = self.site(&id) {
                    self.form = Some(Form::Site(SiteForm::edit(site)));
                }
            }
            Message::NewWorkspace => {
                self.form = Some(Form::Workspace(WorkspaceForm::new_workspace()));
            }
            Message::EditWorkspace(id) => {
                if let Some(workspace) = self.workspace(&id) {
                    self.form = Some(Form::Workspace(WorkspaceForm::edit(workspace)));
                }
            }
            Message::SiteForm(m) => return self.site_form(m),
            Message::WorkspaceForm(m) => return self.workspace_form(m),
            Message::FormDone { op, result } => return self.form_done(op, result),
            Message::OpenProfileFolder => return vec![Effect::OpenProfileFolder],
            Message::ProfileFolderOpened(result) => self.sidebar_error = result.err(),
            Message::CloseTab(id) => {
                if let Some(index) = self.tab_index(id) {
                    return self.close_tab(index);
                }
            }
            Message::Pane { tab, side, msg } => return self.pane_msg(tab, side, msg),
            Message::Listed {
                tab,
                side,
                request_id,
                result,
            } => return self.listed(tab, side, request_id, result),
            Message::KeyPressed { key, shift } => return self.key_pressed(key, shift),
            Message::Key { key, shift } => return self.file_key(key, shift),
            Message::Modifiers(mods) => self.mods = mods,
            Message::FileOpProgress {
                tab,
                side,
                op,
                done,
            } => return self.file_op_progress(tab, side, op, done),
            Message::FileOpDone {
                tab,
                side,
                op,
                result,
            } => return self.file_op_done(tab, side, op, result),
        }
        Vec::new()
    }

    /// Open workspace: a tab per site that still exists (missing ones are skipped and
    /// shown inline in the sidebar), all connected in one batch.
    fn open_workspace(&mut self, id: &WorkspaceId) -> Vec<Effect> {
        let Some(workspace) = self.workspace(id) else {
            return Vec::new();
        };
        let sites: Vec<SiteId> = self
            .workspace_sites(workspace)
            .into_iter()
            .filter_map(|row| match row {
                WorkspaceRow::Site(site) => Some(site.id.clone()),
                WorkspaceRow::Missing(_) => None,
            })
            .collect();
        self.form = None;
        self.open_tabs(sites)
    }

    /// Makes sure each site has a tab, focuses the first, and connects the sites that are
    /// neither connected nor already connecting (a failed tab is retried), as one batch.
    /// A new tab also lists its local folder.
    fn open_tabs(&mut self, sites: Vec<SiteId>) -> Vec<Effect> {
        let mut first = None;
        let mut connect = Vec::new();
        let mut listings = Vec::new();
        for site in sites {
            let index = match self.tab_of(&site) {
                Some(index) => {
                    let tab = &mut self.tabs[index];
                    if matches!(tab.state, TabState::Failed(_)) {
                        tab.state = TabState::Connecting;
                        connect.push(site);
                    }
                    index
                }
                None => {
                    let (tab, listing) = self.new_tab(site.clone());
                    self.tabs.push(tab);
                    listings.push(listing);
                    connect.push(site);
                    self.tabs.len() - 1
                }
            };
            first.get_or_insert(index);
        }
        if first.is_some() {
            self.active_tab = first;
        }
        let mut effects = Vec::new();
        if !connect.is_empty() {
            effects.push(Effect::ConnectSites(connect));
        }
        effects.extend(listings);
        effects
    }

    /// One site's result lands in its own tab, whatever the other sites do.
    fn site_connected(&mut self, site: SiteId, result: ConnectResult) -> Vec<Effect> {
        let Some(index) = self.tab_of(&site) else {
            // Its tab is gone: don't leave the session open behind the user's back.
            let session = match result {
                Ok(connected) => Some(connected.session),
                Err(failure) => failure.session,
            };
            return session.map(Effect::CloseSession).into_iter().collect();
        };
        match result {
            Ok(connected) => {
                self.tabs[index].state = TabState::Connected(connected);
                // The remote pane lists its folder now that it can.
                self.request_listing(index, Side::Remote)
            }
            Err(failure) => {
                self.tabs[index].state = TabState::Failed(failure);
                Vec::new()
            }
        }
    }

    fn site_form(&mut self, msg: SiteFormMsg) -> Vec<Effect> {
        let Some(Form::Site(form)) = &mut self.form else {
            return Vec::new();
        };
        match form.update(msg) {
            SiteFormOutcome::Nothing => Vec::new(),
            SiteFormOutcome::Close => {
                self.form = None;
                Vec::new()
            }
            SiteFormOutcome::Save { site, password } => vec![Effect::SaveSite {
                site: *site,
                password,
            }],
            SiteFormOutcome::Delete(id) => vec![Effect::DeleteSite(id)],
        }
    }

    fn workspace_form(&mut self, msg: WorkspaceFormMsg) -> Vec<Effect> {
        let Some(Form::Workspace(form)) = &mut self.form else {
            return Vec::new();
        };
        match form.update(msg, &self.sites) {
            WorkspaceFormOutcome::Nothing => Vec::new(),
            WorkspaceFormOutcome::Close => {
                self.form = None;
                Vec::new()
            }
            WorkspaceFormOutcome::Save(workspace) => vec![Effect::SaveWorkspace(workspace)],
            WorkspaceFormOutcome::Delete(id) => vec![Effect::DeleteWorkspace(id)],
        }
    }

    /// A form's save or delete finished.
    ///
    /// Whatever form is open now, a success reloads the profiles and fixes up the tabs:
    /// - a saved site's session that is not connected is closed, so a retry starts over
    ///   with the new settings instead of retrying the old connection;
    /// - a deleted site's tabs and session go away.
    ///
    /// The form itself is closed (success) or shows the error (failure) only if it is the
    /// one that asked: same target, still waiting for this result.
    fn form_done(&mut self, op: ProfileOp, result: Result<(), Error>) -> Vec<Effect> {
        let waiting = match (&mut self.form, &op) {
            (Some(Form::Site(form)), ProfileOp::SaveSite(id) | ProfileOp::DeleteSite(id)) => {
                form.busy && form.site_id() == id
            }
            (
                Some(Form::Workspace(form)),
                ProfileOp::SaveWorkspace(id) | ProfileOp::DeleteWorkspace(id),
            ) => form.busy && form.workspace_id() == id,
            _ => false,
        };
        let error = result.err();
        if waiting {
            match (&error, &mut self.form) {
                (None, _) => self.form = None,
                (Some(error), Some(Form::Site(form))) => form.finished(Some(error.clone())),
                (Some(error), Some(Form::Workspace(form))) => form.finished(Some(error.clone())),
                (Some(_), None) => {}
            }
        }
        if error.is_some() {
            return Vec::new();
        }
        let mut effects = vec![Effect::LoadProfiles];
        match op {
            ProfileOp::SaveSite(site) => effects.extend(self.drop_stale_session(&site)),
            ProfileOp::DeleteSite(site) => effects.extend(self.close_site_tabs(&site)),
            ProfileOp::SaveWorkspace(_) | ProfileOp::DeleteWorkspace(_) => {}
        }
        effects
    }

    /// The site's settings changed: a failed tab forgets its session (closed here), so
    /// the next connect builds a new one from the saved site. A connected session keeps
    /// running; the new settings apply to its next connection.
    fn drop_stale_session(&mut self, site: &SiteId) -> Option<Effect> {
        let index = self.tab_of(site)?;
        let tab = &mut self.tabs[index];
        let TabState::Failed(failure) = &mut tab.state else {
            return None;
        };
        failure.session.take().map(Effect::CloseSession)
    }

    /// Removes the site's tab and closes its session, if any.
    fn close_site_tabs(&mut self, site: &SiteId) -> Vec<Effect> {
        match self.tab_of(site) {
            Some(index) => self.close_tab(index),
            None => Vec::new(),
        }
    }

    /// Removes the tab at `index` and closes its session, if any. A listing still in
    /// flight for it finds no tab and is dropped.
    fn close_tab(&mut self, index: usize) -> Vec<Effect> {
        let tab = self.tabs.remove(index);
        self.active_tab = match self.active_tab {
            _ if self.tabs.is_empty() => None,
            Some(active) if active > index => Some(active - 1),
            Some(active) => Some(active.min(self.tabs.len() - 1)),
            None => None,
        };
        let session = match tab.state {
            TabState::Connected(connected) => Some(connected.session),
            TabState::Failed(failure) => failure.session,
            TabState::Connecting => None,
        };
        session.map(Effect::CloseSession).into_iter().collect()
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
