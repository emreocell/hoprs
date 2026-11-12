//! The application state and its pure update: no I/O here, only state
//! changes and the [`Effect`]s for `effects::run` to perform.

use std::collections::HashSet;

use hoprs_core::{Error, Site, SiteId, Workspace, WorkspaceId};
use hoprs_vault::UnreadableEntry;

use crate::backend::{ConnectError, ConnectResult, Connected, ProfileSnapshot, SessionId};
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

/// One tab per site, created by "Open workspace" or a click on a site. This task only
/// tracks its connection; Task 6 gives it file panes.
#[derive(Debug, Clone, PartialEq)]
pub struct Tab {
    pub site: SiteId,
    pub state: TabState,
}

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
    pub tabs: Vec<Tab>,
    pub active_tab: Option<usize>,
    /// A sidebar action failed (opening the profile folder).
    pub sidebar_error: Option<Error>,
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
    /// The open form's save or delete finished.
    FormDone(Result<(), Error>),
    OpenProfileFolder,
    ProfileFolderOpened(Result<(), Error>),
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
            Message::FormDone(result) => return self.form_done(result),
            Message::OpenProfileFolder => return vec![Effect::OpenProfileFolder],
            Message::ProfileFolderOpened(result) => self.sidebar_error = result.err(),
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
    fn open_tabs(&mut self, sites: Vec<SiteId>) -> Vec<Effect> {
        let mut first = None;
        let mut connect = Vec::new();
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
                    self.tabs.push(Tab {
                        site: site.clone(),
                        state: TabState::Connecting,
                    });
                    connect.push(site);
                    self.tabs.len() - 1
                }
            };
            first.get_or_insert(index);
        }
        if first.is_some() {
            self.active_tab = first;
        }
        if connect.is_empty() {
            return Vec::new();
        }
        vec![Effect::ConnectSites(connect)]
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
        self.tabs[index].state = match result {
            Ok(connected) => TabState::Connected(connected),
            Err(failure) => TabState::Failed(failure),
        };
        Vec::new()
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

    /// A form's save or delete finished: success closes it and reloads the profiles (the
    /// backend may have changed more than the form knows); failure shows in the form.
    fn form_done(&mut self, result: Result<(), Error>) -> Vec<Effect> {
        match result {
            Ok(()) => {
                self.form = None;
                vec![Effect::LoadProfiles]
            }
            Err(error) => {
                match &mut self.form {
                    Some(Form::Site(form)) => form.finished(Some(error)),
                    Some(Form::Workspace(form)) => form.finished(Some(error)),
                    None => {}
                }
                Vec::new()
            }
        }
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
