//! hoprs: desktop SSH + SFTP + FTP/FTPS client. Everything lives in the library.

use hoprs_app::cli::{self, Cli};

fn main() -> iced::Result {
    // Handled before `run()`: no instance lock, no logging, no window.
    match cli::parse_args(std::env::args().skip(1)) {
        Cli::Run => hoprs_app::run(),
        Cli::Version => {
            println!("{}", cli::version_line());
            Ok(())
        }
        Cli::Help => {
            println!("{}", cli::usage());
            Ok(())
        }
        Cli::Unknown(arg) => {
            eprintln!("hoprs: unknown argument '{arg}'\n{}", cli::usage());
            std::process::exit(2);
        }
    }
}
