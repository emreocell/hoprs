//! The technical log panel (spec §8): collapsed by default, opened from the status bar.
//! While open, a 250 ms tick copies the lines the [`LogBuffer`] got since the last tick
//! (only those); the list filters by level and text and follows the newest line while
//! scrolled to the end.

use std::collections::VecDeque;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use tracing::Level;

use super::{App, Effect};
use crate::logbuf::{CAPACITY, LogBuffer, LogLine};
use crate::ui::layout::LOG_BODY_HEIGHT;
use crate::ui::virtual_list::ROW_HEIGHT;

/// How often the open panel takes the new lines.
pub const POLL: Duration = Duration::from_millis(250);

/// The level filter: a line shows if it is at least this severe. Debug shows
/// everything recorded, trace included.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
}

impl LogLevel {
    pub const ALL: [LogLevel; 4] = [Self::Error, Self::Warn, Self::Info, Self::Debug];

    pub fn allows(self, level: Level) -> bool {
        let most_verbose = match self {
            Self::Error => Level::ERROR,
            Self::Warn => Level::WARN,
            Self::Info => Level::INFO,
            Self::Debug => Level::TRACE,
        };
        // In `tracing`, more verbose levels compare greater.
        level <= most_verbose
    }
}

#[derive(Debug, Clone)]
pub enum LogMsg {
    /// The status bar's "Log" or the panel's "Hide".
    Toggle,
    /// The poll: copy the new lines.
    Tick,
    Level(LogLevel),
    Filter(String),
    /// "Copy shown": the lines the filters let through.
    CopyShown,
    /// Empties the panel (the lines logged from now on still come).
    Clear,
    Scrolled {
        offset_y: f32,
        /// The list shows its last line: new lines keep it there.
        at_end: bool,
    },
}

/// One line as the panel shows and searches it. Its time and search text are formatted
/// lazily, the first time they are needed (a row in view, a text filter, a copy): a
/// backlog of thousands of lines costs nothing more than sharing them.
#[derive(Debug)]
pub struct LogRow {
    pub line: Arc<LogLine>,
    time: OnceLock<String>,
    haystack: OnceLock<String>,
}

impl LogRow {
    fn new(line: Arc<LogLine>) -> Self {
        Self {
            line,
            time: OnceLock::new(),
            haystack: OnceLock::new(),
        }
    }

    /// `HH:MM:SS.mmm`, local time.
    pub fn time(&self) -> &str {
        self.time.get_or_init(|| {
            chrono::DateTime::<chrono::Local>::from(self.line.time)
                .format("%H:%M:%S%.3f")
                .to_string()
        })
    }

    /// The whole line in lower case, for the text filter.
    fn haystack(&self) -> &str {
        self.haystack.get_or_init(|| {
            let line = &self.line;
            format!("{} {} {}", line.level, line.target, line.message).to_lowercase()
        })
    }

    /// The line as "Copy shown" writes it.
    pub fn text(&self) -> String {
        format!(
            "{} {:<5} {} {}",
            self.time(),
            self.line.level,
            self.line.target,
            self.line.message
        )
    }
}

#[derive(Debug)]
pub struct LogPanel {
    pub open: bool,
    source: LogBuffer,
    /// The lines copied so far (at most [`CAPACITY`]), oldest first.
    rows: VecDeque<LogRow>,
    /// The `seq` of the next line to copy.
    next_seq: u64,
    pub level: LogLevel,
    /// The text filter, as typed.
    pub query: String,
    /// `seq` of the rows the filters let through, in order.
    shown: Vec<u64>,
    pub offset_y: f32,
    follow: bool,
}

impl LogPanel {
    /// A closed panel reading `source`.
    pub fn new(source: LogBuffer) -> Self {
        Self {
            open: false,
            source,
            rows: VecDeque::new(),
            next_seq: 0,
            level: LogLevel::Debug,
            query: String::new(),
            shown: Vec::new(),
            offset_y: 0.0,
            follow: true,
        }
    }

    /// How many lines the filters let through.
    pub fn shown_len(&self) -> usize {
        self.shown.len()
    }

    /// Whether any line was copied (filters aside).
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// The `i`-th line the filters let through.
    pub fn shown_row(&self, i: usize) -> Option<&LogRow> {
        let seq = *self.shown.get(i)?;
        let index = self
            .rows
            .binary_search_by_key(&seq, |row| row.line.seq)
            .ok()?;
        self.rows.get(index)
    }

    fn matches(&self, row: &LogRow, query: &str) -> bool {
        self.level.allows(row.line.level) && (query.is_empty() || row.haystack().contains(query))
    }

    fn query_lower(&self) -> String {
        self.query.trim().to_lowercase()
    }

    fn refilter(&mut self) {
        let query = self.query_lower();
        self.shown = self
            .rows
            .iter()
            .filter(|row| self.matches(row, &query))
            .map(|row| row.line.seq)
            .collect();
        // A shorter list: never scrolled past its end (and at the end, it follows).
        let max_offset = (self.shown.len() as f32 * ROW_HEIGHT - LOG_BODY_HEIGHT).max(0.0);
        if self.offset_y >= max_offset {
            self.offset_y = max_offset;
            self.follow = true;
        }
    }

    /// How many rows had their time formatted (for the tests).
    #[cfg(test)]
    pub fn formatted_rows(&self) -> usize {
        self.rows
            .iter()
            .filter(|row| row.time.get().is_some())
            .count()
    }

    /// Copies the lines logged since the last poll; whether any of them shows.
    fn poll(&mut self) -> bool {
        let new = self.source.since(self.next_seq);
        let Some(last) = new.last() else {
            return false;
        };
        self.next_seq = last.seq + 1;
        let query = self.query_lower();
        let before = self.shown.len();
        for line in new {
            let row = LogRow::new(line);
            if self.matches(&row, &query) {
                self.shown.push(row.line.seq);
            }
            self.rows.push_back(row);
        }
        let mut dropped = false;
        while self.rows.len() > CAPACITY {
            self.rows.pop_front();
            dropped = true;
        }
        if dropped && let Some(first) = self.rows.front().map(|row| row.line.seq) {
            let gone = self.shown.partition_point(|seq| *seq < first);
            self.shown.drain(..gone);
        }
        self.shown.len() != before || dropped
    }

    /// Keeps the newest line in view while the list follows it.
    fn follow_effect(&self) -> Vec<Effect> {
        if self.follow {
            vec![Effect::ScrollLogToEnd]
        } else {
            Vec::new()
        }
    }

    /// The shown lines, one per line, oldest first ("Copy shown").
    pub fn shown_text(&self) -> String {
        let mut text = String::new();
        for i in 0..self.shown.len() {
            if let Some(row) = self.shown_row(i) {
                text.push_str(&row.text());
                text.push('\n');
            }
        }
        text
    }
}

impl App {
    /// Reads `buffer` (the one `main` installed) from now on.
    pub fn attach_log(&mut self, buffer: LogBuffer) {
        self.log = LogPanel::new(buffer);
    }

    pub(super) fn log_msg(&mut self, msg: LogMsg) -> Vec<Effect> {
        let log = &mut self.log;
        match msg {
            LogMsg::Toggle => {
                log.open = !log.open;
                if log.open {
                    // Shown at once, not a tick later, at its newest line.
                    log.poll();
                    log.follow = true;
                    return vec![Effect::ScrollLogToEnd];
                }
            }
            LogMsg::Tick => {
                if log.open && log.poll() {
                    return log.follow_effect();
                }
            }
            LogMsg::Level(level) => {
                log.level = level;
                log.refilter();
                return log.follow_effect();
            }
            LogMsg::Filter(query) => {
                log.query = query;
                log.refilter();
                return log.follow_effect();
            }
            LogMsg::CopyShown => return vec![Effect::WriteClipboard(log.shown_text())],
            LogMsg::Clear => {
                log.rows.clear();
                log.shown.clear();
                log.offset_y = 0.0;
                log.follow = true;
            }
            LogMsg::Scrolled { offset_y, at_end } => {
                log.offset_y = offset_y;
                log.follow = at_end;
            }
        }
        Vec::new()
    }
}

/// Whether a list scrolled to `offset_y`, `viewport_h` tall, shows the end of
/// `content_h` (within a pixel).
pub fn at_end(offset_y: f32, viewport_h: f32, content_h: f32) -> bool {
    offset_y + viewport_h >= content_h - 1.0
}
