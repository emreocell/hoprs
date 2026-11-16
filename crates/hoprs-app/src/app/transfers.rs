//! Transfers from the panes and the queue panel's messages: the selection sent to the
//! other pane's folder (Upload / Download, Ctrl+Enter, a drag from one pane to the other),
//! files dropped from the system file manager, and the engine's events and snapshots.

use std::path::Path;

use hoprs_core::{Error, ErrorKind, RemotePath, SiteId};
use hoprs_transfer::{Direction, Event, EventKind, TransferId, TransferSpec, TransferState};

use hoprs_core::ConnectionState;
use iced::Point;

use super::files::{FileKey, RequestId, local_path};
use super::menu::{ContextMenu, MenuTarget};
use super::queue::QueueMsg;
use super::{App, DRAG_THRESHOLD, Effect, Message};
use crate::app::TabId;
use crate::backend::{ControlOp, Side, TransferSnapshot};
use crate::ui::files::display_path;
use crate::ui::strings;

/// How long finished transfers wait before their folders are listed again: the endings
/// of a burst (a folder, a multi-file drop) share one listing per pane.
pub const RELIST_DELAY: std::time::Duration = std::time::Duration::from_millis(500);

/// The other pane.
pub fn other(side: Side) -> Side {
    match side {
        Side::Local => Side::Remote,
        Side::Remote => Side::Local,
    }
}

impl App {
    /// The selection of `from` as transfers into the other pane's folder: an upload from
    /// the local pane, a download from the remote one. Folders go as they are (the engine
    /// walks them). Empty unless both panes can act (the tab is connected and listed).
    pub(super) fn transfer_specs(&self, index: usize, from: Side) -> Vec<TransferSpec> {
        let tab = &self.tabs[index];
        if !tab.can_transfer(self.tab_conn(index), from) {
            return Vec::new();
        }
        let Some(source) = tab.pane(from) else {
            return Vec::new();
        };
        let Some(target) = tab.pane(other(from)).map(|pane| &pane.path) else {
            return Vec::new();
        };
        source
            .selected_entries()
            .into_iter()
            .map(|entry| {
                let here = source.path.join(&entry.name);
                let there = target.join(&entry.name);
                let (direction, local, remote) = match from {
                    Side::Local => (Direction::Upload, here, there),
                    Side::Remote => (Direction::Download, there, here),
                };
                TransferSpec {
                    site: tab.site.clone(),
                    direction,
                    local,
                    remote,
                }
            })
            .collect()
    }

    /// Upload / Download / Ctrl+Enter / a drop on the other pane.
    pub(super) fn transfer_selection(&mut self, index: usize, from: Side) -> Vec<Effect> {
        let specs = self.transfer_specs(index, from);
        if specs.is_empty() {
            return Vec::new();
        }
        vec![Effect::Enqueue(specs)]
    }

    /// The pointer entered a pane (`inside`) or left it.
    pub(super) fn pointer(&mut self, tab: TabId, side: Side, inside: bool) {
        if inside {
            self.pointer = Some((tab, side));
        } else if self.pointer == Some((tab, side)) {
            self.pointer = None;
        }
    }

    /// The pointer moved to `at`: a press on a row becomes a drag once the pointer is more
    /// than [`DRAG_THRESHOLD`] from the press (and stays one until the button is up).
    pub(super) fn drag_moved(&mut self, at: Point) {
        if let Some(drag) = &mut self.drag {
            drag.at = at;
            drag.active |= drag.origin.distance(at) > DRAG_THRESHOLD;
        }
    }

    /// Escape while a drag is under way cancels it (`None`: taken). Anything else, a press
    /// that has not moved yet, or Escape while a menu is open (a right press during the
    /// drag opened it: that Escape closes the menu, the next one cancels the drag), passes
    /// unchanged.
    pub(super) fn drag_key(&mut self, msg: Message) -> Option<Message> {
        if self.menu.is_none()
            && !self.dialogs.is_open()
            && self.palette.is_none()
            && matches!(
                msg,
                Message::KeyPressed {
                    key: FileKey::Escape,
                    ..
                }
            )
            && self.drag.is_some_and(|d| d.active)
        {
            self.drag = None;
            return None;
        }
        Some(msg)
    }

    /// Whether pane `side` of tab `index` takes a drop: it can act, and the site's
    /// connection is up (a remote pane on a lost or reconnecting session is no target).
    fn takes_drop(&self, index: usize, side: Side) -> bool {
        self.can_act(index, side) && (side == Side::Local || self.site_up(index))
    }

    /// The pane a drag would drop on now: the pane under the pointer, if a drag that moved
    /// past the threshold and carries something started in the other pane of the same tab
    /// and that pane takes a drop ([`takes_drop`](Self::takes_drop)).
    pub fn drop_target(&self) -> Option<(TabId, Side)> {
        let drag = self.drag.filter(|d| d.active)?;
        if self.drag_count() == 0 {
            return None;
        }
        let (tab, over) = self
            .pointer
            .filter(|&(over_tab, over)| over_tab == drag.tab && over == other(drag.from))?;
        let index = self.tab_index(tab)?;
        self.takes_drop(index, over).then_some((tab, over))
    }

    /// Whether pane `side` of `tab` is under an active drag it would not take (its own
    /// pane, or one that can't act): the not-allowed cursor.
    pub fn drag_invalid_over(&self, tab: TabId, side: Side) -> bool {
        self.drag.is_some_and(|d| d.active && d.tab == tab)
            && self.pointer == Some((tab, side))
            && self.drop_target() != Some((tab, side))
    }

    /// How many entries a drag carries: the selection of its pane.
    pub fn drag_count(&self) -> usize {
        self.drag
            .and_then(|d| self.tab_index(d.tab).zip(Some(d.from)))
            .and_then(|(index, side)| self.tabs[index].pane(side))
            .map_or(0, |pane| pane.selected_count())
    }

    /// What a drop would do, for the target pane: "Upload 2 items to /srv",
    /// "Download 1 item to ~/Downloads" (local folders as the breadcrumb shows them).
    pub fn drop_label(&self) -> Option<String> {
        let (tab, side) = self.drop_target()?;
        let index = self.tab_index(tab)?;
        let pane = self.tabs[index].pane(side)?;
        let count = self.drag_count();
        Some(match side {
            Side::Remote => strings::drop_upload(count, &display_path(&pane.path, None)),
            Side::Local => {
                strings::drop_download(count, &display_path(&pane.path, self.home.as_ref()))
            }
        })
    }

    /// What dropping the system's files would do, while they hover the window and the
    /// remote pane of the tab in view takes a drop: "Upload 3 files to /srv" ("Upload
    /// files to /srv" until the system says how many).
    pub fn os_drop_label(&self) -> Option<String> {
        if !self.os_drag {
            return None;
        }
        let index = self.shown_tab().and_then(|id| self.tab_index(id))?;
        if !self.tabs[index].is_files() || !self.takes_drop(index, Side::Remote) {
            return None;
        }
        let folder = display_path(&self.tabs[index].pane(Side::Remote)?.path, None);
        Some(match self.os_drag_count {
            0 => strings::drop_upload_files_unknown(&folder),
            n => strings::drop_upload_files(n, &folder),
        })
    }

    /// Files from the system entered the window (once per file) or left it.
    pub(super) fn files_hovered(&mut self, hovered: bool) {
        self.os_drag = hovered;
        self.os_drag_count = if hovered { self.os_drag_count + 1 } else { 0 };
    }

    /// The pointer left the window: a drag started in it ends without dropping (its
    /// release may never reach the window); a divider's resize ends with the height it
    /// reached, saved.
    pub(super) fn drag_cancel(&mut self) -> Vec<Effect> {
        self.drag = None;
        self.pointer = None;
        self.end_resize()
    }

    /// A mouse button went up anywhere: a drag that ends over a pane taking it
    /// ([`drop_target`](Self::drop_target)) sends its selection there; any other ends with
    /// nothing. A divider's resize ends and is saved.
    pub(super) fn drag_end(&mut self) -> Vec<Effect> {
        let mut effects = self.end_resize();
        let target = self.drop_target();
        let Some(drag) = self.drag.take() else {
            return effects;
        };
        if let (Some(_), Some(index)) = (target, self.tab_index(drag.tab)) {
            effects.extend(self.transfer_selection(index, drag.from));
        }
        effects
    }

    /// A file or folder dropped from the system file manager: uploaded into the remote
    /// folder of the tab in view. Without a connected tab in view, the queue says why.
    pub(super) fn file_dropped(&mut self, path: &Path) -> Vec<Effect> {
        self.os_drag = false;
        self.os_drag_count = 0;
        // No connected tab in view, or one whose remote folder is still loading (or
        // failed to list, or runs a file operation): each says so.
        let connected = self
            .shown_tab()
            .and_then(|id| self.tab_index(id))
            .filter(|&i| self.tabs[i].is_files() && self.tab_session(i).is_some());
        let Some(index) = connected else {
            self.queue_error(Error::coded(
                ErrorKind::NotFound,
                "app-drop-needs-site",
                "Open a connected site to drop files into it",
            ));
            return Vec::new();
        };
        if !self.can_act(index, Side::Remote) {
            self.queue_error(Error::coded(
                ErrorKind::Conflict,
                "app-drop-pane-busy",
                "The remote folder is busy or not listed yet; drop again in a moment",
            ));
            return Vec::new();
        }
        // The session is lost or reconnecting: the pane showed it takes no drop.
        if !self.takes_drop(index, Side::Remote) {
            self.queue_error(Error::coded(
                ErrorKind::Network,
                "app-drop-not-connected",
                "The site is not connected; reconnect it to drop files into it",
            ));
            return Vec::new();
        }
        let local = local_path(path);
        let name = local
            .as_ref()
            .and_then(RemotePath::file_name)
            .map(str::to_owned);
        let (Some(local), Some(name)) = (local, name) else {
            self.queue_error(
                Error::coded(
                    ErrorKind::Io,
                    "app-drop-unsupported",
                    "Can't upload this path",
                )
                .with_detail(path.display().to_string()),
            );
            return Vec::new();
        };
        let tab = &self.tabs[index];
        let Some(remote) = tab.pane(Side::Remote).map(|pane| pane.path.join(&name)) else {
            return Vec::new();
        };
        vec![Effect::Enqueue(vec![TransferSpec {
            site: tab.site.clone(),
            direction: Direction::Upload,
            local,
            remote,
        }])]
    }

    pub(super) fn queue_msg(&mut self, msg: QueueMsg) -> Vec<Effect> {
        match msg {
            QueueMsg::SelectTab(tab) => {
                if self.queue.tab != tab {
                    self.queue.tab = tab;
                    self.queue.offset_y = 0.0;
                }
            }
            QueueMsg::Action(id, action) => {
                if let Some(op) = self.queue.control(id, action) {
                    return vec![Effect::Control { id, op }];
                }
            }
            QueueMsg::ToggleApplyAll(id) => self.queue.toggle_apply_all(id),
            QueueMsg::ClearFinished => return vec![Effect::ClearFinished],
            QueueMsg::DismissError => self.queue.error = None,
            QueueMsg::Scrolled { offset_y } => self.queue.offset_y = offset_y,
        }
        Vec::new()
    }

    /// The site was deleted: its unfinished transfers can never run again, so they are
    /// cancelled through the usual control path (each result comes back as
    /// [`Message::ControlDone`](super::Message::ControlDone)).
    pub(super) fn cancel_site_transfers(&self, site: &SiteId) -> Vec<Effect> {
        self.queue
            .unfinished_of(site)
            .into_iter()
            .map(|id| Effect::Control {
                id,
                op: ControlOp::Cancel,
            })
            .collect()
    }

    /// Asks for a new snapshot; events wait for it (see [`super::queue`]).
    pub(super) fn resync_queue(&mut self) -> Vec<Effect> {
        let request = RequestId(self.next_id());
        self.queue.begin_resync(request);
        vec![Effect::SnapshotQueue(request)]
    }

    pub(super) fn transfer_event(&mut self, event: Event) -> Vec<Effect> {
        let mut effects = self.compare_transfer_event(&event);
        let done = match &event.kind {
            EventKind::StateChanged {
                id,
                state: TransferState::Done,
            } => Some(*id),
            _ => None,
        };
        if self.queue.on_event(event) {
            effects.extend(self.resync_queue());
        }
        // Looked up after the event: an item whose row the queue does not hold yet (its
        // events wait for a snapshot) is found by the snapshot's comparison instead.
        if let Some(id) = done {
            effects.extend(self.refresh_after_transfer(id));
        }
        effects
    }

    pub(super) fn queue_snapshot(
        &mut self,
        request: RequestId,
        snapshot: TransferSnapshot,
    ) -> Vec<Effect> {
        let mut effects = self.compare_snapshot(&snapshot.items);
        // Endings the lost events never told (the first snapshot is the state at start).
        let before = self.queue.has_snapshot().then(|| self.queue.done_ids());
        if self.queue.on_snapshot(request, snapshot) {
            effects.extend(self.resync_queue());
        }
        if let Some(before) = before {
            let now: Vec<TransferId> = self
                .queue
                .done_ids()
                .into_iter()
                .filter(|id| !before.contains(id))
                .collect();
            for id in now {
                effects.extend(self.refresh_after_transfer(id));
            }
        }
        effects
    }

    /// Whether the site of the tab at `index` is up: it has a session and the session's
    /// feed does not say it is lost or reconnecting.
    fn site_up(&self, index: usize) -> bool {
        let conn = self.tab_conn(index);
        conn.session().is_some()
            && conn
                .conn
                .as_ref()
                .is_none_or(|state| *state == ConnectionState::Connected)
    }

    /// Transfer `id` is done: every files pane showing its destination folder, on its side
    /// (and, remote, of its site) and with the site up, is marked for a quiet re-list,
    /// which runs when the [`RELIST_DELAY`] runs out (one timer for a burst).
    pub(super) fn refresh_after_transfer(&mut self, id: TransferId) -> Vec<Effect> {
        let Some(spec) = self.queue.row(id).map(|row| row.item.spec.clone()) else {
            return Vec::new();
        };
        let (side, destination) = match spec.direction {
            Direction::Upload => (Side::Remote, spec.remote),
            Direction::Download => (Side::Local, spec.local),
        };
        let Some(folder) = destination.parent() else {
            return Vec::new();
        };
        let mut marked = false;
        for index in 0..self.tabs.len() {
            if side == Side::Remote && (self.tabs[index].site != spec.site || !self.site_up(index))
            {
                continue;
            }
            if let Some(pane) = self.tabs[index].pane_mut(side)
                && pane.path == folder
            {
                pane.relist = true;
                marked = true;
            }
        }
        if !marked || self.relist_armed {
            return Vec::new();
        }
        self.relist_armed = true;
        vec![Effect::Later {
            after: RELIST_DELAY,
            message: Box::new(Message::RelistPanes),
        }]
    }

    /// The re-list delay ran out: each marked pane lists its folder again, quietly
    /// ([`list_pane_soft`](Self::list_pane_soft)). A pane that is busy (a listing out, a
    /// file operation, an editor, confirmation or dialog open, its menu open, a drag from
    /// it) keeps its mark and the delay starts again, so a pane lists at most once per
    /// delay and never while it lists already; one whose site went down drops it (the
    /// reconnection lists it).
    pub(super) fn relist_panes(&mut self) -> Vec<Effect> {
        self.relist_armed = false;
        let mut effects = Vec::new();
        let mut waiting = false;
        for index in 0..self.tabs.len() {
            for side in [Side::Local, Side::Remote] {
                let tab = &self.tabs[index];
                let Some(pane) = tab.pane(side).filter(|pane| pane.relist) else {
                    continue;
                };
                if side == Side::Remote && !self.site_up(index) {
                    if let Some(pane) = self.tabs[index].pane_mut(side) {
                        pane.relist = false;
                    }
                    continue;
                }
                let menu_open = matches!(
                    &self.menu,
                    Some(ContextMenu { target: MenuTarget::Pane { tab: t, side: s, .. }, .. })
                        if *t == tab.id && *s == side
                );
                let dragged = self
                    .drag
                    .is_some_and(|d| d.active && d.tab == tab.id && d.from == side);
                if pane.request.is_some()
                    || pane.busy.is_some()
                    || pane.action.is_some()
                    || menu_open
                    || dragged
                {
                    waiting = true;
                    continue;
                }
                effects.extend(self.list_pane_soft(index, side));
            }
        }
        // The tick keeps re-arming while a marked pane stays busy (an editor open for
        // minutes, say): it does no I/O, only looks at the marks, and stops once no mark
        // is left (listed, the folder left, or the site down).
        if waiting {
            self.relist_armed = true;
            effects.push(Effect::Later {
                after: RELIST_DELAY,
                message: Box::new(Message::RelistPanes),
            });
        }
        effects
    }

    pub(super) fn control_done(&mut self, id: TransferId, result: Result<(), Error>) {
        self.queue.control_done(id, result);
    }

    /// "Clear finished" emits no engine event: a new snapshot shows the rows gone.
    pub(super) fn cleared(&mut self, result: Result<usize, Error>) -> Vec<Effect> {
        match result {
            Ok(_) => self.resync_queue(),
            Err(error) => {
                self.queue_error(error);
                Vec::new()
            }
        }
    }

    /// Shows `error` above the queue's rows, opening the queue so it is seen.
    pub(super) fn queue_error(&mut self, error: Error) {
        self.queue.error = Some(error);
        self.queue_open = true;
    }

    /// The queue database could not be opened at start: the engine keeps this run's
    /// transfers in memory. Said once, in the queue's error bar.
    pub(super) fn queue_unavailable(&mut self, cause: Error) {
        let message = crate::ui::strings::queue_unavailable(&crate::ui::strings::error(&cause));
        let mut error = Error::new(cause.kind, message);
        error.detail = cause.detail;
        self.queue_error(error);
    }
}
