//! The terminal tab: a [`TerminalModel`] fed by one shell of the site's SSH session.
//!
//! **Output.** The tab's subscription ([`crate::effects::shell_feed`]) batches the shell's
//! output: at most one [`Message::TerminalOutput`] per frame, with every byte received
//! since the last one. `update` feeds the batch to the model (CPU only, never I/O) and
//! writes the model's replies (DA, DSR, colour queries) back to the shell.
//!
//! **Input.** Keys reach the shell only while the terminal has the focus. The copy, paste
//! and search shortcuts (Ctrl+Shift+C / V / F) and the palette's Ctrl+Shift+K are taken
//! first; every other key goes through [`encode_key`] (Ctrl+K included: "kill line"). One
//! write is in flight per shell at a time, the next ones wait and are sent together, so
//! the bytes always arrive in order.
//!
//! **Size.** The widget turns its size in pixels into columns and rows: the model follows
//! at once, the shell 100 ms after the last change ([`RESIZE_DEBOUNCE`]).
//!
//! **How a shell ends.**
//! - The connection drops: the shell closes with it, the site's banner says "Connection
//!   lost" with "Reconnect". When the connection is back (by itself or by "Reconnect"), the
//!   tab opens a **new** shell and writes "— reconnected —" into the scrollback first.
//! - The shell exits: "Shell exited (code N)" with "Restart shell".
//! - The output came faster than the tab took it, past the pump's limit: "Output was too
//!   fast and the shell was closed" with "Reopen".
//! - The shell could not open (the connection's channel budget, for one): why, and
//!   "Retry".
//!
//! Terminal bytes are never logged.

use std::time::Duration;

use alacritty_terminal::index::Side as GridSide;
use alacritty_terminal::term::TermMode;
use hoprs_core::{ConnectionState, Error, ErrorKind, SiteId};

use super::{App, Effect, Message, RequestId, TabId, TabState};
use crate::backend::{SessionId, ShellId};
use crate::terminal::input::{Key, Modifiers, encode_key, encode_paste};
use crate::terminal::search::{Range, Searcher};
use crate::terminal::selection::SelectionKind;
use crate::terminal::{TermEvent, TerminalModel};
use crate::ui::strings;

/// How long the size must stay put before the shell is told (spec §8).
pub const RESIZE_DEBOUNCE: Duration = Duration::from_millis(100);
/// How long typing in the search field pauses before it searches.
pub const SEARCH_DEBOUNCE: Duration = Duration::from_millis(150);
/// The size a terminal starts with, until its widget measured itself.
pub const INITIAL_GRID: (u16, u16) = (80, 24);
/// Lines a wheel notch scrolls.
pub const WHEEL_LINES: i32 = 3;

/// Where the tab's shell is.
#[derive(Debug, Clone, PartialEq)]
pub enum ShellState {
    /// None yet: the session is not connected.
    Idle,
    /// [`Effect::OpenShell`] runs, under this request.
    Opening(RequestId),
    Open(ShellId),
    Ended(ShellEnd),
}

/// Why the shell is gone.
#[derive(Debug, Clone, PartialEq)]
pub enum ShellEnd {
    /// The shell exited, with the status the server reported (if any).
    Exited(Option<u32>),
    /// The pump gave up on output the tab did not take fast enough.
    Overflowed,
    /// Closed with its connection: a new one opens when the connection is back.
    Lost,
    /// Closed without an exit status while the connection was up.
    Closed,
    /// It could not open.
    Failed(Error),
}

/// The end of a shell's output, as its feed reports it (the data comes as
/// [`Message::TerminalOutput`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellSignal {
    /// The exit status the server reported, if any.
    Exit(Option<u32>),
    /// The pump gave up on output the tab did not take fast enough.
    Overflowed,
    /// The last item.
    Closed,
}

/// The longest remote title kept (characters).
pub const MAX_TITLE_CHARS: usize = 256;
/// The most typed or pasted input that may wait behind a write in flight; past it the
/// rest is dropped and the tab says so.
pub const MAX_PENDING_INPUT: usize = 1024 * 1024;

/// What the bar over an ended shell says, and its button.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndBanner {
    pub message: String,
    pub detail: Option<String>,
    pub action: &'static str,
}

/// The search bar under the tab (Ctrl+Shift+F).
#[derive(Debug, Default)]
pub struct SearchBar {
    /// As typed.
    pub query: String,
    /// The matcher of the query searched last (compiled once per query).
    searcher: Searcher,
    /// The match shown.
    pub current: Option<Range>,
    /// The last search found nothing.
    pub no_match: bool,
    /// Bumped per keystroke: only the latest debounce timer searches.
    seq: u64,
}

/// One terminal tab's own state.
pub struct TerminalTab {
    pub model: TerminalModel,
    pub shell: ShellState,
    /// The remote program's title (OSC 0/2), shown as a hint next to the site's name.
    pub title: Option<String>,
    /// The keyboard goes to the shell.
    pub focused: bool,
    pub search: Option<SearchBar>,
    /// The exit status, until the shell's `Closed`.
    exit: Option<Option<u32>>,
    /// The pump said `Overflowed`, until the shell's `Closed`.
    overflowed: bool,
    /// The size the shell was last told (or opened with).
    shell_size: (u16, u16),
    /// Bumped per size change: only the latest debounce timer resizes the shell.
    resize_seq: u64,
    /// The next shell starts after a reconnect: it writes "— reconnected —" first.
    reconnected: bool,
    /// A write to the shell is in flight.
    writing: bool,
    /// Bytes typed meanwhile, sent together when it ends.
    pending: Vec<u8>,
    /// Input was dropped past [`MAX_PENDING_INPUT`]: shown until the next key.
    pub input_dropped: bool,
}

impl std::fmt::Debug for TerminalTab {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never the content: terminal bytes stay out of logs and panics.
        f.debug_struct("TerminalTab")
            .field("shell", &self.shell)
            .field("columns", &self.model.columns())
            .field("lines", &self.model.screen_lines())
            .field("focused", &self.focused)
            .finish_non_exhaustive()
    }
}

/// What the tab's widget and keyboard ask for.
#[derive(Debug, Clone)]
pub enum TerminalMsg {
    /// The widget's size in cells changed.
    Resized {
        cols: u16,
        rows: u16,
    },
    /// [`RESIZE_DEBOUNCE`] after the size change `seq`.
    ResizeDue(u64),
    /// A key while the widget has the focus.
    Key {
        key: Key,
        mods: Modifiers,
    },
    /// A click inside (`true`) or outside the terminal.
    Focus(bool),
    /// The wheel: lines, positive back into history.
    Scroll(i32),
    /// A press on viewport cell (`row`, `col`): starts a selection.
    SelectStart {
        row: usize,
        col: usize,
        side: GridSide,
        kind: SelectionKind,
    },
    /// The drag moved over viewport cell (`row`, `col`).
    SelectMove {
        row: usize,
        col: usize,
        side: GridSide,
    },
    Copy,
    Paste,
    /// The clipboard's text, for a paste.
    Pasted(Option<String>),
    /// A write to the shell ended (its result does not matter: a dead shell says so
    /// through its output).
    InputSent(ShellId),
    /// "Restart shell", "Reopen" or "Retry": a new shell.
    Restart,
    OpenSearch,
    SearchInput(String),
    /// [`SEARCH_DEBOUNCE`] after the keystroke `seq`.
    SearchDue(u64),
    SearchNext,
    SearchPrevious,
    CloseSearch,
}

impl TerminalTab {
    pub fn new(scrollback: usize, theme: crate::settings::TerminalTheme) -> Self {
        let (cols, rows) = INITIAL_GRID;
        let mut model = TerminalModel::new(cols, rows, scrollback);
        model.set_theme(theme.colors());
        Self {
            model,
            shell: ShellState::Idle,
            title: None,
            focused: true,
            search: None,
            exit: None,
            overflowed: false,
            shell_size: INITIAL_GRID,
            resize_seq: 0,
            reconnected: false,
            writing: false,
            pending: Vec::new(),
            input_dropped: false,
        }
    }

    /// The shell that is open, if any.
    pub fn open_shell(&self) -> Option<ShellId> {
        match self.shell {
            ShellState::Open(id) => Some(id),
            _ => None,
        }
    }

    fn grid(&self) -> (u16, u16) {
        let clamp = |n: usize| u16::try_from(n).unwrap_or(u16::MAX);
        (
            clamp(self.model.columns()),
            clamp(self.model.screen_lines()),
        )
    }

    /// The bar over an ended shell; `None` while it runs (or waits for its connection,
    /// whose banner the site's tabs share).
    pub fn end_banner(&self) -> Option<EndBanner> {
        let ShellState::Ended(end) = &self.shell else {
            return None;
        };
        let (message, detail, action) = match end {
            ShellEnd::Exited(code) => (
                strings::shell_exited(*code),
                None,
                strings::TERMINAL_RESTART,
            ),
            ShellEnd::Overflowed => (
                strings::TERMINAL_OVERFLOWED.to_owned(),
                None,
                strings::TERMINAL_REOPEN,
            ),
            ShellEnd::Closed => (
                strings::TERMINAL_SHELL_CLOSED.to_owned(),
                None,
                strings::TERMINAL_RESTART,
            ),
            ShellEnd::Failed(error) => (
                format!("{}: {}", strings::TERMINAL_OPEN_FAILED, error.message),
                error.detail.clone(),
                strings::TERMINAL_RETRY,
            ),
            ShellEnd::Lost => return None,
        };
        Some(EndBanner {
            message,
            detail,
            action,
        })
    }

    /// Queues `data` for the shell: sent now if nothing is in flight, else with the next
    /// write.
    fn send(&mut self, tab: TabId, data: Vec<u8>) -> Vec<Effect> {
        let Some(shell) = self.open_shell() else {
            return Vec::new();
        };
        if data.is_empty() {
            return Vec::new();
        }
        if self.writing {
            if self.pending.len() + data.len() > MAX_PENDING_INPUT {
                // Never sent in part: a cut escape sequence or paste would be worse.
                self.input_dropped = true;
            } else {
                self.pending.extend_from_slice(&data);
            }
            return Vec::new();
        }
        self.writing = true;
        vec![Effect::ShellInput { tab, shell, data }]
    }

    /// Feeds bytes to the model and answers what it asks.
    fn feed(&mut self, tab: TabId, bytes: &[u8]) -> Vec<Effect> {
        let mut effects = Vec::new();
        for event in self.model.feed(bytes) {
            match event {
                TermEvent::PtyWrite(reply) => effects.extend(self.send(tab, reply)),
                // Empty: back to the default (the site's name alone).
                TermEvent::Title(title) => {
                    self.title =
                        (!title.is_empty()).then(|| title.chars().take(MAX_TITLE_CHARS).collect());
                }
                // No bell sound, and remote programs never fill the clipboard (OSC 52 is
                // off by default).
                TermEvent::Bell | TermEvent::ClipboardStore(_) => {}
            }
        }
        effects
    }

    /// The selected text, for the clipboard.
    fn copy(&self) -> Vec<Effect> {
        let text = self.model.selected_text();
        if text.is_empty() {
            Vec::new()
        } else {
            vec![Effect::WriteClipboard(text)]
        }
    }

    /// Runs the search bar's query from `from` and shows the match.
    fn search_from(&mut self, from: alacritty_terminal::index::Point, backwards: bool) {
        let Some(bar) = self.search.as_mut() else {
            return;
        };
        bar.searcher.set_query(&bar.query);
        bar.current = bar.searcher.find(&self.model, from, backwards);
        bar.no_match = bar.current.is_none() && !bar.query.is_empty();
        if let Some(found) = &bar.current {
            self.model.scroll_to_show(found.start().line);
        }
    }
}

impl App {
    /// A new terminal tab's own state, under the settings in force.
    pub(super) fn new_terminal(&self) -> TerminalTab {
        TerminalTab::new(
            self.settings.terminal_scrollback,
            self.settings.terminal_theme,
        )
    }

    /// The shells whose output the app follows, with their tab: one feed each.
    pub fn terminal_feeds(&self) -> Vec<(TabId, ShellId)> {
        self.tabs
            .iter()
            .filter_map(|tab| Some((tab.id, tab.terminal()?.open_shell()?)))
            .collect()
    }

    /// Opens a shell for the terminal tab at `index`, if its session is up: the model's
    /// size goes with it.
    pub(super) fn open_terminal_shell(&mut self, index: usize) -> Vec<Effect> {
        let Some(session) = self.live_session(&self.tabs[index].site.clone()) else {
            return Vec::new();
        };
        let request = RequestId(self.next_id());
        let tab = &mut self.tabs[index];
        let id = tab.id;
        let Some(terminal) = tab.terminal_mut() else {
            return Vec::new();
        };
        let (cols, rows) = terminal.grid();
        // A shell came before: the new one must not inherit its modes (alternate screen,
        // application keys, bracketed paste, mouse, hidden cursor) or a sequence it left
        // half written. The grid and the history stay.
        if terminal.shell != ShellState::Idle {
            terminal.model.reset_modes();
        }
        terminal.shell = ShellState::Opening(request);
        terminal.exit = None;
        terminal.overflowed = false;
        terminal.writing = false;
        terminal.pending.clear();
        terminal.shell_size = (cols, rows);
        vec![Effect::OpenShell {
            tab: id,
            request,
            session,
            cols,
            rows,
        }]
    }

    /// The site's session, when it is connected and not lost.
    pub(crate) fn live_session(&self, site: &SiteId) -> Option<SessionId> {
        let conn = self.conn(site);
        match conn.conn {
            None | Some(ConnectionState::Connected) => conn.session(),
            Some(_) => None,
        }
    }

    /// The site's session is up (a connect, or the connection back after a loss): its
    /// terminals without a shell open one. With `reconnected`, the shells of the lost
    /// connection are dead: those tabs open a new shell too, which writes "— reconnected
    /// —" first. A shell that exited, overflowed or failed waits for its button.
    pub(super) fn open_site_shells(&mut self, site: &SiteId, reconnected: bool) -> Vec<Effect> {
        let mut effects = Vec::new();
        for index in self.tabs_of(site) {
            let Some(terminal) = self.tabs[index].terminal_mut() else {
                continue;
            };
            let reopen = match &terminal.shell {
                ShellState::Idle => false,
                ShellState::Ended(ShellEnd::Lost | ShellEnd::Closed) => true,
                ShellState::Ended(ShellEnd::Failed(error)) if error.kind == ErrorKind::Network => {
                    true
                }
                ShellState::Open(_) if reconnected => true,
                ShellState::Open(_)
                | ShellState::Opening(_)
                | ShellState::Ended(
                    ShellEnd::Exited(_) | ShellEnd::Overflowed | ShellEnd::Failed(_),
                ) => continue,
            };
            if let ShellState::Open(old) = terminal.shell {
                effects.push(Effect::CloseShell(old));
            }
            terminal.reconnected |= reopen;
            effects.extend(self.open_terminal_shell(index));
        }
        effects
    }

    /// The site's connection is no longer up: a shell that closed meanwhile without an
    /// exit status closed with it.
    pub(super) fn site_connection_down(&mut self, site: &SiteId) {
        for index in self.tabs_of(site) {
            if let Some(terminal) = self.tabs[index].terminal_mut()
                && terminal.shell == ShellState::Ended(ShellEnd::Closed)
            {
                terminal.shell = ShellState::Ended(ShellEnd::Lost);
            }
        }
    }

    /// The tab's terminal, by id.
    fn terminal_at(&mut self, tab: TabId) -> Option<(usize, &mut TerminalTab)> {
        let index = self.tab_index(tab)?;
        Some((index, self.tabs[index].terminal_mut()?))
    }

    /// The shell asked for by `request` opened (or not). A result nobody waits for any
    /// more (the tab closed, or asked again) closes its shell.
    pub(super) fn shell_opened(
        &mut self,
        tab: TabId,
        request: RequestId,
        result: Result<ShellId, Error>,
    ) -> Vec<Effect> {
        let waiting = self
            .terminal_at(tab)
            .filter(|(_, t)| t.shell == ShellState::Opening(request))
            .map(|(_, t)| t);
        let Some(terminal) = waiting else {
            return result.map(Effect::CloseShell).into_iter().collect();
        };
        match result {
            Ok(shell) => {
                terminal.shell = ShellState::Open(shell);
                let mut effects = Vec::new();
                if std::mem::take(&mut terminal.reconnected) {
                    let line = format!("\r\n\x1b[0;2m{}\x1b[0m\r\n", strings::TERMINAL_RECONNECTED);
                    effects.extend(terminal.feed(tab, line.as_bytes()));
                }
                // The widget measured itself while the shell opened.
                let size = terminal.grid();
                if size != terminal.shell_size {
                    terminal.shell_size = size;
                    effects.push(Effect::ShellResize {
                        shell,
                        cols: size.0,
                        rows: size.1,
                    });
                }
                effects
            }
            Err(error) => {
                terminal.shell = ShellState::Ended(ShellEnd::Failed(error));
                Vec::new()
            }
        }
    }

    /// A frame's worth of output of `shell`.
    pub(super) fn terminal_output(
        &mut self,
        tab: TabId,
        shell: ShellId,
        bytes: &[u8],
    ) -> Vec<Effect> {
        match self.terminal_at(tab) {
            Some((_, terminal)) if terminal.open_shell() == Some(shell) => {
                terminal.feed(tab, bytes)
            }
            _ => Vec::new(),
        }
    }

    /// The end of `shell`'s output: its exit status, an overflow, then `Closed`.
    pub(super) fn shell_event(
        &mut self,
        tab: TabId,
        shell: ShellId,
        event: ShellSignal,
    ) -> Vec<Effect> {
        let Some(index) = self.tab_index(tab) else {
            return Vec::new();
        };
        let connected = self.live_session(&self.tabs[index].site).is_some();
        let Some(terminal) = self.tabs[index]
            .terminal_mut()
            .filter(|t| t.open_shell() == Some(shell))
        else {
            return Vec::new();
        };
        match event {
            ShellSignal::Exit(code) => terminal.exit = Some(code),
            ShellSignal::Overflowed => terminal.overflowed = true,
            ShellSignal::Closed => {
                let end = if terminal.overflowed {
                    ShellEnd::Overflowed
                } else if let Some(code) = terminal.exit {
                    ShellEnd::Exited(code)
                } else if connected {
                    ShellEnd::Closed
                } else {
                    ShellEnd::Lost
                };
                terminal.shell = ShellState::Ended(end);
                terminal.writing = false;
                terminal.pending.clear();
                // Forget it in the backend too (it keeps ended shells until told).
                return vec![Effect::CloseShell(shell)];
            }
        }
        Vec::new()
    }

    /// The widget's size in cells: the model follows now, the shell after the debounce.
    pub(super) fn terminal_resized(&mut self, tab: TabId, cols: u16, rows: u16) -> Vec<Effect> {
        let Some((_, terminal)) = self.terminal_at(tab) else {
            return Vec::new();
        };
        if terminal.grid() == (cols, rows) {
            return Vec::new();
        }
        terminal.model.resize(cols, rows);
        terminal.resize_seq += 1;
        vec![Effect::Later {
            after: RESIZE_DEBOUNCE,
            message: Box::new(Message::Terminal {
                tab,
                msg: TerminalMsg::ResizeDue(terminal.resize_seq),
            }),
        }]
    }

    pub(super) fn terminal_msg(&mut self, tab: TabId, msg: TerminalMsg) -> Vec<Effect> {
        if let TerminalMsg::Resized { cols, rows } = msg {
            return self.terminal_resized(tab, cols, rows);
        }
        if let TerminalMsg::Restart = msg {
            return self.restart_terminal(tab);
        }
        let Some((_, terminal)) = self.terminal_at(tab) else {
            return Vec::new();
        };
        match msg {
            TerminalMsg::Resized { .. } | TerminalMsg::Restart => unreachable!("handled above"),
            TerminalMsg::ResizeDue(seq) => {
                let size = terminal.grid();
                match terminal.open_shell() {
                    Some(shell) if seq == terminal.resize_seq && size != terminal.shell_size => {
                        terminal.shell_size = size;
                        return vec![Effect::ShellResize {
                            shell,
                            cols: size.0,
                            rows: size.1,
                        }];
                    }
                    _ => {}
                }
            }
            TerminalMsg::Key { key, mods } => return terminal_key(terminal, tab, &key, mods),
            TerminalMsg::Focus(focused) => terminal.focused = focused,
            TerminalMsg::Scroll(lines) => {
                let mode = terminal.model.mode();
                // Full-screen programs (less, vim) get arrow keys instead, as in xterm.
                if mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL) {
                    let key = if lines > 0 { Key::Up } else { Key::Down };
                    let bytes: Vec<u8> = (0..lines.unsigned_abs())
                        .filter_map(|_| encode_key(&key, Modifiers::default(), mode))
                        .flatten()
                        .collect();
                    return terminal.send(tab, bytes);
                }
                terminal.model.scroll(lines);
            }
            TerminalMsg::SelectStart {
                row,
                col,
                side,
                kind,
            } => {
                terminal.focused = true;
                let point = terminal.model.viewport_to_point(row, col);
                terminal.model.start_selection(kind, point, side);
            }
            TerminalMsg::SelectMove { row, col, side } => {
                let point = terminal.model.viewport_to_point(row, col);
                terminal.model.update_selection(point, side);
            }
            TerminalMsg::Copy => return terminal.copy(),
            TerminalMsg::Paste => {
                if terminal.open_shell().is_some() {
                    return vec![Effect::ReadClipboard(tab)];
                }
            }
            TerminalMsg::Pasted(text) => {
                let bytes = encode_paste(&text.unwrap_or_default(), terminal.model.mode());
                terminal.model.scroll_to_bottom();
                return terminal.send(tab, bytes);
            }
            TerminalMsg::InputSent(shell) => {
                if terminal.open_shell() != Some(shell) {
                    return Vec::new();
                }
                terminal.writing = false;
                let pending = std::mem::take(&mut terminal.pending);
                return terminal.send(tab, pending);
            }
            TerminalMsg::OpenSearch => {
                terminal.search.get_or_insert_with(SearchBar::default);
                terminal.focused = false;
                return vec![Effect::FocusTerminalSearch(tab)];
            }
            TerminalMsg::SearchInput(query) => {
                let Some(bar) = terminal.search.as_mut() else {
                    return Vec::new();
                };
                bar.query = query;
                bar.seq += 1;
                return vec![Effect::Later {
                    after: SEARCH_DEBOUNCE,
                    message: Box::new(Message::Terminal {
                        tab,
                        msg: TerminalMsg::SearchDue(bar.seq),
                    }),
                }];
            }
            TerminalMsg::SearchDue(seq) => {
                if terminal.search.as_ref().is_some_and(|bar| bar.seq == seq) {
                    // The newest match first: backwards from the end of the screen.
                    let (_, bottom) = terminal.model.grid_bounds();
                    terminal.search_from(bottom, true);
                }
            }
            TerminalMsg::SearchNext | TerminalMsg::SearchPrevious => {
                let backwards = matches!(msg, TerminalMsg::SearchPrevious);
                let Some(bar) = terminal.search.as_ref() else {
                    return Vec::new();
                };
                let (top, bottom) = terminal.model.grid_bounds();
                let from = match &bar.current {
                    // One cell past the match's start, in the direction of travel.
                    Some(found) => step(*found.start(), backwards, top, bottom),
                    None if backwards => bottom,
                    None => top,
                };
                terminal.search_from(from, backwards);
            }
            TerminalMsg::CloseSearch => {
                terminal.search = None;
                terminal.focused = true;
            }
        }
        Vec::new()
    }

    /// Whether "Restart shell", "Reopen" or "Retry" can work now (the site's session is
    /// up); the bar shows the button disabled with a hint otherwise.
    pub fn can_restart_terminal(&self, index: usize) -> bool {
        self.tabs
            .get(index)
            .is_some_and(|tab| self.live_session(&tab.site).is_some())
    }

    /// "Restart shell", "Reopen" or "Retry" on an ended shell.
    fn restart_terminal(&mut self, tab: TabId) -> Vec<Effect> {
        match self.terminal_at(tab) {
            Some((index, terminal)) if matches!(terminal.shell, ShellState::Ended(_)) => {
                terminal.focused = true;
                self.open_terminal_shell(index)
            }
            _ => Vec::new(),
        }
    }

    /// Escape in a terminal tab closes its search bar; `true` if it did.
    pub(super) fn terminal_escape(&mut self) -> bool {
        let Some(terminal) = self
            .active_tab
            .and_then(|i| self.tabs.get_mut(i))
            .and_then(|t| t.terminal_mut())
            // A focused terminal takes Escape itself (the shell's); the search field has
            // the keyboard only while the terminal does not.
            .filter(|t| t.search.is_some() && !t.focused)
        else {
            return false;
        };
        terminal.search = None;
        terminal.focused = true;
        true
    }

    /// The shells to close with the terminal tab at `index` (its own, if open).
    pub(super) fn terminal_close_effects(tab: &super::Tab) -> Vec<Effect> {
        tab.terminal()
            .and_then(TerminalTab::open_shell)
            .map(Effect::CloseShell)
            .into_iter()
            .collect()
    }

    /// The site connected (first time, or a new session after a failure or "Close and
    /// reconnect"): its terminals open their shells. With `new_session`, a shell of the
    /// old session that still looks open (its `Closed` not in yet) is replaced too.
    pub(super) fn site_shells_after_connect(
        &mut self,
        site: &SiteId,
        new_session: bool,
    ) -> Vec<Effect> {
        if !matches!(self.conn(site).state, TabState::Connected(_)) {
            return Vec::new();
        }
        self.open_site_shells(site, new_session)
    }
}

/// The point one cell after (`backwards`: before) `point`, wrapping around the grid.
fn step(
    point: alacritty_terminal::index::Point,
    backwards: bool,
    top: alacritty_terminal::index::Point,
    bottom: alacritty_terminal::index::Point,
) -> alacritty_terminal::index::Point {
    use alacritty_terminal::index::{Column, Point};
    let last = bottom.column;
    if backwards {
        if point.column.0 > 0 {
            Point::new(point.line, point.column - 1)
        } else if point.line > top.line {
            Point::new(point.line - 1, last)
        } else {
            bottom
        }
    } else if point.column < last {
        Point::new(point.line, point.column + 1)
    } else if point.line < bottom.line {
        Point::new(point.line + 1, Column(0))
    } else {
        top
    }
}

/// A key for the focused terminal: the tab's shortcuts first, then the shell.
fn terminal_key(terminal: &mut TerminalTab, tab: TabId, key: &Key, mods: Modifiers) -> Vec<Effect> {
    if !terminal.focused {
        return Vec::new();
    }
    terminal.input_dropped = false;
    let letter = match key {
        Key::Char(c) => Some(c.to_ascii_lowercase()),
        _ => None,
    };
    if mods.ctrl && mods.shift && !mods.alt {
        match letter {
            Some('c') => return terminal.copy(),
            Some('v') => return vec![Effect::ReadClipboard(tab)],
            Some('f') => {
                terminal.search.get_or_insert_with(SearchBar::default);
                terminal.focused = false;
                return vec![Effect::FocusTerminalSearch(tab)];
            }
            // The palette (Plan 6 Task 9): never the shell's.
            Some('k') => return Vec::new(),
            _ => {}
        }
    }
    // Shift+PageUp / PageDown page through the scrollback, as in most terminals.
    if mods.shift && !mods.ctrl && !mods.alt && matches!(key, Key::PageUp | Key::PageDown) {
        let page = i32::try_from(terminal.model.screen_lines()).unwrap_or(1);
        let lines = if *key == Key::PageUp { page } else { -page };
        terminal.model.scroll(lines);
        return Vec::new();
    }
    let Some(bytes) = encode_key(key, mods, terminal.model.mode()) else {
        return Vec::new();
    };
    // Typing goes back to the prompt, and lets go of the selection.
    terminal.model.scroll_to_bottom();
    terminal.model.clear_selection();
    terminal.send(tab, bytes)
}
