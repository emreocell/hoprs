//! Turns each [`Effect`] into an [`iced::Task`]: the only place that calls
//! the backend.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures_util::{Stream, StreamExt, stream};
use hoprs_core::{Error, ErrorKind, Site, SiteId};
use tokio::sync::mpsc;

use crate::app::{Effect, Message, ProfileOp};
use crate::backend::BackendApi;
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
    }
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
