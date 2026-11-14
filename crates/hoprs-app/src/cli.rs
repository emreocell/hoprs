//! Command-line arguments. Parsed before anything else starts, so `--version` and
//! `--help` never take the instance lock or open a window.

/// What the command line asks for.
#[derive(Debug, PartialEq, Eq)]
pub enum Cli {
    /// No arguments: start the app.
    Run,
    /// `--version` / `-V`.
    Version,
    /// `--help` / `-h`.
    Help,
    /// Anything else; carries the first offending argument.
    Unknown(String),
}

/// Parses the arguments after the program name.
pub fn parse_args<I, S>(args: I) -> Cli
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut args = args.into_iter();
    match args.next() {
        None => Cli::Run,
        Some(first) => match first.as_ref() {
            "--version" | "-V" => Cli::Version,
            "--help" | "-h" => Cli::Help,
            other => Cli::Unknown(other.to_owned()),
        },
    }
}

/// The `--version` line.
pub fn version_line() -> String {
    format!("hoprs {}", env!("CARGO_PKG_VERSION"))
}

/// The `--help` text.
pub fn usage() -> &'static str {
    "Usage: hoprs [--version | --help]"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_args_version_help_unknown() {
        assert_eq!(parse_args(Vec::<String>::new()), Cli::Run);
        assert_eq!(parse_args(["--version"]), Cli::Version);
        assert_eq!(parse_args(["-V"]), Cli::Version);
        assert_eq!(parse_args(["--help"]), Cli::Help);
        assert_eq!(parse_args(["-h"]), Cli::Help);
        assert_eq!(parse_args(["--bogus"]), Cli::Unknown("--bogus".into()));
        assert_eq!(parse_args(["-V", "x"]), Cli::Version);
    }
}
