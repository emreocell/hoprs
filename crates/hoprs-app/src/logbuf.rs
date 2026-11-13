//! The technical log in memory: a `tracing` [`Layer`] that keeps the last
//! [`CAPACITY`] events in a ring buffer for the log panel.
//!
//! `main` installs it behind the same filter as the terminal output
//! ([`crate::log_subscriber`]), so it sees nothing the terminal wouldn't (suppaftp
//! and the terminal crates stay capped). Recording takes the lock only to push one
//! line; formatting happens before, and there is no I/O.

use std::collections::VecDeque;
use std::fmt::{self, Write as _};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::SystemTime;

use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;

/// Lines kept; the oldest go first.
pub const CAPACITY: usize = 5_000;
/// A message (with its fields) longer than this many bytes is cut.
pub const MAX_MESSAGE: usize = 2 * 1024;
/// Ends a cut message.
const ELLIPSIS: char = '…';

/// One event, as the panel shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogLine {
    /// Counts every line ever recorded, from 0: the panel asks for those after the
    /// last one it has.
    pub seq: u64,
    pub time: SystemTime,
    pub level: Level,
    pub target: String,
    /// The message, then the fields as `key=value`, at most [`MAX_MESSAGE`] bytes.
    pub message: String,
}

#[derive(Debug, Default)]
struct Ring {
    lines: VecDeque<Arc<LogLine>>,
    next_seq: u64,
}

/// The ring buffer, shared by the layer (writing) and the panel (reading). Cloning
/// shares it.
#[derive(Debug, Clone, Default)]
pub struct LogBuffer {
    ring: Arc<Mutex<Ring>>,
}

impl LogBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    fn ring(&self) -> MutexGuard<'_, Ring> {
        // A panic while pushing leaves a usable ring: keep logging.
        self.ring.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Records a line (its `seq` is set here), dropping the oldest past [`CAPACITY`].
    pub fn push(&self, time: SystemTime, level: Level, target: String, message: String) {
        let mut ring = self.ring();
        let seq = ring.next_seq;
        ring.next_seq += 1;
        if ring.lines.len() == CAPACITY {
            ring.lines.pop_front();
        }
        ring.lines.push_back(Arc::new(LogLine {
            seq,
            time,
            level,
            target,
            message,
        }));
    }

    /// The lines from `seq` on (those still kept), oldest first. Cheap: the lines are
    /// shared, not copied, and only the new ones are taken.
    pub fn since(&self, seq: u64) -> Vec<Arc<LogLine>> {
        let ring = self.ring();
        let first = ring.lines.front().map_or(ring.next_seq, |line| line.seq);
        let skip = usize::try_from(seq.saturating_sub(first)).unwrap_or(usize::MAX);
        ring.lines.iter().skip(skip).cloned().collect()
    }

    /// How many lines are kept.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.ring().lines.len()
    }

    /// Every kept line's text, one per line (for the tests).
    #[cfg(test)]
    pub fn text(&self) -> String {
        self.since(0)
            .iter()
            .map(|line| format!("{} {} {}\n", line.level, line.target, line.message))
            .collect()
    }
}

impl<S: Subscriber> Layer<S> for LogBuffer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let meta = event.metadata();
        let mut fields = Fields::default();
        event.record(&mut fields);
        // Events bridged from the `log` crate carry their real target as a field.
        let target = fields
            .log_target
            .take()
            .unwrap_or_else(|| meta.target().to_owned());
        self.push(SystemTime::now(), *meta.level(), target, fields.finish());
    }
}

/// Formats an event's fields: the message first, then `key=value` (values as `Debug`,
/// like the terminal output). Stops growing once past [`MAX_MESSAGE`].
#[derive(Default)]
struct Fields {
    message: String,
    rest: String,
    log_target: Option<String>,
}

impl Fields {
    fn full(&self) -> bool {
        self.message.len() + self.rest.len() > MAX_MESSAGE
    }

    fn finish(self) -> String {
        let mut text = self.message;
        if !self.rest.is_empty() {
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(&self.rest);
        }
        truncate(text)
    }
}

impl Visit for Fields {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "log.target" {
            self.log_target = Some(value.to_owned());
            return;
        }
        self.record_debug(field, &value);
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        let name = field.name();
        // The `log` bridge's own metadata (module, file, line) is noise here.
        if name.starts_with("log.") || self.full() {
            return;
        }
        if name == "message" {
            let _ = write!(self.message, "{value:?}");
        } else {
            if !self.rest.is_empty() {
                self.rest.push(' ');
            }
            let _ = write!(self.rest, "{name}={value:?}");
        }
    }
}

/// `text` cut to at most [`MAX_MESSAGE`] bytes (on a character boundary), with an
/// ellipsis when cut.
fn truncate(mut text: String) -> String {
    if text.len() <= MAX_MESSAGE {
        return text;
    }
    let mut end = MAX_MESSAGE - ELLIPSIS.len_utf8();
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    text.push(ELLIPSIS);
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_buffer_caps_at_5000() {
        let buffer = LogBuffer::new();
        for i in 0..CAPACITY + 7 {
            buffer.push(
                SystemTime::UNIX_EPOCH,
                Level::INFO,
                "t".into(),
                format!("line {i}"),
            );
        }
        assert_eq!(buffer.len(), 5_000);
        let all = buffer.since(0);
        assert_eq!(all.len(), 5_000);
        // The oldest went first; the numbering goes on.
        assert_eq!(all[0].seq, 7);
        assert_eq!(all[0].message, "line 7");
        assert_eq!(all.last().unwrap().seq, 5_006);
        // Only what is new since a line is taken.
        let new = buffer.since(5_004);
        assert_eq!(
            new.iter().map(|l| l.seq).collect::<Vec<_>>(),
            [5_004, 5_005, 5_006]
        );
        assert!(buffer.since(5_007).is_empty());
        assert!(buffer.since(u64::MAX).is_empty());
        // The lines are shared with the buffer, not copied.
        assert!(Arc::ptr_eq(&new[0], &buffer.since(5_004)[0]));
    }

    #[test]
    fn long_messages_are_truncated() {
        let buffer = LogBuffer::new();
        let subscriber = tracing_subscriber::layer::SubscriberExt::with(
            tracing_subscriber::registry(),
            buffer.clone(),
        );
        let long = "é".repeat(MAX_MESSAGE); // 2 bytes each: twice the limit
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(target: "hoprs_app", "{long}");
            tracing::info!(target: "hoprs_app", detail = %long, "short");
            tracing::warn!(target: "hoprs_app", site = "Shop", count = 3, "state changed");
        });
        let lines = buffer.since(0);
        assert_eq!(lines.len(), 3);
        for line in &lines[..2] {
            assert!(line.message.len() <= MAX_MESSAGE, "{}", line.message.len());
            assert!(line.message.len() > MAX_MESSAGE - 8);
            assert!(line.message.ends_with('…'));
        }
        assert!(lines[0].message.starts_with("éé"));
        assert!(lines[1].message.starts_with("short detail=éé"));
        // Short ones are whole: the message, then the fields as key=value.
        assert_eq!(lines[2].message, "state changed site=\"Shop\" count=3");
        assert_eq!(lines[2].level, Level::WARN);
        assert_eq!(lines[2].target, "hoprs_app");

        assert_eq!(truncate("abc".into()), "abc");
        let exact = "a".repeat(MAX_MESSAGE);
        assert_eq!(truncate(exact.clone()), exact);
    }
}
