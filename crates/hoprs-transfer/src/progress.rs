//! Aggregates per-transfer byte counters into rate-smoothed updates.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use crate::model::TransferId;

const EMA_ALPHA: f64 = 0.3;

/// A progress sample for one transfer whose byte count changed.
#[derive(Clone, Debug, PartialEq)]
pub struct ProgressUpdate {
    pub id: TransferId,
    pub bytes_done: u64,
    pub total: Option<u64>,
    pub bytes_per_sec: u64,
}

struct Entry {
    counter: Arc<AtomicU64>,
    total: Option<u64>,
    last_bytes: u64,
    last_at: Instant,
    ema: Option<f64>,
}

/// Tracks registered transfers and turns counter deltas into rates.
#[derive(Default)]
pub struct ProgressTracker {
    entries: HashMap<TransferId, Entry>,
}

impl ProgressTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a transfer and returns the counter its worker should update.
    ///
    /// `initial` is the byte position the transfer starts from (the resume
    /// offset); rates are measured from it, so already-present bytes do not
    /// count as transferred.
    pub fn register(&mut self, id: TransferId, total: Option<u64>, initial: u64) -> Arc<AtomicU64> {
        let counter = Arc::new(AtomicU64::new(initial));
        self.entries.insert(
            id,
            Entry {
                counter: Arc::clone(&counter),
                total,
                last_bytes: initial,
                last_at: Instant::now(),
                ema: None,
            },
        );
        counter
    }

    pub fn unregister(&mut self, id: TransferId) {
        self.entries.remove(&id);
    }

    /// Returns updates for transfers whose byte count changed since their
    /// previous sample.
    pub fn sample(&mut self, now: Instant) -> Vec<ProgressUpdate> {
        let mut out = Vec::new();
        for (id, e) in &mut self.entries {
            let bytes = e.counter.load(Ordering::Relaxed);
            if bytes == e.last_bytes {
                continue;
            }
            let dt = now.saturating_duration_since(e.last_at).as_secs_f64();
            if dt > 0.0 {
                let rate = bytes.saturating_sub(e.last_bytes) as f64 / dt;
                e.ema = Some(match e.ema {
                    None => rate,
                    Some(prev) => EMA_ALPHA * rate + (1.0 - EMA_ALPHA) * prev,
                });
                e.last_at = now;
            }
            e.last_bytes = bytes;
            out.push(ProgressUpdate {
                id: *id,
                bytes_done: bytes,
                total: e.total,
                bytes_per_sec: e.ema.unwrap_or(0.0) as u64,
            });
        }
        out.sort_by_key(|u| u.id);
        out
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn samples_rate_and_only_changes() {
        let mut t = ProgressTracker::new();
        let c = t.register(TransferId(1), Some(10_000), 0);
        let t0 = Instant::now();
        c.store(1000, Ordering::Relaxed);
        c.store(3000, Ordering::Relaxed);
        let now = t0 + Duration::from_millis(100);
        let u = t.sample(now);
        assert_eq!(u.len(), 1);
        assert_eq!(u[0].bytes_done, 3000);
        assert_eq!(u[0].total, Some(10_000));
        assert!(u[0].bytes_per_sec > 0);
        assert!(t.sample(now + Duration::from_millis(100)).is_empty());
    }

    #[test]
    fn zero_delta_time_does_not_panic() {
        let mut t = ProgressTracker::new();
        let c = t.register(TransferId(1), None, 0);
        c.store(500, Ordering::Relaxed);
        let u = t.sample(Instant::now() - Duration::from_secs(1));
        assert_eq!(u[0].bytes_done, 500);
        assert_eq!(u[0].bytes_per_sec, 0);
    }

    #[test]
    fn resumed_transfer_rate_is_measured_from_initial_offset() {
        let mut t = ProgressTracker::new();
        let c = t.register(TransferId(1), Some(2_000_000), 900_000);
        let t0 = Instant::now();
        c.store(1_000_000, Ordering::Relaxed);
        let u = t.sample(t0 + Duration::from_secs(1));
        assert_eq!(u.len(), 1);
        assert_eq!(u[0].bytes_done, 1_000_000);
        // 100_000 bytes in ~1 s; the 900_000 already on disk must not count.
        assert!(u[0].bytes_per_sec <= 110_000, "rate {}", u[0].bytes_per_sec);
    }

    #[test]
    fn resumed_transfer_without_new_bytes_reports_nothing() {
        let mut t = ProgressTracker::new();
        let c = t.register(TransferId(1), Some(2_000_000), 900_000);
        assert_eq!(c.load(Ordering::Relaxed), 900_000);
        assert!(t.sample(Instant::now()).is_empty());
    }
}
