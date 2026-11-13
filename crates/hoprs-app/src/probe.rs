//! The start-up probe (spec §9: open in < 500 ms, idle in < 80 MB).
//!
//! With `HOPRS_PROBE_STARTUP=1` the app starts as usual, then prints two lines on stdout
//! and exits:
//!
//! - when the first frame has been drawn: `startup_ms=<n> rss_mb=<m>`, the time since
//!   `run` began and the resident memory;
//! - [`IDLE`] later, no session open: `idle_rss_mb=<m>`.
//!
//! "Drawn" is the first message after the first `RedrawRequested`: iced draws and presents
//! that frame before it takes the next message. Memory is `VmRSS` from `/proc/self/status`
//! in MB (10^6 bytes); `NaN` where there is no `/proc`.

use std::fmt;
use std::time::{Duration, Instant};

use iced::{Subscription, Task};

/// The variable that turns the probe on (value `1`).
pub const ENV: &str = "HOPRS_PROBE_STARTUP";
/// How long the probe waits, idle, after the first frame before reading memory again.
pub const IDLE: Duration = Duration::from_secs(2);

/// Whether this run is a probe.
pub fn enabled() -> bool {
    std::env::var_os(ENV).is_some_and(|value| value == "1")
}

/// One line the probe prints.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ProbeLine {
    /// At the first frame drawn.
    Startup { startup_ms: u64, rss_mb: f64 },
    /// [`IDLE`] after it.
    Idle { rss_mb: f64 },
}

impl fmt::Display for ProbeLine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Startup { startup_ms, rss_mb } => {
                write!(f, "startup_ms={startup_ms} rss_mb={rss_mb:.1}")
            }
            Self::Idle { rss_mb } => write!(f, "idle_rss_mb={rss_mb:.1}"),
        }
    }
}

/// Reads a line the probe printed; `None` for anything else (log lines included).
pub fn parse_probe_line(line: &str) -> Option<ProbeLine> {
    let fields: Vec<(&str, &str)> = line
        .split_whitespace()
        .map(|field| field.split_once('='))
        .collect::<Option<_>>()?;
    match fields[..] {
        [("startup_ms", startup_ms), ("rss_mb", rss_mb)] => Some(ProbeLine::Startup {
            startup_ms: startup_ms.parse().ok()?,
            rss_mb: rss_mb.parse().ok()?,
        }),
        [("idle_rss_mb", rss_mb)] => Some(ProbeLine::Idle {
            rss_mb: rss_mb.parse().ok()?,
        }),
        _ => None,
    }
}

/// The resident memory of this process, in MB.
pub fn rss_mb() -> f64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| vm_rss_kib(&status))
        .map_or(f64::NAN, |kib| (kib * 1024) as f64 / 1_000_000.0)
}

/// `VmRSS` of a `/proc/<pid>/status` text, in KiB.
fn vm_rss_kib(status: &str) -> Option<u64> {
    let value = status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))?;
    value.split_whitespace().next()?.parse().ok()
}

/// A step of the probe, carried by iced.
#[derive(Debug, Clone, Copy)]
pub enum Step {
    /// A `RedrawRequested`: a frame is being drawn.
    Frame,
    /// The first frame is on screen.
    Drawn,
    /// [`IDLE`] has passed since.
    Idle,
}

/// The probe's state.
#[derive(Debug, Clone, Copy)]
pub struct Probe {
    started: Instant,
    first_frame: bool,
}

impl Probe {
    /// A probe timing from `started` (the start of `run`).
    pub fn new(started: Instant) -> Self {
        Self {
            started,
            first_frame: false,
        }
    }

    /// The frames, until the first one.
    pub fn subscription(&self) -> Option<Subscription<Step>> {
        (!self.first_frame).then(|| iced::window::frames().map(|_| Step::Frame))
    }

    pub fn step(&mut self, step: Step) -> Task<Step> {
        match step {
            Step::Frame if self.first_frame => Task::none(),
            Step::Frame => {
                self.first_frame = true;
                // Pending once, so it comes back after iced has drawn this frame.
                Task::future(async {
                    tokio::task::yield_now().await;
                    Step::Drawn
                })
            }
            Step::Drawn => {
                let startup_ms =
                    u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX);
                println!(
                    "{}",
                    ProbeLine::Startup {
                        startup_ms,
                        rss_mb: rss_mb(),
                    }
                );
                Task::perform(tokio::time::sleep(IDLE), |()| Step::Idle)
            }
            Step::Idle => {
                println!("{}", ProbeLine::Idle { rss_mb: rss_mb() });
                iced::exit()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_probe_lines() {
        assert_eq!(
            parse_probe_line("startup_ms=312 rss_mb=41.5"),
            Some(ProbeLine::Startup {
                startup_ms: 312,
                rss_mb: 41.5
            })
        );
        assert_eq!(
            parse_probe_line("idle_rss_mb=43.0\n"),
            Some(ProbeLine::Idle { rss_mb: 43.0 })
        );
        // Whatever the probe prints reads back the same.
        for line in [
            ProbeLine::Startup {
                startup_ms: 7,
                rss_mb: 12.3,
            },
            ProbeLine::Idle { rss_mb: 79.9 },
        ] {
            assert_eq!(parse_probe_line(&line.to_string()), Some(line));
        }
        // No /proc: the memory is NaN, still a probe line.
        assert!(matches!(
            parse_probe_line("startup_ms=1 rss_mb=NaN"),
            Some(ProbeLine::Startup { startup_ms: 1, rss_mb }) if rss_mb.is_nan()
        ));
    }

    #[test]
    fn other_lines_are_not_probe_lines() {
        for line in [
            "",
            "2026-11-13T10:00:00Z  INFO hoprs_app: startup_ms=3 rss_mb=4",
            "startup_ms=312",
            "rss_mb=41.5",
            "startup_ms=abc rss_mb=41.5",
            "startup_ms=-3 rss_mb=41.5",
            "startup_ms=312 rss_mb=41.5 extra=1",
            "startup_ms=312 startup_ms=313 rss_mb=1",
            "idle_rss_mb=",
            "idle_rss_mb=4 rss_mb=4",
        ] {
            assert_eq!(parse_probe_line(line), None, "{line:?}");
        }
    }

    #[test]
    fn reads_vm_rss() {
        let status = "Name:\thoprs-app\nVmPeak:\t  900000 kB\nVmRSS:\t   45312 kB\nThreads:\t12\n";
        assert_eq!(vm_rss_kib(status), Some(45_312));
        assert_eq!(vm_rss_kib("Name:\tx\n"), None);
        assert_eq!(vm_rss_kib("VmRSS:\tlots kB\n"), None);
    }
}
