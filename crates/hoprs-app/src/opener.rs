//! Opens a folder, a file or a URL with the system's handler: `explorer` on Windows, `open`
//! on macOS, `xdg-open` elsewhere ("Open sites.toml folder", and the update's installer or
//! release page).

use std::ffi::{OsStr, OsString};

use hoprs_core::{Error, ErrorKind};

/// Opens `target` with the system handler, off the UI thread.
pub async fn open(target: OsString) -> Result<(), Error> {
    tokio::task::spawn_blocking(move || open_with_system(&target))
        .await
        .map_err(|e| {
            Error::coded(
                ErrorKind::Io,
                "app-opener-task-failed",
                "Opener task failed",
            )
            .with_detail(e.to_string())
        })?
}

fn open_with_system(target: &OsStr) -> Result<(), Error> {
    #[cfg(target_os = "windows")]
    const OPENER: &str = "explorer";
    #[cfg(target_os = "macos")]
    const OPENER: &str = "open";
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    const OPENER: &str = "xdg-open";

    let status = std::process::Command::new(OPENER).arg(target).status()?;
    // Explorer exits with 1 even when it opened the target.
    if status.success() || cfg!(target_os = "windows") {
        Ok(())
    } else {
        let opener = OPENER;
        Err(Error::coded(
            ErrorKind::Io,
            "app-opener-failed",
            format!("{opener} failed"),
        )
        .arg("opener", opener)
        .with_detail(status.to_string()))
    }
}
