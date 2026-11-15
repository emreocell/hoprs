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
use hoprs_transfer::TransferId;

use super::files::Listing;
use super::settings::SettingsMsg;
use super::{App, Effect, Message, TabId};
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
// The transfer states are set by the panel's actions (Task 7).
#[allow(dead_code)]
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
    /// Transfers the panel queued, and the row's path each is for.
    pub queued: HashMap<TransferId, RelPath>,
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
}

impl CompareState {
    /// Whether a run is walking or paused (a cancel is due).
    fn active(&self) -> bool {
        matches!(self.phase, Phase::Running { .. } | Phase::Paused { .. })
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
    /// Double-click on a row: the panes go to the item (Task 7).
    Open { tab: TabId, index: usize },
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
                        .filter(|r| r.group() == group && r.selectable())
                    {
                        row.selected = on;
                    }
                }
                Vec::new()
            }
            // Task 7: the panes go to the item.
            CompareMsg::Open { .. } => Vec::new(),
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
        state.suppress_auto = false;
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
