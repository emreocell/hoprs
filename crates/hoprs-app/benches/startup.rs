//! Spec §9, "opens in < 500 ms" and "idle memory < 80 MB": runs the release binary with
//! `HOPRS_PROBE_STARTUP=1` [`RUNS`] times and takes the medians of what it prints (see
//! `hoprs_app::bench::parse_probe_line`).
//!
//! Needs an X display: run it under `xvfb-run -a`. The app runs on the tiny-skia
//! renderer (`ICED_BACKEND=tiny-skia`), over X11 (`WAYLAND_DISPLAY` removed), with a fresh
//! home (no profiles, no sessions) per run. Prints a Markdown table; exits with status 1
//! when a target is missed, 2 when the probe cannot run.

use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use hoprs_app::bench::{PROBE_ENV, PROBE_IDLE, ProbeLine, parse_probe_line};

const RUNS: usize = 5;
const STARTUP_TARGET_MS: f64 = 500.0;
const IDLE_RSS_TARGET_MB: f64 = 80.0;
/// A probe that has not finished by then is stuck.
const RUN_TIMEOUT: Duration = Duration::from_secs(60);

/// What one probe run measured.
struct Run {
    /// From `run()` to the first frame drawn, as the app measured it.
    startup_ms: f64,
    /// From spawning the process to reading its first probe line (exec and loading
    /// included).
    wall_ms: f64,
    rss_mb: f64,
    idle_rss_mb: f64,
}

fn main() {
    // `cargo bench` passes `--bench` (and any filter); there is nothing to filter here.
    if std::env::var_os("DISPLAY").is_none() {
        eprintln!("startup bench: no DISPLAY; run it under `xvfb-run -a`");
        std::process::exit(2);
    }
    let exe = env!("CARGO_BIN_EXE_hoprs");
    let runs: Vec<Run> = (0..RUNS)
        .map(|i| match probe(Path::new(exe)) {
            Ok(run) => {
                eprintln!(
                    "run {}: startup_ms={:.0} wall_ms={:.0} rss_mb={:.1} idle_rss_mb={:.1}",
                    i + 1,
                    run.startup_ms,
                    run.wall_ms,
                    run.rss_mb,
                    run.idle_rss_mb
                );
                run
            }
            Err(err) => {
                eprintln!("startup bench: run {} failed: {err}", i + 1);
                std::process::exit(2);
            }
        })
        .collect();

    let startup = median(runs.iter().map(|r| r.startup_ms));
    let wall = median(runs.iter().map(|r| r.wall_ms));
    let rss = median(runs.iter().map(|r| r.rss_mb));
    let idle = median(runs.iter().map(|r| r.idle_rss_mb));
    let startup_ok = startup < STARTUP_TARGET_MS;
    let idle_ok = idle < IDLE_RSS_TARGET_MB;

    println!("### Start-up and idle memory (median of {RUNS}, Xvfb, tiny-skia)\n");
    println!("| Target (spec §9) | Measured | Criterion | Result |");
    println!("|---|---|---|---|");
    println!(
        "| Open < 500 ms | {startup:.0} ms (process spawn to first frame: {wall:.0} ms) | `startup_ms < {STARTUP_TARGET_MS}` | {} |",
        verdict(startup_ok)
    );
    println!(
        "| Idle memory < 80 MB | {idle:.1} MB ({rss:.1} MB at the first frame) | `rss_mb < {IDLE_RSS_TARGET_MB}`, {} s after the first frame | {} |",
        PROBE_IDLE.as_secs(),
        verdict(idle_ok)
    );
    if !(startup_ok && idle_ok) {
        std::process::exit(1);
    }
}

fn verdict(ok: bool) -> &'static str {
    if ok { "pass" } else { "**FAIL**" }
}

fn median(values: impl Iterator<Item = f64>) -> f64 {
    let mut values: Vec<f64> = values.collect();
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

/// One probe run in a fresh home.
fn probe(exe: &Path) -> Result<Run, String> {
    let home = tempfile::tempdir().map_err(|e| e.to_string())?;
    let runtime = home.path().join("run");
    std::fs::create_dir(&runtime).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&runtime, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
    }
    let start = Instant::now();
    let mut child = Command::new(exe)
        .env(PROBE_ENV, "1")
        .env("ICED_BACKEND", "tiny-skia")
        .env_remove("WAYLAND_DISPLAY")
        .env("HOME", home.path())
        .env("XDG_CONFIG_HOME", home.path().join("config"))
        .env("XDG_DATA_HOME", home.path().join("data"))
        .env("XDG_STATE_HOME", home.path().join("state"))
        .env("XDG_CACHE_HOME", home.path().join("cache"))
        .env("XDG_RUNTIME_DIR", &runtime)
        .env_remove("RUST_LOG")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot start {}: {e}", exe.display()))?;

    // Lines (with the time each was read) from a thread, so a stuck probe times out.
    let stdout = child.stdout.take().expect("piped stdout");
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if tx.send((line, start.elapsed())).is_err() {
                break;
            }
        }
    });
    let mut stderr = child.stderr.take().expect("piped stderr");
    let errors = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text);
        text
    });

    let mut startup = None;
    let mut idle = None;
    let deadline = start + RUN_TIMEOUT;
    while idle.is_none() {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok((line, at)) => match parse_probe_line(&line) {
                Some(ProbeLine::Startup { startup_ms, rss_mb }) => {
                    startup = Some((startup_ms as f64, at.as_secs_f64() * 1e3, rss_mb));
                }
                Some(ProbeLine::Idle { rss_mb }) => idle = Some(rss_mb),
                None => {}
            },
            Err(_) => break,
        }
    }
    if idle.is_none() {
        let _ = child.kill();
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    let errors = errors.join().unwrap_or_default();
    match (startup, idle) {
        (Some((startup_ms, wall_ms, rss_mb)), Some(idle_rss_mb)) if status.success() => Ok(Run {
            startup_ms,
            wall_ms,
            rss_mb,
            idle_rss_mb,
        }),
        _ => Err(format!(
            "no probe lines (exit {status}); stderr:\n{}",
            errors.trim()
        )),
    }
}
