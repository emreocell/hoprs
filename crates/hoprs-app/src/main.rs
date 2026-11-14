//! hoprs: desktop SSH + SFTP + FTP/FTPS client. Everything lives in the library.

// Release builds on Windows use the GUI subsystem: no console window behind the app.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use hoprs_app::cli::{self, Cli};

fn main() -> iced::Result {
    // Handled before `run()`: no instance lock, no logging, no window.
    match cli::parse_args(std::env::args().skip(1)) {
        Cli::Run => hoprs_app::run(),
        Cli::Version => {
            attach_parent_console();
            println!("{}", cli::version_line());
            Ok(())
        }
        Cli::Help => {
            attach_parent_console();
            println!("{}", cli::usage());
            Ok(())
        }
        Cli::Unknown(arg) => {
            attach_parent_console();
            eprintln!("hoprs: unknown argument '{arg}'\n{}", cli::usage());
            std::process::exit(2);
        }
    }
}

/// A GUI-subsystem process starts without a console, so `--version` would print nowhere.
/// Attach to the parent's console (the terminal that launched us) before printing.
/// Failure is ignored: when double-clicked there is no parent console and nothing to print to.
#[cfg(windows)]
fn attach_parent_console() {
    use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
    // SAFETY: plain Win32 call with no pointers; failure is harmless.
    unsafe {
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

#[cfg(not(windows))]
fn attach_parent_console() {}
