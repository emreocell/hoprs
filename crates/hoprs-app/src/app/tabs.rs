//! The session tabs: what a tab is, which tabs a site opens with, and how a site's tabs
//! share its session.
//!
//! A tab is a terminal or a files view ([`TabKind`]) of one site. The backend keeps one
//! session per site, so all the tabs of a site share one session: "Open as" (or a
//! workspace's saved tabs) opens several tabs with one connect, and "New terminal tab" /
//! "New files tab" add one to the session already connected, without a connect.
//!
//! The connection is not the tabs': it lives once per site, in a [`SiteConn`] that
//! [`App`] holds while the site has tabs. Every writer (a connect's result, the session's
//! feed, a reconnect, a restart, a moved or edited site, the `known_hosts` fix) changes
//! that one record, and every tab of the site, old or opened later, reads it. A tab holds
//! only what is its own ([`TabBody`]): a files tab its panes and focus, a terminal tab its
//! terminal and shell ([`super::terminal`]). The session goes
//! away only with the last tab of its site: closing one tab keeps it for the others, and
//! so does the abandon of a connect's dialogs ([`App::abandon_site_prompts`]), which only
//! the last tab of a connecting site does.

use hoprs_core::{
    ConnectionState, Error, OpenAs, Protocol, RemotePath, SavedTab, Site, SiteId, TabKind,
    WorkspaceId,
};

use super::files::{Listing, Pane, PaneStatus, Restore, local_path};
use super::terminal::TerminalTab;
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

/// A site's connection, shared by all its tabs (one record per site with tabs).
#[derive(Debug, Clone, PartialEq)]
pub struct SiteConn {
    pub state: TabState,
    /// The connected session's state, as its feed last reported it (`None` until then).
    pub conn: Option<ConnectionState>,
    /// How the session logged in (SSH), as its feed last reported it.
    pub auth: Option<AuthMethod>,
    /// The banner's "Reconnect" is in flight.
    pub reconnecting: bool,
    /// The site's `host:port` when its connect failed (the endpoint "Remove the old
    /// entry…" works on).
    pub failed_endpoint: Option<(String, u16)>,
    /// "Remove the old entry…" after a changed host key.
    pub known_hosts: KnownHostsFix,
}

/// What a site's tabs read before it has a record (never, while it has tabs).
static CONNECTING: SiteConn = SiteConn::connecting();

impl SiteConn {
    pub const fn connecting() -> Self {
        Self {
            state: TabState::Connecting,
            conn: None,
            auth: None,
            reconnecting: false,
            failed_endpoint: None,
            known_hosts: KnownHostsFix::Idle,
        }
    }

    /// The connected session, if any.
    pub fn session(&self) -> Option<SessionId> {
        match &self.state {
            TabState::Connected(connected) => Some(connected.session),
            TabState::Connecting | TabState::Failed(_) => None,
        }
    }

    /// Forgets what the session's feed said (a new session, or none).
    fn forget_feed(&mut self) {
        self.conn = None;
        self.auth = None;
        self.reconnecting = false;
    }
}

/// A files tab's own state: its two panes and the one the keyboard drives.
#[derive(Debug, Clone, PartialEq)]
pub struct FilePanes {
    pub local: Pane,
    pub remote: Pane,
    /// The pane the keyboard drives.
    pub focus: Side,
}

/// What a tab holds besides its site: the panes of a files tab, or a terminal.
#[derive(Debug)]
// One per tab, so the size of the larger variant doesn't matter.
#[allow(clippy::large_enum_variant)]
pub enum TabBody {
    Files(FilePanes),
    Terminal(Box<TerminalTab>),
}

/// One tab: a site and its kind's own state ([`TabBody`]). Its connection is its site's
/// [`SiteConn`].
#[derive(Debug)]
pub struct Tab {
    pub id: TabId,
    pub site: SiteId,
    pub body: TabBody,
}

impl Tab {
    pub fn kind(&self) -> TabKind {
        match self.body {
            TabBody::Files(_) => TabKind::Files,
            TabBody::Terminal(_) => TabKind::Terminal,
        }
    }

    pub fn is_files(&self) -> bool {
        matches!(self.body, TabBody::Files(_))
    }

    /// The panes of a files tab.
    pub fn files(&self) -> Option<&FilePanes> {
        match &self.body {
            TabBody::Files(files) => Some(files),
            TabBody::Terminal(_) => None,
        }
    }

    pub fn files_mut(&mut self) -> Option<&mut FilePanes> {
        match &mut self.body {
            TabBody::Files(files) => Some(files),
            TabBody::Terminal(_) => None,
        }
    }

    /// The terminal of a terminal tab.
    pub fn terminal(&self) -> Option<&TerminalTab> {
        match &self.body {
            TabBody::Terminal(terminal) => Some(terminal),
            TabBody::Files(_) => None,
        }
    }

    pub fn terminal_mut(&mut self) -> Option<&mut TerminalTab> {
        match &mut self.body {
            TabBody::Terminal(terminal) => Some(terminal),
            TabBody::Files(_) => None,
        }
    }

    /// Pane `side` of a files tab.
    pub fn pane(&self, side: Side) -> Option<&Pane> {
        self.files().map(|files| match side {
            Side::Local => &files.local,
            Side::Remote => &files.remote,
        })
    }

    pub fn pane_mut(&mut self, side: Side) -> Option<&mut Pane> {
        self.files_mut().map(|files| match side {
            Side::Local => &mut files.local,
            Side::Remote => &mut files.remote,
        })
    }

    /// The pane the keyboard drives (a files tab's).
    pub fn focus(&self) -> Option<Side> {
        self.files().map(|files| files.focus)
    }

    /// Gives the keyboard to pane `side` (a files tab's; nothing for a terminal).
    pub fn set_focus(&mut self, side: Side) {
        if let Some(files) = self.files_mut() {
            files.focus = side;
        }
    }

    /// What a pane shows, given its site's connection `conn`. A terminal tab has no pane:
    /// it says "loading".
    pub fn pane_status<'a>(&'a self, conn: &SiteConn, side: Side) -> PaneStatus<'a> {
        if side == Side::Remote {
            match conn.state {
                TabState::Connecting => return PaneStatus::Loading,
                TabState::Failed(_) => return PaneStatus::NotConnected,
                TabState::Connected(_) => {}
            }
        }
        let Some(pane) = self.pane(side) else {
            return PaneStatus::Loading;
        };
        match &pane.listing {
            Listing::Idle | Listing::Loading => PaneStatus::Loading,
            Listing::Failed(error) => PaneStatus::Failed(error),
            Listing::Loaded if pane.entries.is_empty() => PaneStatus::Empty,
            Listing::Loaded => PaneStatus::Full,
        }
    }

    /// For the tests: pane `side` of a files tab (panics for a terminal).
    #[cfg(test)]
    pub fn local(&self) -> &Pane {
        &self.files().expect("a files tab").local
    }

    #[cfg(test)]
    pub fn remote(&self) -> &Pane {
        &self.files().expect("a files tab").remote
    }

    #[cfg(test)]
    pub fn local_mut(&mut self) -> &mut Pane {
        &mut self.files_mut().expect("a files tab").local
    }

    #[cfg(test)]
    pub fn remote_mut(&mut self) -> &mut Pane {
        &mut self.files_mut().expect("a files tab").remote
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
    /// The connection of `site`, shared by its tabs.
    pub fn conn(&self, site: &SiteId) -> &SiteConn {
        self.conns.get(site).unwrap_or(&CONNECTING)
    }

    /// The connection of the tab at `index`.
    pub fn tab_conn(&self, index: usize) -> &SiteConn {
        self.conn(&self.tabs[index].site)
    }

    /// The one record to change the connection of `site` (created if missing).
    pub(super) fn conn_mut(&mut self, site: &SiteId) -> &mut SiteConn {
        self.conns
            .entry(site.clone())
            .or_insert_with(SiteConn::connecting)
    }

    /// The session of the tab at `index`, if connected.
    pub fn tab_session(&self, index: usize) -> Option<SessionId> {
        self.tab_conn(index).session()
    }

    /// The site whose connected session is `session`.
    pub(super) fn site_of_session(&self, session: SessionId) -> Option<SiteId> {
        self.conns
            .iter()
            .find(|(_, conn)| conn.session() == Some(session))
            .map(|(site, _)| site.clone())
    }

    /// What pane `side` of the tab at `index` shows.
    pub fn pane_status(&self, index: usize, side: Side) -> PaneStatus<'_> {
        self.tabs[index].pane_status(self.tab_conn(index), side)
    }

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

    /// Whether "New terminal tab" / "New files tab" can add a tab next to tab `index`:
    /// its site is SSH and its session is connected (not lost).
    pub fn can_open_session_tab(&self, index: usize) -> bool {
        let Some(tab) = self.tabs.get(index) else {
            return false;
        };
        let conn = self.conn(&tab.site);
        has_terminal(self.site(&tab.site))
            && matches!(conn.state, TabState::Connected(_))
            && conn.lost_connection().is_none()
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
        let (mut tab, effects) = self.new_tab_at(site, kind, local, remote);
        if let Some(files) = tab.files_mut() {
            // A folder from the saved tab may be gone since: its first listing falls back.
            let from_saved = |dir: Option<bool>| dir.unwrap_or(false).then_some(Restore::Saved);
            files.local.restore = from_saved(
                restored
                    .and_then(|t| t.local_dir.as_deref())
                    .map(|d| local_path(d).as_ref() == Some(&files.local.path)),
            );
            files.remote.restore = from_saved(
                restored
                    .and_then(|t| t.remote_dir.as_deref())
                    .map(|d| RemotePath::new(d).ok().as_ref() == Some(&files.remote.path)),
            );
        }
        (tab, effects)
    }

    /// A new tab for `site` (a files tab at these folders), and the local listing a files
    /// tab runs. A terminal tab starts without a shell: it opens one once connected.
    fn new_tab_at(
        &mut self,
        site: SiteId,
        kind: TabKind,
        local: RemotePath,
        remote: RemotePath,
    ) -> (Tab, Vec<Effect>) {
        let id = TabId(self.next_id());
        if kind == TabKind::Terminal {
            let body = TabBody::Terminal(Box::new(self.new_terminal()));
            return (Tab { id, site, body }, Vec::new());
        }
        let request_id = RequestId(self.next_id());
        let mut local_pane = Pane::new(local.clone());
        local_pane.listing = Listing::Loading;
        local_pane.request = Some(request_id);
        let tab = Tab {
            id,
            site,
            body: TabBody::Files(FilePanes {
                local: local_pane,
                remote: Pane::new(remote),
                focus: Side::Local,
            }),
        };
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
                    let conn = self.conn_mut(&site);
                    if matches!(conn.state, TabState::Failed(_)) {
                        conn.state = TabState::Connecting;
                        connect.push(site);
                    }
                    index
                }
                None => {
                    // A new connection: whatever the site had is gone with its tabs.
                    self.conns.insert(site.clone(), SiteConn::connecting());
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
        if self.tab_of(&site).is_none() {
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
        }
        // A new session's feed tells its state and login from now on. The session already
        // watched keeps what its feed said: the feed only reports changes from here.
        let conn = self.conn_mut(&site);
        let same = matches!(&result, Ok(c) if conn.session() == Some(c.session));
        conn.known_hosts = KnownHostsFix::Idle;
        if !same {
            conn.forget_feed();
        }
        match result {
            Ok(connected) => {
                self.queue.site_restored(&site);
                let mut effects = self.settle_saving(connected.session, true);
                self.conn_mut(&site).state = TabState::Connected(connected);
                // The remote panes list their folders now that they can, the terminals
                // open their shells.
                for index in self.tabs_of(&site) {
                    effects.extend(self.request_listing(index, Side::Remote));
                }
                // A new session: shells of the old one that still look open are dead.
                effects.extend(self.site_shells_after_connect(&site, !same));
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
                self.conn_mut(&site).state = TabState::Failed(failure);
                effects
            }
        }
    }

    /// "New terminal tab" / "New files tab" on tab `tab`: a `kind` tab of the same site,
    /// right after it, on the session already connected (no connect). A new files tab
    /// starts at the folders of the files tab it was opened from (else the site's) and
    /// lists both its sides at once.
    pub(super) fn new_session_tab(&mut self, tab: TabId, kind: TabKind) -> Vec<Effect> {
        let Some(index) = self.tab_index(tab) else {
            return Vec::new();
        };
        if !self.can_open_session_tab(index) {
            return Vec::new();
        }
        let source = &self.tabs[index];
        let site = source.site.clone();
        let folders = source
            .files()
            .map(|files| (files.local.path.clone(), files.remote.path.clone()));
        let (new, mut effects) = match folders {
            Some((local, remote)) if kind == TabKind::Files => {
                self.new_tab_at(site, kind, local, remote)
            }
            _ => self.new_tab(site, kind, None),
        };
        let at = index + 1;
        self.tabs.insert(at, new);
        self.active_tab = Some(at);
        self.form = None;
        effects.extend(self.request_listing(at, Side::Remote));
        if kind == TabKind::Terminal {
            effects.extend(self.open_terminal_shell(at));
        }
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
        // A terminal's shell goes with its tab (a shell still opening is closed when its
        // result finds no tab).
        let mut effects = Self::terminal_close_effects(&tab);
        if self.tab_of(&tab.site).is_some() {
            return effects;
        }
        // The last tab: the site's connection goes with it, and what waited for it.
        self.held.remove(&tab.site);
        let conn = self
            .conns
            .remove(&tab.site)
            .unwrap_or(SiteConn::connecting());
        let session = match conn.state {
            TabState::Connected(connected) => Some(connected.session),
            TabState::Failed(failure) => failure.session,
            // The connect's session isn't known yet: its dialogs say which it is.
            TabState::Connecting => {
                effects.extend(self.abandon_site_prompts(&tab.site));
                return effects;
            }
        };
        if let Some(session) = session {
            self.forget_session(session);
        }
        effects.extend(session.map(Effect::CloseSession));
        effects
    }

    /// The site's host or port changed: its session no longer matches the profile. A
    /// connected one is closed (once) and every tab of the site shows why.
    pub(super) fn drop_moved_session(&mut self, site: &SiteId) -> Option<Effect> {
        self.tab_of(site)?;
        match &self.conn(site).state {
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
                let conn = self.conn_mut(site);
                conn.state = TabState::Failed(failure);
                conn.forget_feed();
                self.forget_session(session);
                Some(Effect::CloseSession(session))
            }
            TabState::Failed(_) => self.drop_stale_session(site),
            TabState::Connecting => None,
        }
    }

    /// The site's settings changed: if failed, it forgets its session (closed here), so
    /// the next connect builds a new one from the saved site. A connected session keeps
    /// running; the new settings apply to its next connection.
    pub(super) fn drop_stale_session(&mut self, site: &SiteId) -> Option<Effect> {
        self.tab_of(site)?;
        let TabState::Failed(failure) = &mut self.conns.get_mut(site)?.state else {
            return None;
        };
        let session = failure.session.take()?;
        self.forget_session(session);
        Some(Effect::CloseSession(session))
    }
}
