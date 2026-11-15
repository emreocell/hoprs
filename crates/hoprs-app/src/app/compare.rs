//! Folder compare: the per-tab state machine and its triggers.
//!
//! A files tab holds a [`CompareState`]. A run is started by the toolbar button or the
//! palette ([`CompareMsg::Start`]), or automatically (setting `compare_auto`) once both panes
//! are listed, 500 ms ([`DEBOUNCE`]) after the last listing, so fast navigation starts only
//! the last comparison. Every run has a [`CompareRunId`]; progress and results of any run but
//! the tab's current one (a cancelled run's late messages) are dropped.
//!
//! Cancelling is cooperative ([`Effect::CompareCancel`]: the run's in-flight listing
//! finishes, nothing new starts); only a closed tab or a gone session aborts the task
//! ([`Effect::CompareAbort`]). A cancel is only ever emitted for a run whose
//! [`Effect::Compare`] this state already emitted, as they are made together.

use std::collections::HashMap;
use std::time::Duration;

use hoprs_core::{Error, RemotePath, Result};
use hoprs_remote::{CompareOptions, Difference, Progress, RelPath, Status, Summary};
use hoprs_transfer::{Direction, Event, EventKind, TransferId, TransferSpec, TransferState};

use super::files::Listing;
use super::settings::SettingsMsg;
use super::{App, Effect, FileKey, Message, TabId};
use crate::backend::Side;

/// How long the panes must stay unchanged before an automatic run starts.
pub const DEBOUNCE: Duration = Duration::from_millis(500);

/// Entries per side after which a run pauses ("Continue" lifts the limit).
pub const LIMIT: usize = 10_000;

/// One comparison run of a tab. A tab's runs are told apart by this id, so the late
/// results of a cancelled run can be dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CompareRunId(pub u64);

/// Where a tab's comparison is.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Phase {
    /// No run (yet, or it was cancelled by a folder change).
    #[default]
    Idle,
    /// Waiting out [`DEBOUNCE`].
    Debouncing,
    /// Walking; `seen` entries so far per side `(local, remote)`.
    Running { seen: (usize, usize) },
    /// A side reached [`LIMIT`]; nothing is listing until [`CompareMsg::Continue`].
    Paused { seen: (usize, usize) },
    /// Ended; `partial` when it was stopped early (Stop, auto turned off).
    Done { partial: bool },
    /// Ended by an error (the session dropped, a root that cannot be listed).
    Stopped(Error),
}

/// What a row of the panel is doing.
#[derive(Debug, Clone, PartialEq)]
pub enum RowState {
    Idle,
    Queued(TransferId),
    Done,
    Failed(Error),
    Skipped(String),
}

/// One difference of the panel.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub diff: Difference,
    pub selected: bool,
    pub state: RowState,
}

/// The panel's groups: how a difference reads in the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Group {
    OnlyLocal,
    OnlyRemote,
    /// On both sides but not the same (newer, size, kind).
    Different,
    /// A folder that could not be listed: shown, never selectable.
    Unreadable,
}

impl Group {
    /// The order the panel shows the groups in.
    pub const ORDER: [Group; 4] = [
        Group::OnlyLocal,
        Group::OnlyRemote,
        Group::Different,
        Group::Unreadable,
    ];

    pub fn of(status: &Status) -> Self {
        match status {
            Status::OnlyLocal => Group::OnlyLocal,
            Status::OnlyRemote => Group::OnlyRemote,
            Status::LocalNewer
            | Status::RemoteNewer
            | Status::SizeDiffers
            | Status::KindDiffers => Group::Different,
            Status::Unreadable { .. } => Group::Unreadable,
        }
    }
}

impl Row {
    pub fn group(&self) -> Group {
        Group::of(&self.diff.status)
    }

    /// Whether the checkbox works: not an unreadable folder, and not queued or done.
    pub fn selectable(&self) -> bool {
        self.group() != Group::Unreadable && matches!(self.state, RowState::Idle)
    }

    /// Whether the panel can send this item to the server: it exists locally, is not a
    /// symlink, and is not a file against a folder.
    pub fn can_upload(&self) -> bool {
        self.transferable() && self.diff.local.is_some()
    }

    /// Whether the panel can fetch this item from the server (see [`Row::can_upload`]).
    pub fn can_download(&self) -> bool {
        self.transferable() && self.diff.remote.is_some()
    }

    /// Some transfer exists for the item: not unreadable, not a symlink, not a file
    /// against a folder.
    pub fn transferable(&self) -> bool {
        !matches!(
            self.diff.status,
            Status::Unreadable { .. } | Status::KindDiffers
        ) && self.diff.kind != hoprs_core::EntryKind::Symlink
    }
}

/// The panel's scroll offset that shows `row` (a line of the grouped list), moving the
/// least from the current one.
fn scroll_to_show(state: &CompareState, row: usize) -> f32 {
    use crate::ui::compare::{MIN_BODY_HEIGHT as BODY_HEIGHT, line_of};
    use crate::ui::virtual_list::ROW_HEIGHT;
    let top = line_of(state, row) as f32 * ROW_HEIGHT;
    let offset = state.offset_y;
    if top < offset {
        top
    } else if top + ROW_HEIGHT > offset + BODY_HEIGHT {
        top + ROW_HEIGHT - BODY_HEIGHT
    } else {
        offset
    }
}

/// A transfer the panel queued.
#[derive(Debug, Clone, PartialEq)]
pub struct QueuedItem {
    /// The row's path, relative to `roots`.
    pub path: RelPath,
    /// The roots it was queued under.
    pub roots: (RemotePath, RemotePath),
    /// A file's size at its source (what both sides hold after the transfer); `None` for
    /// a folder.
    pub file_size: Option<u64>,
}

/// One transfer the panel asks the engine for.
#[derive(Debug, Clone)]
pub struct CompareItem {
    pub spec: TransferSpec,
    /// The row's path, relative to the compared roots.
    pub path: RelPath,
    /// Files replace what is there (their own policy); a one-sided folder cannot conflict.
    pub overwrite: bool,
    /// A file's size at its source; `None` for a folder.
    pub size: Option<u64>,
}

/// What queuing one [`CompareItem`] gave.
#[derive(Debug, Clone)]
pub struct Queued {
    pub path: RelPath,
    pub size: Option<u64>,
    pub result: Result<TransferId>,
}

/// A files tab's comparison.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CompareState {
    /// The current run; `None` between runs. Messages of any other run are dropped.
    pub run: Option<CompareRunId>,
    /// The local and remote folders the current (or last) run compares, captured at its
    /// start; row paths are relative to them.
    pub roots: Option<(RemotePath, RemotePath)>,
    pub phase: Phase,
    /// The differences found, in arrival order.
    pub rows: Vec<Row>,
    /// Entries that are the same on both sides (from the run's summary).
    pub identical: u64,
    /// The panel was closed with the cross: hidden until the next run.
    pub hidden: bool,
    /// Files the panel transferred this session: relative path -> (local size, remote size)
    /// after the transfer (servers that cannot set mtimes).
    pub synced: HashMap<RelPath, (u64, u64)>,
    /// Transfers the panel queued that have not finished, and what each is for.
    pub queued: HashMap<TransferId, QueuedItem>,
    /// The roots `synced` belongs to (its paths are relative to them).
    pub synced_roots: Option<(RemotePath, RemotePath)>,
    /// Panel enqueues whose ids have not come back yet.
    pub enqueuing: usize,
    /// Terminal states that arrived while ids were still on their way back.
    pub early: HashMap<TransferId, TransferState>,
    /// Bumped by every start and cancel: a debounce timer only counts if its number is
    /// still the latest.
    pub generation: u64,
    /// Task 7's Open: set this before navigating the panes to an item, and the navigation
    /// keeps the current run, rows and roots and starts no run. One-shot: consumed when the
    /// panes have listed again, when a listing fails with nothing else pending, or by the
    /// next explicit run. Contract: set it only when at least one listing is guaranteed to
    /// follow (a navigation that changes a pane's folder); otherwise do not set it, or it
    /// would swallow the user's next navigation.
    pub suppress_auto: bool,
    /// The listing a file operation's refresh asked for: it neither cancels nor re-arms
    /// anything.
    pub quiet_request: Option<super::RequestId>,
    /// The panel is folded to its head (like the queue).
    pub collapsed: bool,
    /// The keyboard drives the panel (a click in it), not a pane.
    pub focused: bool,
    /// The keyboard's row (index into `rows`).
    pub cursor: Option<usize>,
    /// The rows' scroll offset, as the list last reported it.
    pub offset_y: f32,
}

impl CompareState {
    /// Whether a run is walking or paused (a cancel is due).
    fn active(&self) -> bool {
        matches!(self.phase, Phase::Running { .. } | Phase::Paused { .. })
    }

    /// The row indices in the order the panel shows them: grouped (only here, only on the
    /// server, different, unreadable), each group in arrival order.
    pub fn order(&self) -> Vec<usize> {
        let mut order = Vec::with_capacity(self.rows.len());
        for group in Group::ORDER {
            order.extend(
                self.rows
                    .iter()
                    .enumerate()
                    .filter(|(_, row)| row.group() == group)
                    .map(|(i, _)| i),
            );
        }
        order
    }

    /// Whether the panel is on screen: a run is on or ended and the cross was not pressed.
    /// The wait before an automatic run shows nothing unless the last result is still there.
    pub fn visible(&self) -> bool {
        !self.hidden
            && match self.phase {
                Phase::Idle => false,
                Phase::Debouncing => self.roots.is_some(),
                _ => true,
            }
    }

    /// The selected rows the panel can send to the server / fetch from it.
    pub fn selected_uploads(&self) -> impl Iterator<Item = usize> + '_ {
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.selected && r.selectable() && r.can_upload())
            .map(|(i, _)| i)
    }

    pub fn selected_downloads(&self) -> impl Iterator<Item = usize> + '_ {
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.selected && r.selectable() && r.can_download())
            .map(|(i, _)| i)
    }

    /// Invalidates pending debounces and cancels the current run, if any (the effect to
    /// send, cooperative).
    fn cancel(&mut self, tab: TabId) -> Option<Effect> {
        self.generation += 1;
        self.run
            .take()
            .map(|run| Effect::CompareCancel { tab, run })
    }
}

/// What the panel and a running comparison send.
#[derive(Debug, Clone)]
// `Found` is the common message; boxing it would only add an allocation per difference.
#[allow(clippy::large_enum_variant)]
pub enum CompareMsg {
    /// A difference, a count of entries seen, or the limit being reached.
    Progress {
        tab: TabId,
        run: CompareRunId,
        progress: Progress,
    },
    /// The run ended, always last. A run stopped through [`Effect::CompareCancel`] ends
    /// with `Ok` and [`Summary::stopped`] set (the app ignores it by run id); a closed
    /// session or a root that cannot be listed ends with `Err`.
    Finished {
        tab: TabId,
        run: CompareRunId,
        result: Result<Summary>,
    },
    /// The Compare button or the palette: runs whatever the automatic setting says.
    Start { tab: TabId },
    /// The debounce timer of `generation` fired.
    Debounced { tab: TabId, generation: u64 },
    /// "Continue" after the limit.
    Continue { tab: TabId },
    /// "Stop" while running.
    Stop { tab: TabId },
    /// The cross: hides the panel until the next run.
    Hide { tab: TabId },
    /// The panel's automatic toggle: saves the setting (the same as Settings'); on starts a
    /// run, off cancels the one running and leaves the rows.
    ToggleAuto { tab: TabId, on: bool },
    /// A row's checkbox.
    Select { tab: TabId, index: usize, on: bool },
    /// A group header's select-all.
    SelectGroup { tab: TabId, group: Group, on: bool },
    /// Double-click on a row: the panes go to the item.
    Open { tab: TabId, index: usize },
    /// "Upload selected".
    UploadSelected { tab: TabId },
    /// "Download selected".
    DownloadSelected { tab: TabId },
    /// The ids of what [`Effect::EnqueueCompare`] queued, per row path.
    Enqueued { tab: TabId, results: Vec<Queued> },
    /// The head's Collapse / Expand.
    ToggleCollapsed { tab: TabId },
    /// A press in the panel: the keyboard drives it from now on; `cursor` is the row
    /// pressed, if any.
    Focus { tab: TabId, cursor: Option<usize> },
    /// The rows' list scrolled.
    Scrolled { tab: TabId, offset_y: f32 },
}

impl App {
    /// Routes a comparison message.
    pub(super) fn compare_msg(&mut self, msg: CompareMsg) -> Vec<Effect> {
        match msg {
            CompareMsg::Progress { tab, run, progress } => {
                self.compare_progress(tab, run, progress)
            }
            CompareMsg::Finished { tab, run, result } => self.compare_finished(tab, run, result),
            CompareMsg::Start { tab } => match self.files_tab_index(tab) {
                Some(index) => self.start_compare(index),
                None => Vec::new(),
            },
            CompareMsg::Debounced { tab, generation } => {
                let Some(index) = self.files_tab_index(tab) else {
                    return Vec::new();
                };
                let due = self.tabs[index]
                    .compare()
                    .is_some_and(|c| c.generation == generation && c.phase == Phase::Debouncing);
                if !due {
                    return Vec::new();
                }
                let effects = if self.settings.compare_auto {
                    self.start_compare(index)
                } else {
                    Vec::new()
                };
                // Not started (auto went off, not ready): nothing is waiting any more.
                if let Some(state) = self.tabs[index].compare_mut()
                    && state.phase == Phase::Debouncing
                {
                    state.phase = Phase::Idle;
                }
                effects
            }
            CompareMsg::Continue { tab } => {
                let Some(index) = self.files_tab_index(tab) else {
                    return Vec::new();
                };
                let Some(state) = self.tabs[index].compare_mut() else {
                    return Vec::new();
                };
                match (state.phase.clone(), state.run) {
                    (Phase::Paused { seen }, Some(run)) => {
                        state.phase = Phase::Running { seen };
                        vec![Effect::CompareResume { tab, run }]
                    }
                    _ => Vec::new(),
                }
            }
            CompareMsg::Stop { tab } => {
                let Some(index) = self.files_tab_index(tab) else {
                    return Vec::new();
                };
                self.stop_compare(index, true)
            }
            CompareMsg::Hide { tab } => {
                let Some(index) = self.files_tab_index(tab) else {
                    return Vec::new();
                };
                let effects = self.stop_compare(index, false);
                if let Some(state) = self.tabs[index].compare_mut() {
                    state.rows.clear();
                    state.identical = 0;
                    state.phase = Phase::Idle;
                    state.hidden = true;
                    state.focused = false;
                    state.cursor = None;
                }
                effects
            }
            CompareMsg::ToggleAuto { tab, on } => {
                let Some(index) = self.files_tab_index(tab) else {
                    return Vec::new();
                };
                let mut effects = self.settings_msg(SettingsMsg::CompareAuto(on));
                if on {
                    effects.extend(self.start_compare(index));
                } else {
                    effects.extend(self.stop_compare(index, true));
                }
                effects
            }
            CompareMsg::Select { tab, index, on } => {
                if let Some(state) = self
                    .files_tab_index(tab)
                    .and_then(|i| self.tabs[i].compare_mut())
                    && let Some(row) = state.rows.get_mut(index)
                    && row.selectable()
                    && row.transferable()
                {
                    row.selected = on;
                }
                Vec::new()
            }
            CompareMsg::SelectGroup { tab, group, on } => {
                if let Some(state) = self
                    .files_tab_index(tab)
                    .and_then(|i| self.tabs[i].compare_mut())
                {
                    for row in state
                        .rows
                        .iter_mut()
                        .filter(|r| r.group() == group && r.selectable() && r.transferable())
                    {
                        row.selected = on;
                    }
                }
                Vec::new()
            }
            CompareMsg::Open { tab, index } => match self.files_tab_index(tab) {
                Some(i) => self.compare_open(i, index),
                None => Vec::new(),
            },
            CompareMsg::UploadSelected { tab } => match self.files_tab_index(tab) {
                Some(i) => self.compare_transfer(i, Side::Local),
                None => Vec::new(),
            },
            CompareMsg::DownloadSelected { tab } => match self.files_tab_index(tab) {
                Some(i) => self.compare_transfer(i, Side::Remote),
                None => Vec::new(),
            },
            CompareMsg::Enqueued { tab, results } => self.compare_enqueued(tab, results),
            CompareMsg::ToggleCollapsed { tab } => {
                if let Some(state) = self
                    .files_tab_index(tab)
                    .and_then(|i| self.tabs[i].compare_mut())
                {
                    state.collapsed = !state.collapsed;
                }
                Vec::new()
            }
            CompareMsg::Focus { tab, cursor } => {
                if let Some(state) = self
                    .files_tab_index(tab)
                    .and_then(|i| self.tabs[i].compare_mut())
                {
                    state.focused = true;
                    if cursor.is_some_and(|c| c < state.rows.len()) {
                        state.cursor = cursor;
                    }
                }
                Vec::new()
            }
            CompareMsg::Scrolled { tab, offset_y } => {
                if let Some(state) = self
                    .files_tab_index(tab)
                    .and_then(|i| self.tabs[i].compare_mut())
                {
                    state.offset_y = offset_y;
                }
                Vec::new()
            }
        }
    }

    /// A key while the panel has the keyboard (a click in it): arrows move the row cursor,
    /// Space toggles its checkbox, Enter opens it, Escape and Tab give the keyboard back to
    /// the panes. `None` when the panel does not have the keyboard.
    pub(super) fn compare_key(&mut self, index: usize, key: FileKey) -> Option<Vec<Effect>> {
        let tab = self.tabs[index].id;
        let state = self.tabs[index].compare_mut()?;
        if !state.focused || !state.visible() || state.collapsed {
            return None;
        }
        let order = state.order();
        let at = state
            .cursor
            .and_then(|c| order.iter().position(|&i| i == c));
        let moved = match key {
            FileKey::Up => Some(at.map_or(0, |p| p.saturating_sub(1))),
            FileKey::Down => Some(at.map_or(0, |p| (p + 1).min(order.len().saturating_sub(1)))),
            FileKey::Home => Some(0),
            FileKey::End => Some(order.len().saturating_sub(1)),
            _ => None,
        };
        if let Some(position) = moved {
            if let Some(&row) = order.get(position) {
                state.cursor = Some(row);
                return Some(vec![Effect::ScrollCompare {
                    tab,
                    offset_y: scroll_to_show(state, row),
                }]);
            }
            return Some(Vec::new());
        }
        match key {
            FileKey::Space => {
                if let Some(cursor) = state.cursor
                    && let Some(row) = state.rows.get_mut(cursor)
                    && row.selectable()
                    && row.transferable()
                {
                    row.selected = !row.selected;
                }
                Some(Vec::new())
            }
            FileKey::Enter => Some(match state.cursor {
                Some(row) => self.compare_msg(CompareMsg::Open { tab, index: row }),
                None => Vec::new(),
            }),
            FileKey::Escape | FileKey::SwitchPane => {
                state.focused = false;
                Some(Vec::new())
            }
            _ => Some(Vec::new()),
        }
    }

    /// The index of the files tab `tab`.
    fn files_tab_index(&self, tab: TabId) -> Option<usize> {
        self.tab_index(tab).filter(|&i| self.tabs[i].is_files())
    }

    fn compare_progress(
        &mut self,
        tab: TabId,
        run: CompareRunId,
        progress: Progress,
    ) -> Vec<Effect> {
        let Some(state) = self
            .files_tab_index(tab)
            .and_then(|i| self.tabs[i].compare_mut())
            .filter(|state| state.run == Some(run))
        else {
            return Vec::new();
        };
        match progress {
            Progress::Seen { local, remote } => {
                if matches!(state.phase, Phase::Running { .. }) {
                    state.phase = Phase::Running {
                        seen: (local, remote),
                    };
                }
            }
            Progress::Found(diff) => state.rows.push(Row {
                diff,
                selected: false,
                state: RowState::Idle,
            }),
            Progress::LimitReached { local, remote } => {
                state.phase = Phase::Paused {
                    seen: (local, remote),
                };
            }
        }
        Vec::new()
    }

    fn compare_finished(
        &mut self,
        tab: TabId,
        run: CompareRunId,
        result: Result<Summary>,
    ) -> Vec<Effect> {
        let Some(state) = self
            .files_tab_index(tab)
            .and_then(|i| self.tabs[i].compare_mut())
            .filter(|state| state.run == Some(run))
        else {
            return Vec::new();
        };
        state.run = None;
        match result {
            Ok(summary) => {
                state.identical = summary.identical;
                state.phase = Phase::Done {
                    partial: summary.partial,
                };
            }
            Err(error) => state.phase = Phase::Stopped(error),
        }
        Vec::new()
    }

    /// Starts a run on the tab's two folders, cancelling the current one. Needs the
    /// connected session and both panes listed.
    pub(super) fn start_compare(&mut self, index: usize) -> Vec<Effect> {
        if !self.tabs[index].is_files() || !self.compare_ready(index) {
            return Vec::new();
        }
        let Some(session) = self.tab_session(index) else {
            return Vec::new();
        };
        let run = CompareRunId(self.next_id());
        let ignore = self.settings.compare_ignore.clone();
        let tab = &mut self.tabs[index];
        let id = tab.id;
        let (Some(local), Some(remote)) = (
            tab.pane(Side::Local).map(|p| p.path.clone()),
            tab.pane(Side::Remote).map(|p| p.path.clone()),
        ) else {
            return Vec::new();
        };
        let Some(state) = tab.compare_mut() else {
            return Vec::new();
        };
        let mut effects: Vec<Effect> = state.cancel(id).into_iter().collect();
        state.run = Some(run);
        state.roots = Some((local.clone(), remote.clone()));
        state.phase = Phase::Running { seen: (0, 0) };
        state.rows.clear();
        state.identical = 0;
        state.hidden = false;
        state.focused = false;
        state.cursor = None;
        state.suppress_auto = false;
        if state.synced_roots.as_ref() != Some(&(local.clone(), remote.clone())) {
            // What was sent under other folders says nothing about these.
            state.synced.clear();
            state.synced_roots = None;
        }
        let options = CompareOptions {
            ignore,
            limit: Some(LIMIT),
            known_synced: state.synced.clone(),
            ..CompareOptions::default()
        };
        effects.push(Effect::Compare {
            tab: id,
            run,
            session,
            local_root: local,
            remote_root: remote,
            options,
        });
        effects
    }

    /// Both panes listed and the site connected.
    fn compare_ready(&self, index: usize) -> bool {
        let tab = &self.tabs[index];
        let listed = |side| {
            tab.pane(side)
                .is_some_and(|p| p.listing == Listing::Loaded && p.request.is_none())
        };
        listed(Side::Local)
            && listed(Side::Remote)
            && self.tab_session(index).is_some()
            && matches!(
                self.tab_conn(index).conn,
                None | Some(hoprs_core::ConnectionState::Connected)
            )
    }

    /// Cancels what is running or waiting. `keep` leaves the rows found so far (Stop, auto
    /// off) as a partial result; otherwise a cancelled walk's rows go (a folder change).
    fn stop_compare(&mut self, index: usize, keep: bool) -> Vec<Effect> {
        let tab = &mut self.tabs[index];
        let id = tab.id;
        let Some(state) = tab.compare_mut() else {
            return Vec::new();
        };
        let was_active = state.active();
        let effect = state.cancel(id);
        if was_active {
            if keep {
                state.phase = Phase::Done { partial: true };
            } else {
                state.rows.clear();
                state.identical = 0;
                state.phase = Phase::Idle;
            }
        } else if state.phase == Phase::Debouncing {
            state.phase = Phase::Idle;
        }
        effect.into_iter().collect()
    }

    /// A pane of the tab is about to list again (a new folder, a refresh): the run on the old
    /// folders is cancelled and pending debounces are dropped.
    pub(super) fn compare_folders_changing(&mut self, index: usize) -> Vec<Effect> {
        if self.tabs[index].compare().is_some_and(|s| s.suppress_auto) {
            return Vec::new();
        }
        let effects = self.stop_compare(index, false);
        // Rows of other folders than the panes' now are stale (a refresh keeps them).
        let now = (
            self.tabs[index].pane(Side::Local).map(|p| p.path.clone()),
            self.tabs[index].pane(Side::Remote).map(|p| p.path.clone()),
        );
        if let Some(state) = self.tabs[index].compare_mut()
            && let Some((local, remote)) = &state.roots
            && (Some(local), Some(remote)) != (now.0.as_ref(), now.1.as_ref())
        {
            state.roots = None;
            state.rows.clear();
            state.identical = 0;
            state.phase = Phase::Idle;
        }
        effects
    }

    /// Lists a pane again after a file operation (rename, new folder, delete, permissions):
    /// the run and the rows stay, and no automatic run follows.
    pub(super) fn request_listing_quiet(&mut self, index: usize, side: Side) -> Vec<Effect> {
        if !self.tabs[index].is_files() {
            return Vec::new();
        }
        let effects = self.list_pane(index, side);
        let request = self.tabs[index].pane(side).and_then(|p| p.request);
        if let Some(state) = self.tabs[index].compare_mut() {
            state.quiet_request = request;
        }
        effects
    }

    /// A pane listed: with both listed and the automatic setting on, a run follows after the
    /// debounce.
    pub(super) fn compare_after_listing(&mut self, index: usize) -> Vec<Effect> {
        if !self.compare_ready(index) {
            return Vec::new();
        }
        if let Some(state) = self.tabs[index].compare_mut()
            && state.suppress_auto
        {
            state.suppress_auto = false;
            return Vec::new();
        }
        if !self.settings.compare_auto {
            return Vec::new();
        }
        let tab = &mut self.tabs[index];
        let id = tab.id;
        let Some(state) = tab.compare_mut() else {
            return Vec::new();
        };
        state.generation += 1;
        state.phase = Phase::Debouncing;
        vec![Effect::Later {
            after: DEBOUNCE,
            message: Box::new(Message::Compare(CompareMsg::Debounced {
                tab: id,
                generation: state.generation,
            })),
        }]
    }

    /// The tab's session went away: its run is aborted (no `Finished` follows) and the panel
    /// says it stopped.
    pub(super) fn compare_session_gone(&mut self, index: usize, error: &Error) -> Vec<Effect> {
        let tab = &mut self.tabs[index];
        let id = tab.id;
        let Some(state) = tab.compare_mut() else {
            return Vec::new();
        };
        state.generation += 1;
        let Some(run) = state.run.take() else {
            if state.phase == Phase::Debouncing {
                state.phase = Phase::Idle;
            }
            return Vec::new();
        };
        state.phase = Phase::Stopped(error.clone());
        vec![Effect::CompareAbort { tab: id, run }]
    }

    /// The abort a closing tab needs for its run, if any.
    pub(super) fn compare_close_effects(tab: &super::Tab) -> Vec<Effect> {
        tab.compare()
            .and_then(|state| state.run)
            .map(|run| Effect::CompareAbort { tab: tab.id, run })
            .into_iter()
            .collect()
    }
}

/// The parts of a relative path.
fn components(rel: &str) -> impl Iterator<Item = &str> {
    rel.split('/').filter(|part| !part.is_empty())
}

/// `root` with the relative path's components appended.
fn under(root: &RemotePath, rel: &str) -> RemotePath {
    components(rel).fold(root.clone(), |path, part| path.join(part))
}

impl App {
    /// "Upload selected" (`from` local) / "Download selected" (remote): one transfer per
    /// eligible selected row. Files carry `Overwrite` for themselves (the user chose the
    /// row); a folder only one side has uses the normal policy. Checkboxes clear at once;
    /// rows become `Queued` when the ids come back.
    fn compare_transfer(&mut self, index: usize, from: Side) -> Vec<Effect> {
        if !self.can_act(index, Side::Local) || !self.can_act(index, Side::Remote) {
            return Vec::new();
        }
        let tab = &mut self.tabs[index];
        let id = tab.id;
        let site = tab.site.clone();
        let Some(state) = tab.compare_mut() else {
            return Vec::new();
        };
        let Some((local_root, remote_root)) = state.roots.clone() else {
            return Vec::new();
        };
        let rows: Vec<usize> = match from {
            Side::Local => state.selected_uploads().collect(),
            Side::Remote => state.selected_downloads().collect(),
        };
        let mut items = Vec::with_capacity(rows.len());
        for row in rows {
            let row = &mut state.rows[row];
            row.selected = false;
            let is_file = row.diff.kind == hoprs_core::EntryKind::File;
            let source = match from {
                Side::Local => row.diff.local.as_ref(),
                Side::Remote => row.diff.remote.as_ref(),
            };
            let direction = match from {
                Side::Local => Direction::Upload,
                Side::Remote => Direction::Download,
            };
            items.push(CompareItem {
                spec: TransferSpec {
                    site: site.clone(),
                    direction,
                    local: under(&local_root, &row.diff.path),
                    remote: under(&remote_root, &row.diff.path),
                },
                path: row.diff.path.clone(),
                overwrite: is_file,
                size: is_file.then(|| source.map_or(0, |e| e.size)),
            });
        }
        if items.is_empty() {
            return Vec::new();
        }
        state.enqueuing += 1;
        vec![Effect::EnqueueCompare { tab: id, items }]
    }

    /// The ids came back: rows become queued (or failed), and terminal states that beat the
    /// ids here are applied.
    fn compare_enqueued(&mut self, tab: TabId, results: Vec<Queued>) -> Vec<Effect> {
        let Some(index) = self.files_tab_index(tab) else {
            return Vec::new();
        };
        let Some(state) = self.tabs[index].compare_mut() else {
            return Vec::new();
        };
        state.enqueuing = state.enqueuing.saturating_sub(1);
        let Some(roots) = state.roots.clone() else {
            state.early.clear();
            return Vec::new();
        };
        let mut ids = Vec::new();
        for Queued { path, size, result } in results {
            let Some(row) = state
                .rows
                .iter_mut()
                .find(|r| r.diff.path == path && r.state == RowState::Idle)
            else {
                continue;
            };
            match result {
                Ok(id) => {
                    row.state = RowState::Queued(id);
                    state.queued.insert(
                        id,
                        QueuedItem {
                            path,
                            roots: roots.clone(),
                            file_size: size,
                        },
                    );
                    ids.push(id);
                }
                Err(error) => row.state = RowState::Failed(error),
            }
        }
        let early = std::mem::take(&mut state.early);
        let mut effects = Vec::new();
        if state.enqueuing > 0 {
            state.early = early.clone();
        }
        for id in ids {
            if let Some(done) = early.get(&id) {
                effects.extend(self.compare_settle(index, id, done));
            }
        }
        effects
    }

    /// An engine event: a panel transfer that ended moves its row.
    pub(super) fn compare_transfer_event(&mut self, event: &Event) -> Vec<Effect> {
        let EventKind::StateChanged { id, state } = &event.kind else {
            return Vec::new();
        };
        if !state.is_terminal() {
            return Vec::new();
        }
        let mut effects = Vec::new();
        for index in 0..self.tabs.len() {
            let Some(compare) = self.tabs[index].compare_mut() else {
                continue;
            };
            if compare.queued.contains_key(id) {
                effects.extend(self.compare_settle(index, *id, state));
            } else if compare.enqueuing > 0 {
                compare.early.insert(*id, state.clone());
            }
        }
        effects
    }

    /// A snapshot (after a lag or a restart of the feed) may hold endings whose events were
    /// lost.
    pub(super) fn compare_snapshot(
        &mut self,
        items: &[hoprs_transfer::TransferItem],
    ) -> Vec<Effect> {
        let mut effects = Vec::new();
        for index in 0..self.tabs.len() {
            let Some(compare) = self.tabs[index].compare() else {
                continue;
            };
            let ended: Vec<_> = items
                .iter()
                .filter(|i| i.state.is_terminal() && compare.queued.contains_key(&i.id))
                .collect();
            for item in ended {
                effects.extend(self.compare_settle(index, item.id, &item.state));
            }
        }
        effects
    }

    /// Applies the end of panel transfer `id` to its row; when the last transfer waiting
    /// ends and the automatic setting is on, the comparison runs once more.
    fn compare_settle(
        &mut self,
        index: usize,
        id: TransferId,
        ended: &TransferState,
    ) -> Vec<Effect> {
        let Some(state) = self.tabs[index].compare_mut() else {
            return Vec::new();
        };
        let Some(item) = state.queued.remove(&id) else {
            return Vec::new();
        };
        if let Some(row) = state
            .rows
            .iter_mut()
            .find(|r| r.state == RowState::Queued(id))
        {
            row.state = match ended {
                TransferState::Done => RowState::Done,
                TransferState::Failed(error) => RowState::Failed(error.clone()),
                TransferState::Skipped(reason) => RowState::Skipped(reason.clone()),
                // Selectable again.
                _ => RowState::Idle,
            };
        }
        if *ended == TransferState::Done
            && let Some(size) = item.file_size
        {
            if state.synced_roots.as_ref() != Some(&item.roots) {
                state.synced.clear();
                state.synced_roots = Some(item.roots.clone());
            }
            state.synced.insert(item.path.clone(), (size, size));
        }
        // The batch is over when nothing queued is left; one run follows, for these folders.
        let over = state.queued.is_empty() && state.enqueuing == 0;
        let same_folders = state.roots.as_ref() == Some(&item.roots);
        if over && same_folders && self.settings.compare_auto {
            return self.start_compare(index);
        }
        Vec::new()
    }

    /// Double-click / Enter on a row: each pane goes to the item's folder if it exists on
    /// that side, else to the nearest existing ancestor up to the compared root. The panes
    /// listing again does not start a run (the current rows stay).
    fn compare_open(&mut self, index: usize, row: usize) -> Vec<Effect> {
        let tab = &self.tabs[index];
        let Some(state) = tab.compare() else {
            return Vec::new();
        };
        let (Some(row), Some((local_root, remote_root))) = (state.rows.get(row), &state.roots)
        else {
            return Vec::new();
        };
        // A folder only one side has is never walked, so every walked folder (an item's
        // parent) is on both sides unless it lies inside such a one-sided folder.
        let mut parent: Vec<&str> = components(&row.diff.path).collect();
        parent.pop();
        let target =
            |root: &RemotePath, absent: Status| {
                let mut depth = parent.len();
                // Up while an ancestor (or the folder itself) is a folder this side lacks.
                for r in state.rows.iter().filter(|r| {
                    r.diff.status == absent && r.diff.kind != hoprs_core::EntryKind::File
                }) {
                    let parts: Vec<&str> = components(&r.diff.path).collect();
                    if parts.len() <= depth && parent[..parts.len()] == parts[..] {
                        depth = depth.min(parts.len() - 1);
                    }
                }
                parent[..depth]
                    .iter()
                    .fold(root.clone(), |path, part| path.join(part))
            };
        let local = target(local_root, Status::OnlyRemote);
        let remote = target(remote_root, Status::OnlyLocal);
        let moves: Vec<(Side, RemotePath)> = [(Side::Local, local), (Side::Remote, remote)]
            .into_iter()
            .filter(|(side, path)| tab.pane(*side).is_some_and(|p| &p.path != path))
            .collect();
        if moves.is_empty() {
            return Vec::new();
        }
        // At least one pane's folder changes, so a listing follows and consumes the flag.
        if let Some(state) = self.tabs[index].compare_mut() {
            state.suppress_auto = true;
        }
        let mut effects = Vec::new();
        for (side, path) in moves {
            effects.extend(self.navigate(index, side, path));
        }
        effects
    }
}
