//! A file tab's two panes, local and remote: listing, sorting, selection and navigation.
//! Pure state and its update; `ui::files` draws it and `effects` runs the listings.
//!
//! Every listing request carries a [`RequestId`]. A pane remembers only its latest one,
//! and a [`Message::Listed`] with any other id is dropped: the user left that folder (or
//! asked again) while the old listing was in flight. The old listing is never cancelled;
//! it finishes and its result is discarded (Review Focus 1).

use std::collections::{BTreeSet, HashSet};

use hoprs_core::{ConnectionState, EntryKind, Error, RemoteEntry, RemotePath, SavedTab, SiteId};

use super::actions::{Busy, PaneAction};
use super::{App, Effect, TabState};
use crate::backend::{AuthMethod, SessionId, Side};
use crate::ui::virtual_list::ROW_HEIGHT;

/// A tab's identity, stable while tabs open and close around it.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TabId(pub u64);

/// One listing request; only a pane's latest one is applied.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RequestId(pub u64);

/// Where a pane's listing is.
#[derive(Clone, Debug, PartialEq)]
pub enum Listing {
    /// Nothing asked yet: the remote side before its session is connected.
    Idle,
    Loading,
    Loaded,
    Failed(Error),
}

/// The column a pane is sorted by.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SortKey {
    Name,
    Size,
    Modified,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Sort {
    pub key: SortKey,
    pub descending: bool,
}

impl Default for Sort {
    fn default() -> Self {
        Self {
            key: SortKey::Name,
            descending: false,
        }
    }
}

/// The keyboard modifiers held, as the selection reads them (`ctrl` is Cmd on macOS).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub ctrl: bool,
    pub shift: bool,
}

/// Selected rows (row 0 is `..` when the folder has a parent), the anchor a Shift range
/// starts from and the cursor the arrows move.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Selection {
    pub rows: BTreeSet<usize>,
    pub anchor: Option<usize>,
    pub cursor: Option<usize>,
}

impl Selection {
    /// A click on `row`: alone; Ctrl toggles it; Shift selects from the anchor to it
    /// (added to the selection with Ctrl too).
    pub fn click(&mut self, row: usize, mods: Mods) {
        match (mods.shift, mods.ctrl) {
            (true, ctrl) => {
                let anchor = *self.anchor.get_or_insert(row);
                if !ctrl {
                    self.rows.clear();
                }
                self.rows.extend(anchor.min(row)..=anchor.max(row));
            }
            (false, true) => {
                if !self.rows.remove(&row) {
                    self.rows.insert(row);
                }
                self.anchor = Some(row);
            }
            (false, false) => {
                self.rows = BTreeSet::from([row]);
                self.anchor = Some(row);
            }
        }
        self.cursor = Some(row);
    }

    /// The arrows: the cursor moves to `row`, alone or, with Shift, extending the range
    /// from the anchor.
    pub fn move_to(&mut self, row: usize, shift: bool) {
        self.click(row, Mods { ctrl: false, shift });
    }

    pub fn contains(&self, row: usize) -> bool {
        self.rows.contains(&row)
    }
}

/// One row of a pane.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Row<'a> {
    /// `..`, to the parent folder.
    Up,
    Entry(&'a RemoteEntry),
}

/// One side of a file tab.
#[derive(Clone, Debug, PartialEq)]
pub struct Pane {
    pub path: RemotePath,
    pub listing: Listing,
    /// Sorted by `sort`, once per listing or header click.
    pub entries: Vec<RemoteEntry>,
    pub sort: Sort,
    pub selection: Selection,
    /// The row under the mouse.
    pub hovered: Option<usize>,
    /// The list's scroll offset and visible height, as `on_scroll` last reported them.
    pub offset_y: f32,
    pub viewport_h: f32,
    /// The request whose result this pane waits for.
    pub request: Option<RequestId>,
    /// The inline editor, delete confirmation or permissions dialog open, if any.
    pub action: Option<PaneAction>,
    /// The file operation in flight, if any.
    pub busy: Option<Busy>,
    /// The last operation's failure, inline above the table until dismissed.
    pub action_error: Option<Error>,
    /// The entry to select once the folder is listed again (a new or renamed one).
    pub select_after: Option<String>,
}

impl Pane {
    pub fn new(path: RemotePath) -> Self {
        Self {
            path,
            listing: Listing::Idle,
            entries: Vec::new(),
            sort: Sort::default(),
            selection: Selection::default(),
            hovered: None,
            offset_y: 0.0,
            viewport_h: 0.0,
            request: None,
            action: None,
            busy: None,
            action_error: None,
            select_after: None,
        }
    }

    /// Whether row 0 is `..`.
    pub fn has_up(&self) -> bool {
        self.path.parent().is_some()
    }

    pub fn row_count(&self) -> usize {
        usize::from(self.has_up()) + self.entries.len()
    }

    pub fn row(&self, index: usize) -> Option<Row<'_>> {
        match (self.has_up(), index) {
            (true, 0) => Some(Row::Up),
            (true, i) => self.entries.get(i - 1).map(Row::Entry),
            (false, i) => self.entries.get(i).map(Row::Entry),
        }
    }

    /// Re-sorts by `sort`, keeping the same entries selected.
    fn resort(&mut self, sort: Sort) {
        let up = usize::from(self.has_up());
        let name_of = |pane: &Pane, row: Option<usize>| {
            row.and_then(|r| r.checked_sub(up))
                .and_then(|i| pane.entries.get(i))
                .map(|e| e.name.clone())
        };
        let selected: HashSet<String> = self
            .selection
            .rows
            .iter()
            .filter_map(|&r| name_of(self, Some(r)))
            .collect();
        let anchor = name_of(self, self.selection.anchor);
        let cursor = name_of(self, self.selection.cursor);
        let up_selected = up == 1 && self.selection.contains(0);

        self.sort = sort;
        sort_entries(&mut self.entries, sort);

        let mut selection = Selection::default();
        if up_selected {
            selection.rows.insert(0);
        }
        for (i, entry) in self.entries.iter().enumerate() {
            if selected.contains(&entry.name) {
                selection.rows.insert(i + up);
            }
            if anchor.as_deref() == Some(&entry.name) {
                selection.anchor = Some(i + up);
            }
            if cursor.as_deref() == Some(&entry.name) {
                selection.cursor = Some(i + up);
            }
        }
        // `..` keeps its place at the top.
        selection.anchor = selection
            .anchor
            .or(self.selection.anchor.filter(|&r| r < up));
        selection.cursor = selection
            .cursor
            .or(self.selection.cursor.filter(|&r| r < up));
        self.selection = selection;
    }

    /// The offset that brings `row` into view, if it is not.
    pub(super) fn scroll_to_show(&self, row: usize) -> Option<f32> {
        if self.viewport_h <= 0.0 {
            return None;
        }
        let top = row as f32 * ROW_HEIGHT;
        let bottom = top + ROW_HEIGHT;
        if top < self.offset_y {
            Some(top)
        } else if bottom > self.offset_y + self.viewport_h {
            Some(bottom - self.viewport_h)
        } else {
            None
        }
    }
}

/// Folders first, then the rest; within each group by `sort` (names compared without
/// case, then exactly, so the order is total and stable).
pub fn sort_entries(entries: &mut Vec<RemoteEntry>, sort: Sort) {
    let is_dir = |e: &RemoteEntry| e.kind == EntryKind::Dir;
    // Lower-cased once per entry, not once per comparison.
    let mut keyed: Vec<(String, RemoteEntry)> = entries
        .drain(..)
        .map(|e| (e.name.to_lowercase(), e))
        .collect();
    keyed.sort_by(|(ka, a), (kb, b)| {
        let by_name = ka.cmp(kb).then_with(|| a.name.cmp(&b.name));
        let order = match sort.key {
            SortKey::Name => by_name,
            // Folder sizes mean nothing: folders keep their name order.
            SortKey::Size if is_dir(a) && is_dir(b) => by_name,
            SortKey::Size => a.size.cmp(&b.size).then(by_name),
            SortKey::Modified => a.modified.cmp(&b.modified).then(by_name),
        };
        let order = if sort.descending {
            order.reverse()
        } else {
            order
        };
        is_dir(b).cmp(&is_dir(a)).then(order)
    });
    entries.extend(keyed.into_iter().map(|(_, e)| e));
}

/// What a pane shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PaneStatus<'a> {
    /// The skeleton: the listing (or, on the remote side, the connection) is on its way.
    Loading,
    Empty,
    Failed(&'a Error),
    /// The remote side of a tab whose connection failed: "Not connected", Reconnect.
    NotConnected,
    Full,
}

/// What a pane's widgets send.
#[derive(Clone, Debug)]
pub enum PaneMsg {
    /// Focus the pane (a click anywhere in it).
    Focus,
    Click(usize),
    /// Double click on a row: enter the folder.
    Open(usize),
    /// A breadcrumb segment.
    Navigate(RemotePath),
    Sort(SortKey),
    Scrolled {
        offset_y: f32,
        viewport_h: f32,
    },
    Hover(usize),
    Unhover(usize),
    Retry,
    /// The toolbar.
    NewFolder,
    Rename,
    Delete,
    Permissions,
    /// The inline name editor.
    EditChanged(String),
    EditCommit,
    /// Cancel / Keep in the editor, confirmation or dialog.
    CancelAction,
    ConfirmDelete,
    /// A box of the rwx grid (its bit).
    PermissionToggled(u32),
    PermissionOctal(String),
    ApplyPermissions,
    /// "Dismiss" on the error bar.
    DismissError,
    /// Upload (local pane) / Download (remote pane): the selection to the other pane's
    /// folder.
    Transfer,
    /// The pointer entered or left the pane (a drag drops on the pane under it).
    PointerIn,
    PointerOut,
}

/// The keys the focused pane handles.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FileKey {
    Up,
    Down,
    Home,
    End,
    /// Enter the folder under the cursor.
    Enter,
    /// Backspace: up one folder.
    Back,
    /// Tab: focus the other pane.
    SwitchPane,
    /// F7.
    NewFolder,
    /// F2.
    Rename,
    /// Del.
    Delete,
    /// Closes the inline editor, confirmation or dialog (or the error bar).
    Escape,
    /// Ctrl+Enter: the selection to the other pane's folder.
    Transfer,
}

/// One file tab: a site, its connection and its two panes.
#[derive(Debug, Clone, PartialEq)]
pub struct Tab {
    pub id: TabId,
    pub site: SiteId,
    pub state: TabState,
    pub local: Pane,
    pub remote: Pane,
    /// The pane the keyboard drives.
    pub focus: Side,
    /// "Remove the old entry…" after a changed host key.
    pub known_hosts: super::KnownHostsFix,
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
}

/// `path` as a [`RemotePath`], if it is absolute and POSIX-shaped (the local side lists
/// through `LocalFs::new("/")`).
pub fn local_path(path: &std::path::Path) -> Option<RemotePath> {
    RemotePath::new(path.to_str()?).ok()
}

impl App {
    pub fn tab_index(&self, id: TabId) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }

    pub(super) fn next_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    /// A new tab for `site`, and the local listing to run. Its panes start at the folders
    /// of `restored` (a workspace's saved tab), else at the site's (local: `local_dir`,
    /// else home; remote: `remote_dir`, else `/`). A saved folder that is not a usable
    /// absolute path falls back the same way.
    pub(super) fn new_tab(&mut self, site: SiteId, restored: Option<&SavedTab>) -> (Tab, Effect) {
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
        let request_id = RequestId(self.next_id());
        let mut tab = Tab {
            id,
            site,
            state: TabState::Connecting,
            local: Pane::new(local.clone()),
            remote: Pane::new(remote),
            focus: Side::Local,
            known_hosts: super::KnownHostsFix::Idle,
            failed_endpoint: None,
            conn: None,
            auth: None,
            reconnecting: false,
        };
        tab.local.listing = Listing::Loading;
        tab.local.request = Some(request_id);
        let effect = Effect::List {
            tab: id,
            side: Side::Local,
            request_id,
            session: None,
            path: local,
        };
        (tab, effect)
    }

    /// Lists the pane's folder again, superseding any listing in flight. The remote side
    /// waits for its session. The list starts again from the top: the skeleton replaces
    /// it meanwhile, so the scrollable that comes back is at 0 too.
    pub(super) fn request_listing(&mut self, index: usize, side: Side) -> Vec<Effect> {
        let request_id = RequestId(self.next_id());
        let tab = &mut self.tabs[index];
        let (tab_id, session) = (tab.id, tab.session());
        let pane = tab.pane_mut(side);
        pane.offset_y = 0.0;
        let scroll = Effect::ScrollPane {
            tab: tab_id,
            side,
            offset_y: 0.0,
        };
        let session = match side {
            Side::Local => None,
            Side::Remote if session.is_none() => {
                pane.listing = Listing::Idle;
                pane.request = None;
                return vec![scroll];
            }
            Side::Remote => session,
        };
        pane.listing = Listing::Loading;
        pane.request = Some(request_id);
        vec![
            Effect::List {
                tab: tab_id,
                side,
                request_id,
                session,
                path: pane.path.clone(),
            },
            scroll,
        ]
    }

    /// Goes to `path`: the old entries go, with any editor or dialog about them, and the
    /// list scrolls to the top.
    fn navigate(&mut self, index: usize, side: Side, path: RemotePath) -> Vec<Effect> {
        let pane = self.tabs[index].pane_mut(side);
        pane.path = path;
        pane.entries.clear();
        pane.selection = Selection::default();
        pane.hovered = None;
        pane.action = None;
        pane.action_error = None;
        pane.select_after = None;
        self.request_listing(index, side)
    }

    /// Enters the folder of `row` (`..` goes up); files stay where they are.
    fn open_row(&mut self, index: usize, side: Side, row: usize) -> Vec<Effect> {
        let pane = self.tabs[index].pane(side);
        let target = match pane.row(row) {
            Some(Row::Up) => pane.path.parent(),
            // A link may point at a folder: try it; a failure shows in the pane.
            Some(Row::Entry(entry)) if entry.kind != EntryKind::File => {
                Some(pane.path.join(&entry.name))
            }
            Some(Row::Entry(_)) | None => None,
        };
        match target {
            Some(path) => self.navigate(index, side, path),
            None => Vec::new(),
        }
    }

    pub(super) fn pane_msg(&mut self, tab: TabId, side: Side, msg: PaneMsg) -> Vec<Effect> {
        let Some(index) = self.tab_index(tab) else {
            return Vec::new();
        };
        let mods = self.mods;
        let pane = self.tabs[index].pane_mut(side);
        match msg {
            PaneMsg::Focus => {}
            PaneMsg::Click(row) => {
                if row < pane.row_count() {
                    pane.selection.click(row, mods);
                    // A press on a row may start a drag to the other pane.
                    self.drag = Some((tab, side));
                }
            }
            PaneMsg::Open(row) => {
                self.tabs[index].focus = side;
                return self.open_row(index, side, row);
            }
            PaneMsg::Navigate(path) => {
                self.tabs[index].focus = side;
                return self.navigate(index, side, path);
            }
            PaneMsg::Sort(key) => {
                let descending = pane.sort.key == key && !pane.sort.descending;
                pane.resort(Sort { key, descending });
            }
            PaneMsg::Scrolled {
                offset_y,
                viewport_h,
            } => {
                pane.offset_y = offset_y;
                pane.viewport_h = viewport_h;
                return Vec::new();
            }
            PaneMsg::Hover(row) => {
                pane.hovered = Some(row);
                return Vec::new();
            }
            PaneMsg::Unhover(row) => {
                if pane.hovered == Some(row) {
                    pane.hovered = None;
                }
                return Vec::new();
            }
            PaneMsg::Retry => return self.request_listing(index, side),
            PaneMsg::NewFolder => {
                self.tabs[index].focus = side;
                return self.start_new_folder(index, side);
            }
            PaneMsg::Rename => {
                self.tabs[index].focus = side;
                return self.start_rename(index, side);
            }
            PaneMsg::Delete => {
                self.tabs[index].focus = side;
                return self.start_delete(index, side);
            }
            PaneMsg::Permissions => {
                self.tabs[index].focus = side;
                return self.start_permissions(index, side);
            }
            PaneMsg::EditChanged(value) => {
                if let Some(PaneAction::Edit(edit)) = &mut pane.action {
                    edit.value = value;
                    edit.error = None;
                }
                return Vec::new();
            }
            PaneMsg::EditCommit => return self.commit_edit(index, side),
            PaneMsg::CancelAction => self.cancel_action(index, side),
            PaneMsg::ConfirmDelete => return self.confirm_delete(index, side),
            PaneMsg::PermissionToggled(bit) => {
                if let Some(PaneAction::Permissions(dialog)) = &mut pane.action {
                    dialog.toggle(bit);
                }
                return Vec::new();
            }
            PaneMsg::PermissionOctal(text) => {
                if let Some(PaneAction::Permissions(dialog)) = &mut pane.action {
                    dialog.set_octal(text);
                }
                return Vec::new();
            }
            PaneMsg::ApplyPermissions => return self.apply_permissions(index, side),
            PaneMsg::DismissError => pane.action_error = None,
            PaneMsg::Transfer => {
                self.tabs[index].focus = side;
                return self.transfer_selection(index, side);
            }
            PaneMsg::PointerIn | PaneMsg::PointerOut => {
                self.pointer(tab, side, matches!(msg, PaneMsg::PointerIn));
                return Vec::new();
            }
        }
        self.tabs[index].focus = side;
        Vec::new()
    }

    /// A listing finished. Applied only if it is the pane's latest request.
    pub(super) fn listed(
        &mut self,
        tab: TabId,
        side: Side,
        request_id: RequestId,
        result: Result<Vec<RemoteEntry>, Error>,
    ) -> Vec<Effect> {
        let Some(index) = self.tab_index(tab) else {
            return Vec::new();
        };
        let pane = self.tabs[index].pane_mut(side);
        if pane.request != Some(request_id) {
            // Superseded (or never ours): the pane shows another folder now.
            return Vec::new();
        }
        pane.request = None;
        pane.selection = Selection::default();
        pane.hovered = None;
        let listed = result.is_ok();
        match result {
            Ok(mut entries) => {
                // FTP listings may carry the folder's own "." and "..".
                entries.retain(|e| {
                    !matches!(e.name.as_str(), "" | "." | "..") && !e.name.contains('/')
                });
                sort_entries(&mut entries, pane.sort);
                pane.entries = entries;
                pane.listing = Listing::Loaded;
            }
            Err(error) => {
                pane.entries.clear();
                pane.listing = Listing::Failed(error);
                pane.select_after = None;
            }
        }
        if listed {
            self.select_new_entry(index, side)
        } else {
            Vec::new()
        }
    }

    /// A key from the subscription. Escape goes straight to the pane (a text field that
    /// had it already let go of the focus); the others first make sure no text field has
    /// the focus. With the pane's editor or dialog open, only Enter may matter.
    pub(super) fn key_pressed(&mut self, key: FileKey, shift: bool) -> Vec<Effect> {
        if self.dialogs.is_open() {
            return self.dialog_key(key, shift);
        }
        if self.form.is_some() {
            return Vec::new();
        }
        let Some(tab) = self.active_tab.and_then(|i| self.tabs.get(i)) else {
            return Vec::new();
        };
        if key == FileKey::Escape {
            return self.file_key(key, shift);
        }
        let typing = matches!(
            tab.pane(tab.focus).action,
            Some(PaneAction::Edit(_) | PaneAction::Permissions(_))
        );
        if typing && key != FileKey::Enter {
            return Vec::new();
        }
        vec![Effect::KeyUnlessTyping { key, shift }]
    }

    /// A key for the active tab's focused pane (ignored while a form is open).
    pub(super) fn file_key(&mut self, key: FileKey, shift: bool) -> Vec<Effect> {
        if self.form.is_some() || self.dialogs.is_open() {
            return Vec::new();
        }
        let Some(index) = self.active_tab.filter(|&i| i < self.tabs.len()) else {
            return Vec::new();
        };
        let tab = &mut self.tabs[index];
        let side = tab.focus;
        // An editor or dialog is open: only Escape (cancel) and Enter (commit a name) act;
        // every other key belongs to it.
        match (&tab.pane(side).action, key) {
            (Some(_), FileKey::Escape) => {
                self.cancel_action(index, side);
                return Vec::new();
            }
            (Some(PaneAction::Edit(_)), FileKey::Enter) => return self.commit_edit(index, side),
            (Some(_), _) => return Vec::new(),
            (None, FileKey::Escape) => {
                tab.pane_mut(side).action_error = None;
                return Vec::new();
            }
            (None, FileKey::NewFolder) => return self.start_new_folder(index, side),
            (None, FileKey::Rename) => return self.start_rename(index, side),
            (None, FileKey::Delete) => return self.start_delete(index, side),
            (None, FileKey::Transfer) => return self.transfer_selection(index, side),
            (None, _) => {}
        }
        if key == FileKey::SwitchPane {
            tab.focus = match side {
                Side::Local => Side::Remote,
                Side::Remote => Side::Local,
            };
            return Vec::new();
        }
        if !matches!(tab.pane_status(side), PaneStatus::Full | PaneStatus::Empty) {
            return Vec::new();
        }
        let tab_id = tab.id;
        let pane = tab.pane_mut(side);
        let count = pane.row_count();
        if count == 0 {
            return Vec::new();
        }
        let cursor = pane.selection.cursor;
        let target = match key {
            FileKey::Up => cursor.map_or(0, |c| c.saturating_sub(1)),
            FileKey::Down => cursor.map_or(0, |c| (c + 1).min(count - 1)),
            FileKey::Home => 0,
            FileKey::End => count - 1,
            FileKey::Enter => {
                return match cursor {
                    Some(row) => self.open_row(index, side, row),
                    None => Vec::new(),
                };
            }
            FileKey::Back => {
                return match pane.path.parent() {
                    Some(parent) => self.navigate(index, side, parent),
                    None => Vec::new(),
                };
            }
            FileKey::SwitchPane
            | FileKey::NewFolder
            | FileKey::Rename
            | FileKey::Delete
            | FileKey::Transfer
            | FileKey::Escape => unreachable!("handled above"),
        };
        pane.selection.move_to(target, shift);
        match pane.scroll_to_show(target) {
            Some(offset_y) => {
                pane.offset_y = offset_y;
                vec![Effect::ScrollPane {
                    tab: tab_id,
                    side,
                    offset_y,
                }]
            }
            None => Vec::new(),
        }
    }

    /// The scroll offsets of the tab now in the center, so its lists show where their
    /// state says (the scrollable widgets are shared between tabs and forms).
    pub(super) fn sync_scroll(&self) -> Vec<Effect> {
        let Some(tab) = self
            .active_tab
            .and_then(|i| self.tabs.get(i))
            .filter(|_| self.form.is_none())
        else {
            return Vec::new();
        };
        [Side::Local, Side::Remote]
            .into_iter()
            .map(|side| Effect::ScrollPane {
                tab: tab.id,
                side,
                offset_y: tab.pane(side).offset_y,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use super::*;

    fn entry(name: &str, kind: EntryKind, size: u64, modified: u64) -> RemoteEntry {
        RemoteEntry {
            name: name.into(),
            path: RemotePath::root().join(name),
            kind,
            size,
            modified: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(modified)),
            mode: None,
        }
    }

    fn names(entries: &[RemoteEntry]) -> Vec<&str> {
        entries.iter().map(|e| e.name.as_str()).collect()
    }

    #[test]
    fn sort_folders_first() {
        use EntryKind::{Dir, File, Symlink};
        let mut entries = vec![
            entry("b.txt", File, 300, 3),
            entry("Zeta", Dir, 0, 1),
            entry("a.txt", File, 100, 5),
            entry("alpha", Dir, 0, 9),
            entry("B.txt", File, 200, 4),
            entry("link", Symlink, 10, 2),
            entry("Beta", Dir, 0, 7),
        ];
        let by = |key, descending| Sort { key, descending };

        // Name: folders first, names without case (ties broken exactly: "B.txt" < "b.txt").
        sort_entries(&mut entries, Sort::default());
        assert_eq!(
            names(&entries),
            ["alpha", "Beta", "Zeta", "a.txt", "B.txt", "b.txt", "link"]
        );
        // Descending flips each group; folders stay first.
        sort_entries(&mut entries, by(SortKey::Name, true));
        assert_eq!(
            names(&entries),
            ["Zeta", "Beta", "alpha", "link", "b.txt", "B.txt", "a.txt"]
        );
        // Size: files by size; folders (no meaningful size) by name.
        sort_entries(&mut entries, by(SortKey::Size, false));
        assert_eq!(
            names(&entries),
            ["alpha", "Beta", "Zeta", "link", "a.txt", "B.txt", "b.txt"]
        );
        // Modified, newest first.
        sort_entries(&mut entries, by(SortKey::Modified, true));
        assert_eq!(
            names(&entries),
            ["alpha", "Beta", "Zeta", "a.txt", "B.txt", "b.txt", "link"]
        );

        // A header click re-sorts once and keeps the selection on the same entries.
        let mut pane = Pane::new(RemotePath::new("/srv").unwrap());
        pane.entries = entries.clone();
        pane.listing = Listing::Loaded;
        pane.resort(Sort::default());
        // Rows: 0 "..", 1 alpha, 2 Beta, 3 Zeta, 4 a.txt, 5 B.txt, 6 b.txt, 7 link.
        pane.selection.click(4, Mods::default());
        pane.selection.click(
            7,
            Mods {
                ctrl: true,
                shift: false,
            },
        );
        pane.resort(by(SortKey::Name, true));
        // Rows: 0 "..", 1 Zeta, 2 Beta, 3 alpha, 4 link, 5 b.txt, 6 B.txt, 7 a.txt.
        assert_eq!(pane.selection.rows, BTreeSet::from([4, 7]));
        assert_eq!(pane.selection.cursor, Some(4), "the cursor follows link");
        assert_eq!(pane.row(7), Some(Row::Entry(&pane.entries[6])));
        assert_eq!(pane.row(0), Some(Row::Up));
        assert_eq!(pane.row(8), None);
    }

    #[test]
    fn selection_shift_ctrl() {
        let plain = Mods::default();
        let ctrl = Mods {
            ctrl: true,
            shift: false,
        };
        let shift = Mods {
            ctrl: false,
            shift: true,
        };
        let both = Mods {
            ctrl: true,
            shift: true,
        };
        let rows = |s: &Selection| s.rows.iter().copied().collect::<Vec<_>>();
        let mut s = Selection::default();

        // A click selects one row and anchors there.
        s.click(3, plain);
        assert_eq!(rows(&s), [3]);
        assert_eq!((s.anchor, s.cursor), (Some(3), Some(3)));
        // Shift+click: the range from the anchor, either direction.
        s.click(6, shift);
        assert_eq!(rows(&s), [3, 4, 5, 6]);
        s.click(1, shift);
        assert_eq!(
            rows(&s),
            [1, 2, 3],
            "the range is replaced, the anchor kept"
        );
        assert_eq!((s.anchor, s.cursor), (Some(3), Some(1)));
        // Ctrl+click toggles one row and moves the anchor to it.
        s.click(8, ctrl);
        assert_eq!(rows(&s), [1, 2, 3, 8]);
        s.click(2, ctrl);
        assert_eq!(rows(&s), [1, 3, 8]);
        assert_eq!(s.anchor, Some(2));
        // Ctrl+Shift+click adds a range to what is selected.
        s.click(10, both);
        assert_eq!(rows(&s), (1..=10).collect::<Vec<_>>());
        assert_eq!((s.anchor, s.cursor), (Some(2), Some(10)));
        // A plain click starts over.
        s.click(5, plain);
        assert_eq!(rows(&s), [5]);
        // Shift+click with no anchor yet selects just that row.
        let mut fresh = Selection::default();
        fresh.click(4, shift);
        assert_eq!(rows(&fresh), [4]);
        assert_eq!(fresh.anchor, Some(4));

        // Arrows: alone, or extending from the anchor with Shift.
        s.move_to(6, false);
        assert_eq!(rows(&s), [6]);
        assert_eq!((s.anchor, s.cursor), (Some(6), Some(6)));
        s.move_to(7, true);
        s.move_to(8, true);
        assert_eq!(rows(&s), [6, 7, 8]);
        s.move_to(4, true);
        assert_eq!(rows(&s), [4, 5, 6]);
        assert_eq!((s.anchor, s.cursor), (Some(6), Some(4)));
        assert!(s.contains(5) && !s.contains(7));
    }
}
