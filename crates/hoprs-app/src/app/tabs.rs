//! The session tabs: what a tab is, which tabs a site opens with, and how a site's tabs
//! share its session.
//!
//! A tab is a terminal or a files view ([`TabKind`]) of one site. The backend keeps one
//! session per site, so all the tabs of a site share one session: "Open as" (or a
//! workspace's saved tabs) opens several tabs with one connect, and "New terminal tab" /
//! "New files tab" add one to the session already connected, without a connect.
//!
//! The connection fields of a tab (`state`, `conn`, `auth`, `reconnecting`,
//! `failed_endpoint`) are the same in every tab of its site: everything that changes them
//! (a connect's result, the session's feed, a reconnect, a restart, a moved or edited
//! site) changes them in all those tabs at once. The session goes away only with the last
//! tab of its site: closing one tab keeps it for the others, and so does the abandon of
//! a connect's dialogs ([`App::abandon_site_prompts`]), which only the last tab of a
//! connecting site does.

use hoprs_core::{
    ConnectionState, Error, OpenAs, Protocol, RemotePath, SavedTab, Site, SiteId, TabKind,
    WorkspaceId,
};

use super::files::{Listing, Pane, PaneStatus, Restore, local_path};
use super::{App, Effect, KnownHostsFix, RequestId, WorkspaceRow};
use crate::backend::{AuthMethod, ConnectError, ConnectResult, Connected, SessionId, Side};

/// A tab's identity, stable while tabs open and close around it.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TabId(pub u64);

#[derive(Debug, Clone, PartialEq)]
pub enum TabState {
    Connecting,
    Connected(Connected),
    /// This site failed alone: the error shows in its tabs.
    Failed(ConnectError),
}

/// One tab: a site, its connection and, for a files tab, its two panes. A terminal tab
/// has panes too but never lists them.
#[derive(Debug, Clone, PartialEq)]
pub struct Tab {
    pub id: TabId,
    pub site: SiteId,
    pub kind: TabKind,
    pub state: TabState,
    pub local: Pane,
    pub remote: Pane,
    /// The pane the keyboard drives.
    pub focus: Side,
    /// "Remove the old entry…" after a changed host key.
    pub known_hosts: KnownHostsFix,
    /// The site's `host:port` when its connect failed (the endpoint "Remove the old
    /// entry…" works on).
    pub failed_endpoint: Option<(String, u16)>,
    /// The connected session's state, as its feed last reported it (`None` until then).
    pub conn: Option<ConnectionState>,
    /// How the session logged in (SSH), as its feed last reported it.
    pub auth: Option<AuthMethod>,
    /// The banner's "Reconnect" is in flight.
    pub reconnecting: bool,
}

impl Tab {
    pub fn is_files(&self) -> bool {
        self.kind == TabKind::Files
    }

    pub fn pane(&self, side: Side) -> &Pane {
        match side {
            Side::Local => &self.local,
            Side::Remote => &self.remote,
        }
    }

    pub fn pane_mut(&mut self, side: Side) -> &mut Pane {
        match side {
            Side::Local => &mut self.local,
            Side::Remote => &mut self.remote,
        }
    }

    /// The connected session, if any.
    pub fn session(&self) -> Option<SessionId> {
        match &self.state {
            TabState::Connected(connected) => Some(connected.session),
            TabState::Connecting | TabState::Failed(_) => None,
        }
    }

    pub fn pane_status(&self, side: Side) -> PaneStatus<'_> {
        if side == Side::Remote {
            match self.state {
                TabState::Connecting => return PaneStatus::Loading,
                TabState::Failed(_) => return PaneStatus::NotConnected,
                TabState::Connected(_) => {}
            }
        }
        let pane = self.pane(side);
        match &pane.listing {
            Listing::Idle | Listing::Loading => PaneStatus::Loading,
            Listing::Failed(error) => PaneStatus::Failed(error),
            Listing::Loaded if pane.entries.is_empty() => PaneStatus::Empty,
            Listing::Loaded => PaneStatus::Full,
        }
    }

    /// Takes the connection fields of `other`, a tab of the same site.
    fn share_connection(&mut self, other: &Tab) {
        self.state = other.state.clone();
        self.conn = other.conn.clone();
        self.auth = other.auth.clone();
        self.reconnecting = other.reconnecting;
        self.failed_endpoint = other.failed_endpoint.clone();
    }

    /// Forgets what the session's feed said (a new session, or none).
    pub(super) fn forget_feed(&mut self) {
        self.conn = None;
        self.auth = None;
        self.reconnecting = false;
    }
}

/// Whether `site` has a terminal: SSH sites only (FTP is files only).
fn has_terminal(site: Option<&Site>) -> bool {
    site.is_some_and(|s| s.protocol == Protocol::Ssh)
}

/// The tabs a site opens with, each with the saved tab it restores (if any):
/// - the workspace's saved tabs, in their order, if it saved any (a terminal tab of a
///   site that has no terminal is dropped);
/// - else what the site's "Open as" says (SSH): Files, Terminal, or Both (terminal
///   first). FTP sites always open files.
pub fn tab_plan(site: Option<&Site>, saved: &[SavedTab]) -> Vec<(TabKind, Option<SavedTab>)> {
    let terminal = has_terminal(site);
    let restored: Vec<_> = saved
        .iter()
        .filter(|t| terminal || t.kind == TabKind::Files)
        // A terminal tab saves no folders worth restoring.
        .map(|t| (t.kind, (t.kind == TabKind::Files).then(|| t.clone())))
        .collect();
    if !restored.is_empty() {
        return restored;
    }
    let open_as = match site {
        Some(site) if terminal => site.ssh.open_as,
        _ => OpenAs::Files,
    };
    let kinds: &[TabKind] = match open_as {
        OpenAs::Files => &[TabKind::Files],
        OpenAs::Terminal => &[TabKind::Terminal],
        OpenAs::Both => &[TabKind::Terminal, TabKind::Files],
    };
    kinds.iter().map(|&kind| (kind, None)).collect()
}

impl App {
    pub fn tab_index(&self, id: TabId) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }

    /// The site's first tab (the one focused when the site is clicked again).
    pub fn tab_of(&self, site: &SiteId) -> Option<usize> {
        self.tabs.iter().position(|t| &t.site == site)
    }

    /// The indices of the site's tabs, in strip order.
    pub fn tabs_of(&self, site: &SiteId) -> Vec<usize> {
        (0..self.tabs.len())
            .filter(|&i| &self.tabs[i].site == site)
            .collect()
    }

    /// The indices of the tabs on `session`, in strip order.
    pub(super) fn session_tabs(&self, session: SessionId) -> Vec<usize> {
        (0..self.tabs.len())
            .filter(|&i| self.tabs[i].session() == Some(session))
            .collect()
    }

    /// Applies `f` to every tab of `site`.
    pub(super) fn each_site_tab(&mut self, site: &SiteId, mut f: impl FnMut(&mut Tab)) {
        self.tabs
            .iter_mut()
            .filter(|t| &t.site == site)
            .for_each(&mut f);
    }

    /// Whether "New terminal tab" / "New files tab" can add a tab next to tab `index`:
    /// its site is SSH and its session is connected (not lost).
    pub fn can_open_session_tab(&self, index: usize) -> bool {
        let Some(tab) = self.tabs.get(index) else {
            return false;
        };
        has_terminal(self.site(&tab.site))
            && matches!(tab.state, TabState::Connected(_))
            && tab.lost_connection().is_none()
    }

    /// A new tab for `site`, and the listing it runs (a files tab lists its local side;
    /// a terminal tab lists nothing). Its panes start at the folders of `restored` (a
    /// workspace's saved tab), else at the site's (local: `local_dir`, else home; remote:
    /// `remote_dir`, else `/`). A saved folder that is not a usable absolute path falls
    /// back the same way.
    pub(super) fn new_tab(
        &mut self,
        site: SiteId,
        kind: TabKind,
        restored: Option<&SavedTab>,
    ) -> (Tab, Vec<Effect>) {
        let saved = self.site(&site);
        let remote_path = |dir: &str| RemotePath::new(dir).ok();
        let local = restored
            .and_then(|t| t.local_dir.as_deref())
            .and_then(local_path)
            .or_else(|| {
                saved
                    .and_then(|s| s.local_dir.as_deref())
                    .and_then(local_path)
            })
            .or_else(|| self.home.clone())
            .unwrap_or_else(RemotePath::root);
        let remote = restored
            .and_then(|t| t.remote_dir.as_deref())
            .and_then(remote_path)
            .or_else(|| {
                saved
                    .and_then(|s| s.remote_dir.as_deref())
                    .and_then(remote_path)
            })
            .unwrap_or_else(RemotePath::root);
        let id = TabId(self.next_id());
        let mut tab = Tab {
            id,
            site,
            kind,
            state: TabState::Connecting,
            local: Pane::new(local.clone()),
            remote: Pane::new(remote),
            focus: Side::Local,
            known_hosts: KnownHostsFix::Idle,
            failed_endpoint: None,
            conn: None,
            auth: None,
            reconnecting: false,
        };
        if kind == TabKind::Terminal {
            return (tab, Vec::new());
        }
        // A folder from the saved tab may be gone since: its first listing falls back.
        let from_saved = |dir: Option<bool>| dir.unwrap_or(false).then_some(Restore::Saved);
        tab.local.restore = from_saved(
            restored
                .and_then(|t| t.local_dir.as_deref())
                .map(|d| local_path(d).as_ref() == Some(&tab.local.path)),
        );
        tab.remote.restore = from_saved(
            restored
                .and_then(|t| t.remote_dir.as_deref())
                .map(|d| RemotePath::new(d).ok().as_ref() == Some(&tab.remote.path)),
        );
        let request_id = RequestId(self.next_id());
        tab.local.listing = Listing::Loading;
        tab.local.request = Some(request_id);
        let effect = Effect::List {
            tab: id,
            side: Side::Local,
            request_id,
            session: None,
            path: local,
        };
        (tab, vec![effect])
    }

    /// Open workspace: the tabs of each site that still exists (missing ones are skipped
    /// and shown inline in the sidebar), as it saved them, all connected in one batch.
    /// Another workspace open until now has its tabs saved first (they stay open).
    pub(super) fn open_workspace_tabs(&mut self, id: &WorkspaceId) -> Vec<Effect> {
        let Some(workspace) = self.workspace(id) else {
            return Vec::new();
        };
        let sites: Vec<(SiteId, Vec<SavedTab>)> = self
            .workspace_sites(workspace)
            .into_iter()
            .zip(&workspace.sites)
            .filter_map(|(row, entry)| match row {
                WorkspaceRow::Site(site) => Some((site.id.clone(), entry.tabs.clone())),
                WorkspaceRow::Missing(_) => None,
            })
            .collect();
        let mut effects: Vec<Effect> = match self.open_workspace.replace(id.clone()) {
            Some(previous) if &previous != id => {
                self.save_tabs_effect(&previous).into_iter().collect()
            }
            _ => Vec::new(),
        };
        self.form = None;
        effects.extend(self.open_tabs(sites));
        effects
    }

    /// Makes sure each site has its tabs ([`tab_plan`], from the saved tabs given),
    /// focuses the first site's first tab, and connects the sites that are neither
    /// connected nor already connecting (a failed site is retried), as one batch: one
    /// connect per site, however many tabs it has. A site that already has tabs gets no
    /// more.
    pub(super) fn open_tabs(&mut self, sites: Vec<(SiteId, Vec<SavedTab>)>) -> Vec<Effect> {
        let mut first = None;
        let mut connect = Vec::new();
        let mut listings = Vec::new();
        for (site, saved) in sites {
            self.readopt_site(&site);
            let index = match self.tab_of(&site) {
                Some(index) => {
                    if matches!(self.tabs[index].state, TabState::Failed(_)) {
                        self.each_site_tab(&site, |tab| tab.state = TabState::Connecting);
                        connect.push(site);
                    }
                    index
                }
                None => {
                    let start = self.tabs.len();
                    for (kind, restored) in tab_plan(self.site(&site), &saved) {
                        let (tab, effects) = self.new_tab(site.clone(), kind, restored.as_ref());
                        self.tabs.push(tab);
                        listings.extend(effects);
                    }
                    connect.push(site);
                    start
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

    /// One site's result lands in all its tabs, whatever the other sites do.
    pub(super) fn site_connected(&mut self, site: SiteId, result: ConnectResult) -> Vec<Effect> {
        let Some(first) = self.tab_of(&site) else {
            self.readopt_site(&site);
            // Its tabs are gone: don't leave the session open behind the user's back.
            let session = match result {
                Ok(connected) => Some(connected.session),
                Err(failure) => failure.session,
            };
            if let Some(session) = session {
                self.forget_session(session);
            }
            return session.map(Effect::CloseSession).into_iter().collect();
        };
        // A new session's feed tells its state and login from now on. The session already
        // watched keeps what its feed said: the feed only reports changes from here.
        let same = matches!(&result, Ok(c) if self.tabs[first].session() == Some(c.session));
        self.each_site_tab(&site, |tab| {
            tab.known_hosts = KnownHostsFix::Idle;
            if !same {
                tab.forget_feed();
            }
        });
        match result {
            Ok(connected) => {
                self.queue.site_restored(&site);
                let mut effects = self.settle_saving(connected.session, true);
                self.each_site_tab(&site, |tab| {
                    tab.state = TabState::Connected(connected);
                });
                // The remote panes list their folders now that they can.
                for index in self.tabs_of(&site) {
                    effects.extend(self.request_listing(index, Side::Remote));
                }
                effects
            }
            Err(failure) => {
                let effects = match failure.session {
                    Some(session) => self.settle_saving(session, false),
                    None => Vec::new(),
                };
                // Where it failed, as the profile said then: "Remove the old entry…"
                // works on that endpoint even if the site is edited meanwhile.
                self.note_failed_endpoint(&site);
                self.each_site_tab(&site, |tab| tab.state = TabState::Failed(failure.clone()));
                effects
            }
        }
    }

    /// "New terminal tab" / "New files tab" on tab `tab`: a `kind` tab of the same site,
    /// right after it, on the session already connected (no connect). A new files tab
    /// lists both its sides at once.
    pub(super) fn new_session_tab(&mut self, tab: TabId, kind: TabKind) -> Vec<Effect> {
        let Some(index) = self.tab_index(tab) else {
            return Vec::new();
        };
        if !self.can_open_session_tab(index) {
            return Vec::new();
        }
        let site = self.tabs[index].site.clone();
        let (mut new, mut effects) = self.new_tab(site, kind, None);
        new.share_connection(&self.tabs[index]);
        let at = index + 1;
        self.tabs.insert(at, new);
        self.active_tab = Some(at);
        self.form = None;
        effects.extend(self.request_listing(at, Side::Remote));
        effects
    }

    /// Removes every tab of the site, and closes its session once.
    pub(super) fn close_site_tabs(&mut self, site: &SiteId) -> Vec<Effect> {
        let mut effects = Vec::new();
        while let Some(index) = self.tab_of(site) {
            effects.extend(self.close_tab(index));
        }
        effects
    }

    /// Removes the tab at `index`. A listing still in flight for it finds no tab and is
    /// dropped. The last tab of its site also closes the session, if any; a last tab still
    /// connecting takes the site's dialogs with it
    /// ([`abandon_site_prompts`](Self::abandon_site_prompts)). Other tabs of the site
    /// keep the session (and its connect) as it is.
    pub(super) fn close_tab(&mut self, index: usize) -> Vec<Effect> {
        let tab = self.tabs.remove(index);
        self.active_tab = match self.active_tab {
            _ if self.tabs.is_empty() => None,
            Some(active) if active > index => Some(active - 1),
            Some(active) => Some(active.min(self.tabs.len() - 1)),
            None => None,
        };
        if self.tab_of(&tab.site).is_some() {
            return Vec::new();
        }
        let session = match tab.state {
            TabState::Connected(connected) => Some(connected.session),
            TabState::Failed(failure) => failure.session,
            // The connect's session isn't known yet: its dialogs say which it is.
            TabState::Connecting => return self.abandon_site_prompts(&tab.site),
        };
        if let Some(session) = session {
            self.forget_session(session);
        }
        session.map(Effect::CloseSession).into_iter().collect()
    }

    /// The site's host or port changed: its session no longer matches the profile. A
    /// connected one is closed (once) and every tab of the site shows why.
    pub(super) fn drop_moved_session(&mut self, site: &SiteId) -> Option<Effect> {
        let index = self.tab_of(site)?;
        match &self.tabs[index].state {
            TabState::Connected(connected) => {
                let session = connected.session;
                let failure = ConnectError {
                    session: None,
                    error: Error::new(
                        hoprs_core::ErrorKind::Conflict,
                        crate::ui::strings::SITE_ENDPOINT_CHANGED,
                    ),
                    keyring_unavailable: false,
                };
                self.each_site_tab(site, |tab| {
                    tab.state = TabState::Failed(failure.clone());
                    tab.forget_feed();
                });
                self.forget_session(session);
                Some(Effect::CloseSession(session))
            }
            TabState::Failed(_) => self.drop_stale_session(site),
            TabState::Connecting => None,
        }
    }

    /// The site's settings changed: its failed tabs forget their session (closed here,
    /// once), so the next connect builds a new one from the saved site. A connected
    /// session keeps running; the new settings apply to its next connection.
    pub(super) fn drop_stale_session(&mut self, site: &SiteId) -> Option<Effect> {
        let index = self.tab_of(site)?;
        let TabState::Failed(failure) = &self.tabs[index].state else {
            return None;
        };
        let session = failure.session?;
        self.each_site_tab(site, |tab| {
            if let TabState::Failed(failure) = &mut tab.state {
                failure.session = None;
            }
        });
        self.forget_session(session);
        Some(Effect::CloseSession(session))
    }
}
