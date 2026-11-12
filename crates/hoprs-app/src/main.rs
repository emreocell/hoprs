//! hoprs: desktop SSH + SFTP + FTP/FTPS client.
//!
//! `main` owns the process-level concerns: the single-instance lock,
//! logging and handing the [`App`] to iced.

mod app;
mod backend;
mod effects;
mod ui;

use std::borrow::Cow;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use iced::Task;

use crate::app::{App, Message};
use crate::backend::{Backend, BackendApi, BackendConfig, LazyKeyringStore};

/// Smallest window that still fits the sidebar and two file panes.
const MIN_WINDOW: iced::Size = iced::Size::new(960.0, 640.0);
const INITIAL_WINDOW: iced::Size = iced::Size::new(1280.0, 800.0);

/// The default filter, used when `RUST_LOG` is unset or empty.
const DEFAULT_LOG_FILTER: &str = "info,suppaftp=warn";
/// suppaftp's own default; added unless `RUST_LOG` sets plain `suppaftp`.
const SUPPAFTP_DEFAULT: &str = "suppaftp=warn";
/// The most suppaftp may ever log at: at trace it logs `PASS <password>`.
const SUPPAFTP_CAP: &str = "debug";

fn main() -> iced::Result {
    let filter = log_filter(std::env::var("RUST_LOG").ok().as_deref());
    init_logging(&filter);

    let _instance = match acquire_first_instance_lock(&instance_lock_paths()) {
        Ok(Some(lock)) => Some(lock),
        Ok(None) => {
            println!("{}", ui::strings::ALREADY_RUNNING);
            std::process::exit(0);
        }
        Err(err) => {
            // No usable location at all: start anyway, unprotected.
            tracing::warn!(%err, "could not take the single-instance lock anywhere");
            None
        }
    };

    iced::application(boot, update, view)
        .subscription(subscription)
        .title(ui::strings::APP_TITLE)
        .theme(|_: &Hoprs| ui::theme::theme())
        .settings(iced::Settings {
            fonts: ui::theme::FONTS.map(Cow::Borrowed).to_vec(),
            default_font: ui::theme::INTER,
            default_text_size: ui::theme::TEXT_SM.into(),
            ..iced::Settings::default()
        })
        .window(iced::window::Settings {
            size: INITIAL_WINDOW,
            min_size: Some(MIN_WINDOW),
            ..iced::window::Settings::default()
        })
        .run()
}

/// iced's state: the pure [`App`] plus the backend its effects run against.
struct Hoprs {
    app: App,
    backend: Arc<dyn BackendApi>,
}

fn boot() -> (Hoprs, Task<Message>) {
    let backend = match open_backend() {
        Ok(backend) => backend,
        Err(err) => {
            // No profiles, no app: there is nothing to show yet without them.
            tracing::error!(%err, "cannot open the profiles");
            eprintln!("{}: {err}", ui::strings::CANNOT_OPEN_PROFILES);
            std::process::exit(1);
        }
    };
    let backend: Arc<dyn BackendApi> = Arc::new(backend);
    let (app, effects) = App::boot();
    let task = run_effects(&backend, effects);
    (Hoprs { app, backend }, task)
}

/// The real backend at the standard locations, secrets in the system keyring.
fn open_backend() -> hoprs_core::Result<Backend> {
    let config = BackendConfig::standard().ok_or_else(|| {
        hoprs_core::Error::new(hoprs_core::ErrorKind::Io, "No configuration directory")
    })?;
    Backend::open(config, Arc::new(LazyKeyringStore::new()))
}

/// Thin wrapper: the pure update decides, `effects::run` performs.
fn update(state: &mut Hoprs, message: Message) -> Task<Message> {
    let effects = state.app.update(message);
    run_effects(&state.backend, effects)
}

fn run_effects(backend: &Arc<dyn BackendApi>, effects: Vec<app::Effect>) -> Task<Message> {
    Task::batch(
        effects
            .into_iter()
            .map(|effect| effects::run(Arc::clone(backend), effect)),
    )
}

fn view(state: &Hoprs) -> iced::Element<'_, Message> {
    ui::layout::view(&state.app)
}

/// The keyboard for the file panes, the mouse button and dropped files (drags), and the
/// transfer engine's events.
fn subscription(state: &Hoprs) -> iced::Subscription<Message> {
    iced::Subscription::batch([
        iced::event::listen_with(ui::files::key_message),
        iced::event::listen_with(ui::queue::pointer_message),
        iced::Subscription::run_with(TransferFeed(Arc::clone(&state.backend)), |feed| {
            effects::transfer_feed(feed.0.as_ref())
        }),
    ])
}

/// The backend, as the identity of the transfer events' subscription: there is one
/// backend, so one subscription for the whole run.
struct TransferFeed(Arc<dyn BackendApi>);

impl std::hash::Hash for TransferFeed {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        "transfer-feed".hash(state);
    }
}

/// Where the lock may live, in order: `$XDG_RUNTIME_DIR/hoprs/`, the
/// config directory, the temp directory.
fn instance_lock_paths() -> Vec<PathBuf> {
    [
        dirs::runtime_dir(),
        dirs::config_dir(),
        Some(std::env::temp_dir()),
    ]
    .into_iter()
    .flatten()
    .map(|base| base.join("hoprs").join("instance.lock"))
    .collect()
}

/// Takes the lock at the first location that works. A location that fails
/// (can't be created, opened or locked) falls through to the next one;
/// one held by another instance stops the search with `Ok(None)`.
fn acquire_first_instance_lock(paths: &[PathBuf]) -> io::Result<Option<File>> {
    let mut last_err = io::Error::new(io::ErrorKind::NotFound, "no lock location");
    for path in paths {
        match acquire_instance_lock(path) {
            Ok(lock) => return Ok(lock),
            Err(err) => {
                tracing::warn!(%err, path = %path.display(), "lock location unusable; trying the next");
                last_err = err;
            }
        }
    }
    Err(last_err)
}

/// Takes the exclusive lock at `path`, creating its directory.
///
/// `Ok(None)` means another process holds it. The lock lives as long as
/// the returned file.
fn acquire_instance_lock(path: &Path) -> io::Result<Option<File>> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    // Called through the trait: std's inherent `File::try_lock` shadows it.
    match fs4::FileExt::try_lock(&file) {
        Ok(()) => Ok(Some(file)),
        Err(fs4::TryLockError::WouldBlock) => Ok(None),
        Err(fs4::TryLockError::Error(err)) => Err(err),
    }
}

/// The `EnvFilter` directives to log with, and whether a request for
/// suppaftp at trace had to be lowered.
#[derive(Debug, Clone, PartialEq, Eq)]
struct LogFilter {
    directives: String,
    capped: bool,
}

/// Builds the filter from `RUST_LOG`; suppaftp never goes above debug.
fn log_filter(rust_log: Option<&str>) -> LogFilter {
    let Some(rust_log) = rust_log.map(str::trim).filter(|s| !s.is_empty()) else {
        return LogFilter {
            directives: DEFAULT_LOG_FILTER.to_owned(),
            capped: false,
        };
    };

    let mut directives = Vec::new();
    let mut capped = false;
    let mut sets_suppaftp = false;
    for directive in rust_log.split(',').map(str::trim).filter(|d| !d.is_empty()) {
        // `selector=level`; the level never contains `]` or `}`, which
        // tells it apart from a `=` inside a span field filter.
        let (selector, level) = match directive.rsplit_once('=') {
            Some((selector, level)) if !level.contains([']', '}']) => (selector, Some(level)),
            _ => (directive, None),
        };
        let (target, level) = match level {
            Some(level) => (selector.split('[').next().unwrap_or(""), level_rank(level)),
            // A bare level is global; a bare target means trace for it.
            None => match level_rank(selector) {
                Some(rank) => ("", Some(rank)),
                None => (selector.split('[').next().unwrap_or(""), Some(TRACE)),
            },
        };
        let is_suppaftp = target == "suppaftp" || target.starts_with("suppaftp::");
        // A span-only directive (`[span]=level`) matches every target.
        let reaches_suppaftp = is_suppaftp || (target.is_empty() && selector.contains('['));
        // Only a plain `suppaftp=<valid level>` covers the whole target; a
        // span-scoped or submodule directive leaves the rest of suppaftp at
        // the global level. An unparsable level is dropped by `EnvFilter`.
        sets_suppaftp |= selector == "suppaftp" && level.is_some();

        if reaches_suppaftp && level == Some(TRACE) {
            directives.push(format!("{selector}={SUPPAFTP_CAP}"));
            capped = true;
        } else {
            directives.push(directive.to_owned());
        }
    }
    // Otherwise suppaftp (or the parts of it not covered by a scoped
    // directive) would inherit the global level, trace included.
    if !sets_suppaftp {
        directives.push(SUPPAFTP_DEFAULT.to_owned());
    }

    LogFilter {
        directives: directives.join(","),
        capped,
    }
}

const TRACE: u8 = 5;

/// A `tracing` level filter as a number, 0 (off) to 5 (trace), the way
/// `EnvFilter` parses it.
fn level_rank(level: &str) -> Option<u8> {
    const NAMES: [&str; 6] = ["off", "error", "warn", "info", "debug", "trace"];
    if let Ok(rank) = level.parse::<u8>() {
        return (rank <= TRACE).then_some(rank);
    }
    NAMES
        .iter()
        .position(|name| level.eq_ignore_ascii_case(name))
        .and_then(|rank| u8::try_from(rank).ok())
}

fn init_logging(filter: &LogFilter) {
    let env_filter = tracing_subscriber::EnvFilter::new(&filter.directives);
    tracing_subscriber::fmt().with_env_filter(env_filter).init();
    if filter.capped {
        tracing::warn!(
            "RUST_LOG would log suppaftp at trace, where it logs passwords; lowered to debug"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_instance_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hoprs").join("instance.lock");

        let first = acquire_instance_lock(&path).unwrap();
        assert!(first.is_some(), "the first instance takes the lock");
        assert!(
            acquire_instance_lock(&path).unwrap().is_none(),
            "a second instance is refused while the first runs"
        );

        drop(first);
        assert!(
            acquire_instance_lock(&path).unwrap().is_some(),
            "the lock is free again once the first instance exits"
        );
    }

    /// Whether the real `EnvFilter` built from `directives` lets a trace
    /// event through for `target`.
    fn trace_enabled(directives: &str, target: &str) -> bool {
        use tracing_subscriber::layer::SubscriberExt;

        let subscriber =
            tracing_subscriber::registry().with(tracing_subscriber::EnvFilter::new(directives));
        tracing::subscriber::with_default(subscriber, || match target {
            "suppaftp" => tracing::enabled!(target: "suppaftp", tracing::Level::TRACE),
            "suppaftp::async_ftp" => {
                tracing::enabled!(target: "suppaftp::async_ftp", tracing::Level::TRACE)
            }
            "suppaftp::foo" => tracing::enabled!(target: "suppaftp::foo", tracing::Level::TRACE),
            "hoprs_ssh" => tracing::enabled!(target: "hoprs_ssh", tracing::Level::TRACE),
            other => panic!("no callsite for {other}"),
        })
    }

    #[test]
    fn real_env_filter_never_traces_suppaftp() {
        let inputs = [
            None,
            Some("suppaftp=trace"),
            Some("trace"),
            Some("debug"),
            Some("trace,suppaftp[cmd]=warn"),
            Some("trace,suppaftp::foo=warn"),
            Some("trace,suppaftp::foo=trace,suppaftp[cmd]=trace"),
        ];
        for input in inputs {
            let directives = log_filter(input).directives;
            for target in ["suppaftp", "suppaftp::async_ftp", "suppaftp::foo"] {
                assert!(
                    !trace_enabled(&directives, target),
                    "{input:?} -> {directives:?} traces {target}"
                );
            }
        }
        // The check is not vacuous: other targets still trace.
        assert!(trace_enabled(
            &log_filter(Some("trace")).directives,
            "hoprs_ssh"
        ));
        assert!(!trace_enabled(&log_filter(None).directives, "hoprs_ssh"));
    }

    #[test]
    fn lock_falls_back_when_the_first_location_fails() {
        let dir = tempfile::tempdir().unwrap();
        // A file where the runtime directory should be: create_dir_all fails.
        let blocker = dir.path().join("not-a-dir");
        fs::write(&blocker, b"").unwrap();
        let broken = blocker.join("hoprs").join("instance.lock");
        let fallback = dir
            .path()
            .join("config")
            .join("hoprs")
            .join("instance.lock");
        let candidates = [broken, fallback.clone()];

        let first = acquire_first_instance_lock(&candidates).unwrap();
        assert!(first.is_some(), "the fallback location is locked");
        assert!(fallback.exists());
        assert!(
            acquire_first_instance_lock(&candidates).unwrap().is_none(),
            "a second instance hits the same fallback lock"
        );

        // Every location broken: the error is reported.
        assert!(acquire_first_instance_lock(&candidates[..1]).is_err());
    }

    #[test]
    fn trace_for_suppaftp_is_capped() {
        // Absent or empty: the default.
        for unset in [None, Some(""), Some("  ")] {
            let filter = log_filter(unset);
            assert_eq!(filter.directives, "info,suppaftp=warn");
            assert!(!filter.capped);
        }

        // Explicit trace for suppaftp is lowered to debug, with a warning.
        let filter = log_filter(Some("info,suppaftp=trace"));
        assert_eq!(filter.directives, "info,suppaftp=debug");
        assert!(filter.capped);

        // A bare target means trace for it.
        let filter = log_filter(Some("suppaftp"));
        assert_eq!(filter.directives, "suppaftp=debug");
        assert!(filter.capped);

        // Submodules, numeric levels and span filters are capped too.
        let filter = log_filter(Some("suppaftp::async_ftp=TRACE,suppaftp[cmd]=5"));
        assert_eq!(
            filter.directives,
            "suppaftp::async_ftp=debug,suppaftp[cmd]=debug,suppaftp=warn"
        );
        assert!(filter.capped);

        // A global trace would reach suppaftp: it keeps its default.
        let filter = log_filter(Some("trace"));
        assert_eq!(filter.directives, "trace,suppaftp=warn");
        assert!(!filter.capped);

        // A span-only directive applies to every target, suppaftp included.
        let filter = log_filter(Some("info,[connect]=trace"));
        assert_eq!(filter.directives, "info,[connect]=debug,suppaftp=warn");
        assert!(filter.capped);

        // Levels at or below debug are honoured; other targets untouched.
        let filter = log_filter(Some("hoprs_ssh=trace,suppaftp=debug"));
        assert_eq!(filter.directives, "hoprs_ssh=trace,suppaftp=debug");
        assert!(!filter.capped);
        let filter = log_filter(Some("warn,suppaftp=off"));
        assert_eq!(filter.directives, "warn,suppaftp=off");
        assert!(!filter.capped);

        // A target that only starts with the same letters is not suppaftp.
        let filter = log_filter(Some("suppaftp_ext=trace"));
        assert_eq!(filter.directives, "suppaftp_ext=trace,suppaftp=warn");
        assert!(!filter.capped);

        // Scoped suppaftp directives don't set the `suppaftp` target
        // itself: the cap still applies to it and its other modules.
        let filter = log_filter(Some("trace,suppaftp[cmd]=warn"));
        assert_eq!(filter.directives, "trace,suppaftp[cmd]=warn,suppaftp=warn");
        let filter = log_filter(Some("trace,suppaftp::foo=warn"));
        assert_eq!(filter.directives, "trace,suppaftp::foo=warn,suppaftp=warn");

        // An unparsable level sets nothing, so the default still applies.
        let filter = log_filter(Some("trace,suppaftp=loud"));
        assert_eq!(filter.directives, "trace,suppaftp=loud,suppaftp=warn");

        // Everything produced is valid `EnvFilter` syntax.
        for input in ["suppaftp", "[connect]=trace", "suppaftp[cmd{id=1}]=trace"] {
            let directives = log_filter(Some(input)).directives;
            assert!(
                tracing_subscriber::EnvFilter::builder()
                    .parse(&directives)
                    .is_ok(),
                "{directives}"
            );
        }
        assert_eq!(
            log_filter(Some("suppaftp[cmd{id=1}]=trace")).directives,
            "suppaftp[cmd{id=1}]=debug,suppaftp=warn"
        );
    }
}
