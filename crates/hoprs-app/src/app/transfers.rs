//! Transfers from the panes and the queue panel's messages: the selection sent to the
//! other pane's folder (Upload / Download, Ctrl+Enter, a drag from one pane to the other),
//! files dropped from the system file manager, and the engine's events and snapshots.

use std::path::Path;

use hoprs_core::{Error, ErrorKind, RemotePath, SiteId};
use hoprs_transfer::{Direction, Event, TransferId, TransferSpec};

use super::files::{RequestId, local_path};
use super::queue::QueueMsg;
use super::{App, Effect};
use crate::app::TabId;
use crate::backend::{ControlOp, Side, TransferSnapshot};
use crate::ui::strings;

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
        let source = tab.pane(from);
        let target = &tab.pane(other(from)).path;
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

    /// The pane a drag would drop on now: the pane under the pointer, if a drag started in
    /// the other pane of the same tab.
    pub fn drop_target(&self) -> Option<(TabId, Side)> {
        let (tab, from) = self.drag?;
        self.pointer
            .filter(|&(over_tab, over)| over_tab == tab && over == other(from))
    }

    /// The pointer left the window: a drag started in it ends without dropping (its
    /// release may never reach the window).
    pub(super) fn drag_cancel(&mut self) {
        self.drag = None;
        self.pointer = None;
    }

    /// A mouse button went up anywhere: a drag that ends over the other pane sends its
    /// selection there; any other ends with nothing.
    pub(super) fn drag_end(&mut self) -> Vec<Effect> {
        let target = self.drop_target();
        let Some((tab, from)) = self.drag.take() else {
            return Vec::new();
        };
        match (target, self.tab_index(tab)) {
            (Some(_), Some(index)) => self.transfer_selection(index, from),
            _ => Vec::new(),
        }
    }

    /// A file or folder dropped from the system file manager: uploaded into the remote
    /// folder of the tab in view. Without a connected tab in view, the queue says why.
    pub(super) fn file_dropped(&mut self, path: &Path) -> Vec<Effect> {
        self.os_drag = false;
        // No connected tab in view, or one whose remote folder is still loading (or
        // failed to list, or runs a file operation): each says so.
        let connected = self
            .shown_tab()
            .and_then(|id| self.tab_index(id))
            .filter(|&i| self.tabs[i].is_files() && self.tab_session(i).is_some());
        let Some(index) = connected else {
            self.queue_error(Error::new(ErrorKind::NotFound, strings::DROP_NEEDS_SITE));
            return Vec::new();
        };
        if !self.can_act(index, Side::Remote) {
            self.queue_error(Error::new(ErrorKind::Conflict, strings::DROP_PANE_BUSY));
            return Vec::new();
        }
        let local = local_path(path);
        let name = local
            .as_ref()
            .and_then(RemotePath::file_name)
            .map(str::to_owned);
        let (Some(local), Some(name)) = (local, name) else {
            self.queue_error(
                Error::new(ErrorKind::Io, strings::DROP_UNSUPPORTED)
                    .with_detail(path.display().to_string()),
            );
            return Vec::new();
        };
        let tab = &self.tabs[index];
        let remote = tab.remote.path.join(&name);
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
        if self.queue.on_event(event) {
            return self.resync_queue();
        }
        Vec::new()
    }

    pub(super) fn queue_snapshot(
        &mut self,
        request: RequestId,
        snapshot: TransferSnapshot,
    ) -> Vec<Effect> {
        if self.queue.on_snapshot(request, snapshot) {
            return self.resync_queue();
        }
        Vec::new()
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
        let message = crate::ui::strings::queue_unavailable(&cause.message);
        let mut error = Error::new(cause.kind, message);
        error.detail = cause.detail;
        self.queue_error(error);
    }
}
