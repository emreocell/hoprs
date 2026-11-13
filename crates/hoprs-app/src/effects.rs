//! Turns each [`Effect`] into an [`iced::Task`]: the only place that calls
//! the backend.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use futures_util::{Stream, StreamExt, stream};
use hoprs_core::{Error, ErrorKind, RemotePath, Site, SiteId, WorkspaceId};
use hoprs_transfer::{Event, TransferSpec};
use tokio::sync::{broadcast, mpsc};

use hoprs_import::ImportOptions;
use hoprs_vault::ImportMode;
use secrecy::SecretString;

use crate::app::{Effect, FileAction, Message, PickTarget, ProfileOp, RequestId, TabId};
use crate::backend::{
    BackendApi, KnownHostEntry, PromptAnswer, PromptId, SessionId, Side, WorkspaceTabs,
};
use crate::settings::Settings;
use crate::ui::site_form::PasswordChange;
use crate::ui::{dialogs, files, strings, tabs};

/// Runs one effect against the backend.
pub fn run(backend: Arc<dyn BackendApi>, effect: Effect) -> iced::Task<Message> {
    use iced::Task;
    match effect {
        Effect::LoadProfiles(request) => {
            Task::perform(async move { backend.profiles().await }, move |result| {
                Message::ProfilesLoaded { request, result }
            })
        }
        Effect::SaveSite { site, password } => {
            let op = ProfileOp::SaveSite(site.id.clone());
            Task::perform(save_site(backend, site, password), move |result| {
                Message::FormDone { op, result }
            })
        }
        Effect::DeleteSite(id) => {
            let op = ProfileOp::DeleteSite(id.clone());
            Task::perform(
                async move { backend.delete_site(id).await },
                move |result| Message::FormDone { op, result },
            )
        }
        Effect::SaveWorkspace(workspace) => {
            let op = ProfileOp::SaveWorkspace(workspace.id.clone());
            Task::perform(
                async move { backend.save_workspace(workspace).await },
                move |result| Message::FormDone { op, result },
            )
        }
        Effect::DeleteWorkspace(id) => {
            let op = ProfileOp::DeleteWorkspace(id.clone());
            Task::perform(
                async move { backend.delete_workspace(id).await },
                move |result| Message::FormDone { op, result },
            )
        }
        Effect::ConnectSites(sites) => Task::run(connect_sites(backend, sites), |m| m),
        Effect::CloseSession(session) => {
            Task::future(async move { backend.close(session).await }).discard()
        }
        Effect::OpenProfileFolder => Task::perform(
            open_folder(backend.profile_dir()),
            Message::ProfileFolderOpened,
        ),
        // Never cancelled: a listing the user moved away from finishes, and `update`
        // drops its result by `request_id`.
        Effect::List {
            tab,
            side,
            request_id,
            session,
            path,
        } => Task::future(list(backend, tab, side, request_id, session, path)),
        Effect::ScrollTabs { x } => iced::widget::operation::snap_to(
            tabs::strip_id(),
            iced::widget::operation::RelativeOffset {
                x: Some(x),
                y: None,
            },
        ),
        Effect::ScrollPane {
            tab,
            side,
            offset_y,
        } => iced::widget::operation::scroll_to(
            files::scroll_id(tab, side),
            iced::widget::operation::AbsoluteOffset {
                x: None,
                y: Some(offset_y),
            },
        ),
        // Never cancelled either: the result finds its pane by `op`, or is dropped.
        Effect::FileOp {
            tab,
            side,
            op,
            session,
            action,
        } => Task::run(file_op(backend, tab, side, op, session, action), |m| m),
        Effect::FocusEdit {
            tab,
            side,
            select_end,
        } => {
            let id = files::edit_id(tab, side);
            iced::widget::operation::focus(id.clone())
                .chain(iced::widget::operation::select_range(id, 0, select_end))
        }
        Effect::KeyUnlessTyping { key, shift } => {
            iced::advanced::widget::operate(files::text_focused()).then(move |typing| {
                if typing {
                    Task::none()
                } else {
                    Task::done(Message::Key { key, shift })
                }
            })
        }
        Effect::Enqueue(specs) => Task::perform(enqueue_all(backend, specs), Message::Enqueued),
        Effect::Control { id, op } => Task::perform(
            async move { backend.control(id, op).await },
            move |result| Message::ControlDone { id, result },
        ),
        Effect::SnapshotQueue(request) => Task::perform(
            async move { backend.transfer_snapshot().await },
            move |snapshot| Message::QueueSnapshot { request, snapshot },
        ),
        Effect::ClearFinished => Task::perform(
            async move { backend.clear_finished().await },
            Message::Cleared,
        ),
        Effect::AnswerPrompt { id, answer } => {
            Task::future(answer_prompt(backend, id, answer)).discard()
        }
        Effect::FocusDialogField(n) => iced::widget::operation::focus(dialogs::field_id(n)),
        Effect::Unfocus => {
            iced::advanced::widget::operate(iced::advanced::widget::operation::focusable::unfocus())
        }
        Effect::KnownHostEntries { tab, host, port } => {
            Task::future(known_host_entries(backend, tab, host, port))
        }
        Effect::RemoveKnownHost {
            tab,
            host,
            port,
            expected,
        } => Task::future(remove_known_host(backend, tab, host, port, expected)),
        Effect::RestartSite { close, site } => Task::run(restart_site(backend, close, site), |m| m),
        Effect::Reconnect { session } => Task::future(reconnect(backend, session)),
        Effect::SaveTabs { workspace, tabs } => Task::future(save_tabs(backend, workspace, tabs)),
        Effect::Shutdown { save, window } => {
            Task::perform(shutdown(backend, save, SHUTDOWN_LIMIT), move |_| {
                Message::ShutdownDone(window)
            })
        }
        Effect::ExitWindow(window) => {
            exit_watchdog();
            iced::window::close(window)
        }
        Effect::LoadSettings => Task::done(Message::SettingsLoaded(backend.settings())),
        Effect::LoadQueueStatus => Task::done(Message::QueueStatus(backend.queue_error())),
        Effect::SaveSettings(settings) => Task::future(save_settings(backend, settings)),
        Effect::SetPolicy(policy) => {
            Task::future(async move { backend.set_policy(policy).await }).discard()
        }
        Effect::SetWorkers(workers) => {
            backend.set_workers(workers);
            Task::none()
        }
        Effect::CheckExportTarget(path) => Task::future(async move {
            // An error (no permission to look) counts as "not there": the export itself
            // then reports what is wrong.
            let probe = path.clone();
            let exists = tokio::task::spawn_blocking(move || probe.try_exists())
                .await
                .is_ok_and(|found| found.unwrap_or(false));
            Message::ExportTargetChecked { path, exists }
        }),
        Effect::PickFile(target) => Task::perform(pick_file(target), move |path| {
            Message::FilePicked { target, path }
        }),
        Effect::ImportFilezilla {
            path,
            folders_as_workspaces,
        } => Task::future(import_filezilla(backend, path, folders_as_workspaces)),
        Effect::DeleteSitemanager(path) => Task::future(delete_sitemanager(backend, path)),
        Effect::ExportBackup { path, password } => {
            Task::future(export_backup(backend, path, password))
        }
        Effect::RestoreBackup {
            path,
            password,
            mode,
        } => Task::future(restore_backup(backend, path, password, mode)),
    }
}

/// "Import from FileZilla": the backend reads, parses and applies the file on the
/// blocking pool.
pub async fn import_filezilla(
    backend: Arc<dyn BackendApi>,
    path: PathBuf,
    folders_as_workspaces: bool,
) -> Message {
    let options = ImportOptions {
        folders_as_workspaces,
    };
    Message::Imported(backend.import_filezilla(path, options).await)
}

pub async fn delete_sitemanager(backend: Arc<dyn BackendApi>, path: PathBuf) -> Message {
    Message::SitemanagerDeleted(backend.delete_sitemanager(path).await)
}

/// Writes a backup; the password goes straight from the effect to the vault.
pub async fn export_backup(
    backend: Arc<dyn BackendApi>,
    path: PathBuf,
    password: SecretString,
) -> Message {
    Message::BackupExported(backend.export_backup(path, password).await)
}

pub async fn restore_backup(
    backend: Arc<dyn BackendApi>,
    path: PathBuf,
    password: SecretString,
    mode: ImportMode,
) -> Message {
    Message::BackupRestored(backend.restore_backup(path, password, mode).await)
}

pub async fn save_settings(backend: Arc<dyn BackendApi>, settings: Settings) -> Message {
    Message::SettingsSaved(backend.save_settings(settings).await)
}

/// The system file dialog (on Linux the XDG desktop portal, else zenity), on its own
/// thread: the UI keeps running. `None` if cancelled or no dialog is available.
async fn pick_file(target: PickTarget) -> Option<PathBuf> {
    let home = dirs::home_dir();
    let dialog = rfd::AsyncFileDialog::new();
    let dialog = match &home {
        Some(home) => dialog.set_directory(home),
        None => dialog,
    };
    let handle = match target {
        PickTarget::Sitemanager => {
            let dialog = match hoprs_import::default_sitemanager_path()
                .as_deref()
                .and_then(Path::parent)
                .filter(|dir| dir.is_dir())
            {
                Some(dir) => dialog.set_directory(dir),
                None => dialog,
            };
            dialog
                .set_title(strings::PICK_SITEMANAGER)
                .add_filter(strings::PICK_FILTER_XML, &["xml"])
                .pick_file()
                .await
        }
        PickTarget::ExportBackup => {
            dialog
                .set_title(strings::PICK_EXPORT)
                .set_file_name(strings::BACKUP_FILE_NAME)
                .save_file()
                .await
        }
        PickTarget::RestoreBackup => dialog.set_title(strings::PICK_RESTORE).pick_file().await,
    };
    handle.map(|handle| handle.path().to_path_buf())
}

/// The longest the window waits for the backend to shut down once asked to close.
pub const SHUTDOWN_LIMIT: Duration = Duration::from_secs(5);
/// The part of [`SHUTDOWN_LIMIT`] the tab save may take: the engine always gets the rest.
pub const SAVE_LIMIT: Duration = Duration::from_secs(2);
/// After the window is asked to close, the process ends at the latest this much later,
/// even if something still holds the runtime (an abandoned blocking save, for one).
pub const EXIT_GRACE: Duration = Duration::from_secs(2);

/// How [`shutdown`] ended. The window closes whatever it is.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ShutdownOutcome {
    Done,
    /// `limit` ran out first; what was left is abandoned (the process exits).
    TimedOut,
    /// The backend's shutdown panicked.
    Failed,
}

/// The window is closing (Review Focus 5), in this order and within `limit` in all:
/// 1. saves the open workspace's tabs (`save`), through the vault's atomic write, within
///    [`SAVE_LIMIT`]; a failure, a panic or a save that takes too long is logged and
///    abandoned (never waited for), and the shutdown goes on;
/// 2. shuts the transfer engine down, so its running items reopen paused next time;
/// 3. closes the sessions ([`BackendApi::shutdown`] does 2 and 3), in what is left of
///    `limit`.
///
/// Each step runs in its own task, so a panic becomes an outcome instead of a lost
/// message. The caller closes the window when this returns, whatever the outcome.
pub async fn shutdown(
    backend: Arc<dyn BackendApi>,
    save: Option<(WorkspaceId, WorkspaceTabs)>,
    limit: Duration,
) -> ShutdownOutcome {
    let deadline = tokio::time::Instant::now() + limit;
    if let Some((workspace, tabs)) = save {
        let saver = backend.clone();
        let mut saving =
            tokio::spawn(async move { saver.save_workspace_tabs(workspace, tabs).await });
        match tokio::time::timeout(SAVE_LIMIT.min(limit), &mut saving).await {
            Ok(Ok(Ok(()))) => {}
            Ok(Ok(Err(error))) => {
                tracing::warn!(%error, "cannot save the workspace's tabs on close");
            }
            Ok(Err(error)) => {
                tracing::error!(%error, "saving the workspace's tabs on close panicked");
            }
            Err(_) => {
                saving.abort();
                tracing::warn!(limit = ?SAVE_LIMIT, "saving the workspace's tabs on close took too long; skipped");
            }
        }
    }
    let mut stopping = tokio::spawn(async move { backend.shutdown().await });
    match tokio::time::timeout_at(deadline, &mut stopping).await {
        Ok(Ok(())) => ShutdownOutcome::Done,
        Ok(Err(error)) => {
            tracing::error!(%error, "the backend's shutdown panicked; closing anyway");
            ShutdownOutcome::Failed
        }
        Err(_) => {
            stopping.abort();
            tracing::warn!(
                ?limit,
                "the backend did not shut down in time; closing anyway"
            );
            ShutdownOutcome::TimedOut
        }
    }
}

/// Ends the process [`EXIT_GRACE`] from now unless it ended already: closing the window
/// ends iced's loop, but dropping the runtime waits for blocking tasks still running.
fn exit_watchdog() {
    let spawned = std::thread::Builder::new()
        .name("exit-watchdog".into())
        .spawn(|| {
            std::thread::sleep(EXIT_GRACE);
            tracing::warn!("still running after the window closed; exiting");
            std::process::exit(0);
        });
    if let Err(error) = spawned {
        tracing::warn!(%error, "cannot start the exit watchdog");
    }
}

/// Saves a workspace's tabs and folders.
pub async fn save_tabs(
    backend: Arc<dyn BackendApi>,
    workspace: WorkspaceId,
    tabs: WorkspaceTabs,
) -> Message {
    Message::TabsSaved(backend.save_workspace_tabs(workspace, tabs).await)
}

/// The banner's Reconnect: the backend's reconnect of the session (`reconnect_now`, then
/// the wait that also settles the secrets typed with "save").
pub async fn reconnect(backend: Arc<dyn BackendApi>, session: SessionId) -> Message {
    let result = backend.reconnect(session).await;
    Message::Reconnected { session, result }
}

/// One session's feed for its subscription: the state now, then each change, as
/// [`Message::SessionState`] with how the session logged in. Ends when the session is
/// closed (its watch goes away), or at once if it is already.
pub fn session_feed(
    backend: Arc<dyn BackendApi>,
    session: SessionId,
) -> impl Stream<Item = Message> + Send + 'static {
    let watch = backend.session_state(session);
    stream::unfold((watch, true), move |(watch, first)| {
        let backend = backend.clone();
        async move {
            let mut watch = watch?;
            if !first {
                watch.changed().await.ok()?;
            }
            let state = watch.borrow_and_update().clone();
            let message = Message::SessionState {
                session,
                state,
                auth: backend.session_auth(session),
            };
            Some((message, (Some(watch), false)))
        }
    })
}

/// Delivers a dialog's answer (the effect's secret goes straight to the backend).
pub async fn answer_prompt(backend: Arc<dyn BackendApi>, id: PromptId, answer: PromptAnswer) {
    backend.answer_prompt(id, answer).await;
}

/// Reads the `known_hosts` lines "Remove the old entry" would delete.
pub async fn known_host_entries(
    backend: Arc<dyn BackendApi>,
    tab: TabId,
    host: String,
    port: u16,
) -> Message {
    let result = backend.known_host_entries(host, port).await;
    Message::KnownHostEntries { tab, result }
}

/// Removes the `known_hosts` lines of `host:port`, if still the ones confirmed.
pub async fn remove_known_host(
    backend: Arc<dyn BackendApi>,
    tab: TabId,
    host: String,
    port: u16,
    expected: Vec<KnownHostEntry>,
) -> Message {
    let result = backend.remove_known_host(host, port, expected).await;
    Message::KnownHostRemoved { tab, result }
}

/// Closes `close` first, then connects `site`: the connect never finds (and reuses) the
/// old session.
pub fn restart_site(
    backend: Arc<dyn BackendApi>,
    close: Option<SessionId>,
    site: SiteId,
) -> impl Stream<Item = Message> + Send + 'static {
    let closer = backend.clone();
    stream::once(async move {
        if let Some(session) = close {
            closer.close(session).await;
        }
    })
    .flat_map(move |()| connect_sites(backend.clone(), vec![site.clone()]))
}

/// The broker's dialogs for the UI's subscription: each request or withdrawal as
/// [`Message::Prompt`]. The backend hands its channel out once; without it (a second
/// subscription) the stream is empty.
pub fn prompt_feed(backend: &dyn BackendApi) -> impl Stream<Item = Message> + Send + use<> {
    let events = backend.prompt_events();
    stream::unfold(events, |events| async move {
        let mut events = events?;
        let event = events.recv().await?;
        Some((Message::Prompt(event), Some(events)))
    })
}

/// Queues each transfer in order; stops at the first failure.
pub async fn enqueue_all(
    backend: Arc<dyn BackendApi>,
    specs: Vec<TransferSpec>,
) -> Result<(), Error> {
    for spec in specs {
        backend.enqueue(spec).await?;
    }
    Ok(())
}

/// The engine's events for the queue's subscription. It subscribes at once, then starts
/// with [`Message::QueueResync`], so the snapshot it leads to is taken after the
/// subscription (nothing falls between the two).
pub fn transfer_feed(backend: &dyn BackendApi) -> impl Stream<Item = Message> + Send + use<> {
    engine_messages(backend.transfer_events())
}

/// Each event as [`Message::TransferEvent`], after a first [`Message::QueueResync`]. A
/// receiver that lagged (it lost events) gives another `QueueResync`; the stream ends with
/// the engine.
pub fn engine_messages(
    events: broadcast::Receiver<Event>,
) -> impl Stream<Item = Message> + Send + 'static {
    let events = stream::unfold(events, |mut events| async move {
        match events.recv().await {
            Ok(event) => Some((Message::TransferEvent(event), events)),
            Err(broadcast::error::RecvError::Lagged(lost)) => {
                tracing::warn!(
                    lost,
                    "the queue lagged behind the transfer engine; resyncing"
                );
                Some((Message::QueueResync, events))
            }
            Err(broadcast::error::RecvError::Closed) => None,
        }
    });
    stream::once(std::future::ready(Message::QueueResync)).chain(events)
}

/// Runs one file operation: a delete's progress as it goes, then
/// [`Message::FileOpDone`] (last).
pub fn file_op(
    backend: Arc<dyn BackendApi>,
    tab: TabId,
    side: Side,
    op: RequestId,
    session: Option<SessionId>,
    action: FileAction,
) -> impl Stream<Item = Message> + Send + 'static {
    let (tx, rx) = mpsc::unbounded_channel();
    let done = async move {
        let result = match action {
            FileAction::Mkdir(path) => backend.mkdir(session, side, path).await,
            FileAction::Rename { from, to } => backend.rename(session, side, from, to).await,
            FileAction::Delete(targets) => {
                backend.delete(session, side, targets, tx).await.map(|_| ())
            }
            FileAction::Chmod { paths, mode } => {
                chmod_all(&*backend, session, side, paths, mode).await
            }
        };
        Message::FileOpDone {
            tab,
            side,
            op,
            result,
        }
    };
    // The progress sender lives in the operation, so the progress stream ends with it,
    // before the final message.
    let progress = stream::unfold(rx, move |mut rx| async move {
        rx.recv().await.map(|done| {
            (
                Message::FileOpProgress {
                    tab,
                    side,
                    op,
                    done,
                },
                rx,
            )
        })
    });
    let (result_tx, result_rx) = tokio::sync::oneshot::channel();
    let driver = stream::once(async move {
        let _ = result_tx.send(done.await);
    })
    .filter_map(|()| std::future::ready(None));
    let last = stream::once(result_rx).filter_map(|r| std::future::ready(r.ok()));
    stream::select(driver, progress.chain(last))
}

/// The same mode on each path, stopping at the first failure.
async fn chmod_all(
    backend: &dyn BackendApi,
    session: Option<SessionId>,
    side: Side,
    paths: Vec<RemotePath>,
    mode: u32,
) -> Result<(), Error> {
    for path in paths {
        backend.chmod(session, side, path, mode).await?;
    }
    Ok(())
}

/// Saves the site, then stores or forgets its password (the site must exist first).
pub async fn save_site(
    backend: Arc<dyn BackendApi>,
    site: Site,
    password: PasswordChange,
) -> Result<(), Error> {
    let id = site.id.clone();
    backend.save_site(site).await?;
    match password {
        PasswordChange::Keep => Ok(()),
        PasswordChange::Set(password) => backend.set_password(id, Some(password)).await,
        PasswordChange::Forget => backend.set_password(id, None).await,
    }
}

/// Runs one listing to its end and tags the result with its request.
pub async fn list(
    backend: Arc<dyn BackendApi>,
    tab: TabId,
    side: Side,
    request_id: RequestId,
    session: Option<SessionId>,
    path: RemotePath,
) -> Message {
    let result = backend.list(session, side, path).await;
    Message::Listed {
        tab,
        side,
        request_id,
        result,
    }
}

/// Open workspace: one [`Message::SiteConnected`] per site, each as soon as that site is
/// done. The backend reads the secrets once for the whole batch.
pub fn connect_sites(
    backend: Arc<dyn BackendApi>,
    sites: Vec<SiteId>,
) -> impl Stream<Item = Message> + Send + 'static {
    let (tx, rx) = mpsc::unbounded_channel();
    // Drives the batch; yields nothing itself. The channel closes when it returns.
    let driver = stream::once(async move { backend.connect_batch(sites, tx).await })
        .filter_map(|()| std::future::ready(None));
    let results = stream::unfold(rx, |mut rx| async move {
        rx.recv()
            .await
            .map(|(site, result)| (Message::SiteConnected { site, result }, rx))
    });
    stream::select(driver, results)
}

/// Opens `dir` with the system file manager, off the UI thread.
async fn open_folder(dir: PathBuf) -> Result<(), Error> {
    tokio::task::spawn_blocking(move || open_with_system(&dir))
        .await
        .map_err(|e| Error::new(ErrorKind::Io, "Opener task failed").with_detail(e.to_string()))?
}

fn open_with_system(dir: &Path) -> Result<(), Error> {
    #[cfg(target_os = "windows")]
    const OPENER: &str = "explorer";
    #[cfg(target_os = "macos")]
    const OPENER: &str = "open";
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    const OPENER: &str = "xdg-open";

    let status = std::process::Command::new(OPENER).arg(dir).status()?;
    // Explorer exits with 1 even when it opened the folder.
    if status.success() || cfg!(target_os = "windows") {
        Ok(())
    } else {
        Err(Error::new(ErrorKind::Io, format!("{OPENER} failed")).with_detail(status.to_string()))
    }
}
