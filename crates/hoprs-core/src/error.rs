/// Category of a [`Error`], used to decide how the caller reacts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorKind {
    Network,
    Auth,
    Permission,
    Conflict,
    Protocol,
    HostKey,
    Tls,
    Io,
    NotFound,
}

impl ErrorKind {
    /// Stable snake_case name, used for persistence.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Network => "network",
            Self::Auth => "auth",
            Self::Permission => "permission",
            Self::Conflict => "conflict",
            Self::Protocol => "protocol",
            Self::HostKey => "host_key",
            Self::Tls => "tls",
            Self::Io => "io",
            Self::NotFound => "not_found",
        }
    }

    /// Inverse of [`ErrorKind::as_str`].
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "network" => Self::Network,
            "auth" => Self::Auth,
            "permission" => Self::Permission,
            "conflict" => Self::Conflict,
            "protocol" => Self::Protocol,
            "host_key" => Self::HostKey,
            "tls" => Self::Tls,
            "io" => Self::Io,
            "not_found" => Self::NotFound,
            _ => return None,
        })
    }
}

/// The single error type of the workspace: a category, a human-readable
/// message and an optional technical detail for logs.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{message}")]
pub struct Error {
    pub kind: ErrorKind,
    pub message: String,
    pub detail: Option<String>,
    /// Stable, kebab-case key of the message (the app's catalog looks up
    /// `error-<code>`); `None` for errors that have no translation.
    pub code: Option<&'static str>,
    /// Named values of the message, for the translation (`host`, `path`, ...).
    pub args: Vec<(&'static str, String)>,
}

impl Error {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            detail: None,
            code: None,
            args: Vec::new(),
        }
    }

    /// An error with a stable `code`; `message` is its English text (logs,
    /// `Display` and the fallback).
    pub fn coded(kind: ErrorKind, code: &'static str, message: impl Into<String>) -> Self {
        let mut err = Self::new(kind, message);
        err.code = Some(code);
        err
    }

    /// Adds a named value for the translation.
    pub fn arg(mut self, name: &'static str, value: impl Into<String>) -> Self {
        self.args.push((name, value.into()));
        self
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        use std::io::ErrorKind as K;
        let kind = match err.kind() {
            K::NotFound => ErrorKind::NotFound,
            K::PermissionDenied => ErrorKind::Permission,
            K::ConnectionReset | K::ConnectionAborted | K::BrokenPipe | K::TimedOut => {
                ErrorKind::Network
            }
            _ => ErrorKind::Io,
        };
        Self::new(kind, err.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn io_mapping() {
        use std::io::{Error as Io, ErrorKind as K};
        assert_eq!(Error::from(Io::from(K::NotFound)).kind, ErrorKind::NotFound);
        assert_eq!(
            Error::from(Io::from(K::PermissionDenied)).kind,
            ErrorKind::Permission
        );
        assert_eq!(
            Error::from(Io::from(K::ConnectionReset)).kind,
            ErrorKind::Network
        );
        assert_eq!(Error::from(Io::from(K::InvalidData)).kind, ErrorKind::Io);
    }
    #[test]
    fn kind_roundtrip() {
        for k in [
            ErrorKind::Network,
            ErrorKind::Auth,
            ErrorKind::Permission,
            ErrorKind::Conflict,
            ErrorKind::Protocol,
            ErrorKind::HostKey,
            ErrorKind::Tls,
            ErrorKind::Io,
            ErrorKind::NotFound,
        ] {
            assert_eq!(ErrorKind::parse(k.as_str()), Some(k));
        }
    }
    #[test]
    fn display_is_message_only() {
        let e =
            Error::new(ErrorKind::Auth, "Wrong password").with_detail("SSH_MSG_USERAUTH_FAILURE");
        assert_eq!(e.to_string(), "Wrong password");
        assert_eq!(e.detail.as_deref(), Some("SSH_MSG_USERAUTH_FAILURE"));
    }
    #[test]
    fn coded_error_keeps_message_and_display() {
        let e =
            Error::coded(ErrorKind::Auth, "auth-wrong-password", "Wrong password").arg("host", "h");
        assert_eq!(e.to_string(), "Wrong password");
        assert_eq!(e.code, Some("auth-wrong-password"));
        assert_eq!(e.args, vec![("host", "h".to_owned())]);
        let plain = Error::new(ErrorKind::Auth, "x");
        assert!(plain.code.is_none() && plain.args.is_empty());
    }
}
