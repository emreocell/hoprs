//! Turns each [`Effect`] into an [`iced::Task`]: the only place that calls
//! the backend.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures_util::{Stream, StreamExt, stream};
use hoprs_core::{Error, ErrorKind, RemotePath, Site, SiteId};
use hoprs_transfer::{Event, TransferSpec};
use tokio::sync::{broadcast, mpsc};

use crate::app::{Effect, FileAction, Message, ProfileOp, RequestId, TabId};
use crate::backend::{BackendApi, SessionId, Side};
use crate::ui::files;
use crate::ui::site_form::PasswordChange;

/// Runs one effect against the backend.
pub fn run(backend: Arc<dyn BackendApi>, effect: Effect) -> iced::Task<Message> {
    use iced::Task;
    match effect {
        Effect::LoadProfiles => Task::perform(
            async move { backend.profiles().await },
            Message::ProfilesLoaded,
        ),
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
    }
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
