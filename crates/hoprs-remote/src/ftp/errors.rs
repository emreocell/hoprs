//! FTP replies and suppaftp errors → [`hoprs_core::Error`].

use std::fmt;
use std::net::SocketAddr;

use hoprs_core::{Error, ErrorKind};
use suppaftp::FtpError;
use suppaftp::types::Response;

/// Maps a negative (or unexpected) FTP reply. `context` names what was being done (a path,
/// `user@host`, ...). `450`/`550` are classified by their text; the caller may still
/// disambiguate with a `stat`.
pub(crate) fn map_reply(code: u32, text: &str, context: &str) -> Error {
    let lower = text.to_lowercase();
    let context_arg = context.to_string();
    let error = match code {
        530 => Error::coded(
            ErrorKind::Auth,
            "ftp-login-failed",
            format!("Login failed: {context}"),
        )
        .arg("context", context_arg),
        421 => Error::coded(
            ErrorKind::Network,
            "ftp-server-closed",
            format!("Server closed the connection: {context}"),
        )
        .arg("context", context_arg),
        425 => Error::coded(
            ErrorKind::Network,
            "ftp-cannot-open-data-connection",
            format!("Cannot open data connection: {context}"),
        )
        .arg("context", context_arg),
        426 => Error::coded(
            ErrorKind::Network,
            "ftp-transfer-aborted",
            format!("Transfer aborted: {context}"),
        )
        .arg("context", context_arg),
        450 | 550 if lower.contains("no such file") || lower.contains("not found") => Error::coded(
            ErrorKind::NotFound,
            "ftp-reply-no-such-file",
            format!("No such file or directory: {context}"),
        )
        .arg("context", context_arg),
        450 | 550 if lower.contains("denied") || lower.contains("permission") => Error::coded(
            ErrorKind::Permission,
            "ftp-reply-permission-denied",
            format!("Permission denied: {context}"),
        )
        .arg("context", context_arg),
        450 | 550 => Error::coded(
            ErrorKind::Io,
            "ftp-file-unavailable",
            format!("File unavailable: {context}"),
        )
        .arg("context", context_arg),
        // vsftpd: "Failure writing to local file." (a full disk among others).
        451 => Error::coded(
            ErrorKind::Io,
            "ftp-server-failed-file",
            format!("Server failed to process the file: {context}"),
        )
        .arg("context", context_arg),
        452 | 552 => Error::coded(
            ErrorKind::Io,
            "ftp-storage-full",
            format!("Server storage full or quota exceeded: {context}"),
        )
        .arg("context", context_arg),
        553 => Error::coded(
            ErrorKind::Io,
            "ftp-name-not-allowed",
            format!("File name not allowed: {context}"),
        )
        .arg("context", context_arg),
        _ => Error::coded(
            ErrorKind::Protocol,
            "ftp-unexpected-reply",
            format!("Unexpected FTP reply: {context}"),
        )
        .arg("context", context_arg),
    };
    let detail = match code {
        425 | 426 => format!("{DATA_CHANNEL}{code} {text}"),
        _ => format!("{code} {text}"),
    };
    error.with_detail(detail)
}

/// Detail prefix of the `Network` errors for data channel replies (`425`, `426`): they come
/// over a healthy control connection.
const DATA_CHANNEL: &str = "data channel: ";

/// A transfer's data connection failed while its control connection still answers: `Network`,
/// but not a control loss.
pub(crate) fn data_channel_lost(context: &str, detail: impl fmt::Display) -> Error {
    Error::coded(
        ErrorKind::Network,
        "ftp-connection-lost",
        format!("Connection lost: {context}"),
    )
    .arg("context", context.to_string())
    .with_detail(format!("{DATA_CHANNEL}{detail}"))
}

/// Whether `e`, returned by a command, means the connection it ran on is lost: a socket or
/// I/O failure, a timeout, or `421`. Data channel replies (`425`, `426`) are `Network` errors
/// too, but the control connection that carried them is fine.
pub(crate) fn is_control_loss(e: &Error) -> bool {
    e.kind == ErrorKind::Network
        && !e
            .detail
            .as_deref()
            .is_some_and(|d| d.starts_with(DATA_CHANNEL))
}

/// Why a passive data connection could not be opened. Carried inside the
/// `FtpError::ConnectionError` of [`data_connect_failed`] so that [`map_ftp_error`] can tell it
/// from a failure of the control connection.
#[derive(Debug)]
struct DataConnectFailed {
    addr: SocketAddr,
    source: std::io::Error,
}

impl fmt::Display for DataConnectFailed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "connect to {}: {}", self.addr, self.source)
    }
}

impl std::error::Error for DataConnectFailed {}

/// The error for a passive data connection to `addr` that failed with `source`. suppaftp
/// connects it after reading the `PASV` reply and before sending the transfer command, so the
/// control connection is in sync: it maps to a data channel error, not a control loss.
pub(crate) fn data_connect_failed(addr: SocketAddr, source: std::io::Error) -> FtpError {
    let kind = source.kind();
    FtpError::ConnectionError(std::io::Error::new(
        kind,
        DataConnectFailed { addr, source },
    ))
}

/// Maps any suppaftp error. Replies go through [`map_reply`].
pub(crate) fn map_ftp_error(err: FtpError, context: &str) -> Error {
    match err {
        FtpError::UnexpectedResponse(reply) => {
            let (code, text) = reply_parts(&reply);
            map_reply(code, &text, context)
        }
        FtpError::ConnectionError(e)
            if e.get_ref()
                .is_some_and(|inner| inner.is::<DataConnectFailed>()) =>
        {
            Error::coded(
                ErrorKind::Network,
                "ftp-cannot-open-data-connection",
                format!("Cannot open data connection: {context}"),
            )
            .arg("context", context.to_string())
            .with_detail(format!(
                "{DATA_CHANNEL}{}",
                e.get_ref().map(|i| i.to_string()).unwrap_or_default()
            ))
        }
        FtpError::ConnectionError(e) => Error::coded(
            ErrorKind::Network,
            "ftp-connection-lost",
            format!("Connection lost: {context}"),
        )
        .arg("context", context.to_string())
        .with_detail(e.to_string()),
        FtpError::SecureError(e) => Error::coded(
            ErrorKind::Tls,
            "tls-failed",
            format!("TLS failed: {context}"),
        )
        .arg("context", context.to_string())
        .with_detail(e),
        e @ (FtpError::BadResponse
        | FtpError::InvalidAddress(_)
        | FtpError::DataConnectionAlreadyOpen) => Error::coded(
            ErrorKind::Protocol,
            "ftp-unexpected-reply",
            format!("Unexpected FTP reply: {context}"),
        )
        .arg("context", context.to_string())
        .with_detail(e.to_string()),
    }
}

/// Code and text of a reply. The code is read from the text: suppaftp maps codes it does not
/// know to `Status::Unknown`. Multi-line replies keep their continuation lines.
pub(crate) fn reply_parts(reply: &Response) -> (u32, String) {
    let body = String::from_utf8_lossy(&reply.body);
    let body = body.trim_end();
    let code = body
        .get(..3)
        .and_then(|c| c.parse().ok())
        .unwrap_or_else(|| reply.status.code());
    let text = match body.get(..3).map(|c| c.parse::<u32>().is_ok()) {
        Some(true) => body.get(4..).unwrap_or(""),
        _ => body,
    };
    (code, text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use suppaftp::Status;

    fn kind(code: u32, text: &str) -> ErrorKind {
        map_reply(code, text, "/a").kind
    }

    #[test]
    fn auth() {
        assert_eq!(kind(530, "Login incorrect."), ErrorKind::Auth);
    }

    #[test]
    fn network() {
        for code in [421, 425, 426] {
            assert_eq!(kind(code, "x"), ErrorKind::Network, "{code}");
        }
    }

    #[test]
    fn data_channel_replies_are_not_control_loss() {
        for code in [425, 426] {
            let e = map_reply(code, "Can't open data connection.", "/a");
            assert_eq!(e.kind, ErrorKind::Network, "{code}");
            assert!(!is_control_loss(&e), "{e:?}");
            let detail = e.detail.as_deref().unwrap_or("");
            assert!(detail.contains(&format!("{code} Can't open")), "{e:?}");
        }
        assert!(is_control_loss(&map_reply(421, "Timeout.", "/a")));
        let io = std::io::Error::from(std::io::ErrorKind::ConnectionReset);
        assert!(is_control_loss(&map_ftp_error(
            FtpError::ConnectionError(io),
            "h"
        )));
        assert!(is_control_loss(&Error::new(
            ErrorKind::Network,
            "Control command timed out"
        )));
        for e in [
            map_reply(550, "No such file or directory", "/a"),
            map_reply(530, "Login incorrect.", "/a"),
            map_reply(500, "Unknown command.", "/a"),
        ] {
            assert!(!is_control_loss(&e), "{e:?}");
        }
    }

    #[test]
    fn failed_data_connect_is_not_control_loss() {
        let addr: SocketAddr = "127.0.0.1:30001".parse().unwrap();
        let io = std::io::Error::from(std::io::ErrorKind::ConnectionRefused);
        let e = map_ftp_error(data_connect_failed(addr, io), "/a");
        assert_eq!(e.kind, ErrorKind::Network, "{e:?}");
        assert!(!is_control_loss(&e), "{e:?}");
        assert!(e.message.contains("/a"), "{e:?}");
        let detail = e.detail.as_deref().unwrap_or("");
        assert!(detail.starts_with("data channel: "), "{e:?}");
        assert!(detail.contains("127.0.0.1:30001"), "{e:?}");
        // A plain socket error still is.
        let io = std::io::Error::from(std::io::ErrorKind::ConnectionRefused);
        assert!(is_control_loss(&map_ftp_error(
            FtpError::ConnectionError(io),
            "/a"
        )));
    }

    #[test]
    fn file_unavailable_by_text() {
        for code in [450, 550] {
            assert_eq!(kind(code, "No such file or directory"), ErrorKind::NotFound);
            assert_eq!(kind(code, "File NOT FOUND"), ErrorKind::NotFound);
            assert_eq!(kind(code, "Permission denied."), ErrorKind::Permission);
            assert_eq!(kind(code, "Access DENIED"), ErrorKind::Permission);
            assert_eq!(kind(code, "No PERMISSION"), ErrorKind::Permission);
            assert_eq!(kind(code, "Failed to change directory."), ErrorKind::Io);
        }
    }

    #[test]
    fn storage_full() {
        for code in [452, 552] {
            let e = map_reply(code, "Disk full", "/a");
            assert_eq!(e.kind, ErrorKind::Io, "{code}");
            assert!(
                e.message
                    .starts_with("Server storage full or quota exceeded"),
                "{e:?}"
            );
        }
    }

    #[test]
    fn bad_filename_is_io() {
        assert_eq!(kind(553, "Could not create file."), ErrorKind::Io);
    }

    #[test]
    fn local_processing_error_is_io() {
        let e = map_reply(451, "Failure writing to local file.", "/a");
        assert_eq!(e.kind, ErrorKind::Io, "{e:?}");
        assert!(!is_control_loss(&e), "{e:?}");
    }

    #[test]
    fn other_codes_are_protocol() {
        for code in [400, 500, 501, 502, 503, 504, 532, 551, 599] {
            assert_eq!(kind(code, "x"), ErrorKind::Protocol, "{code}");
        }
    }

    #[test]
    fn detail_has_code_and_text() {
        let e = map_reply(550, "Permission denied.", "/a/b");
        assert_eq!(e.detail.as_deref(), Some("550 Permission denied."));
        assert!(e.message.contains("/a/b"), "{e:?}");
    }

    #[test]
    fn ftp_errors() {
        let io = std::io::Error::from(std::io::ErrorKind::ConnectionRefused);
        assert_eq!(
            map_ftp_error(FtpError::ConnectionError(io), "h").kind,
            ErrorKind::Network
        );
        assert_eq!(
            map_ftp_error(FtpError::SecureError("bad".into()), "h").kind,
            ErrorKind::Tls
        );
        assert_eq!(
            map_ftp_error(FtpError::BadResponse, "h").kind,
            ErrorKind::Protocol
        );
        // The code comes from the reply text, so codes suppaftp does not know survive.
        let reply = Response::new(Status::Unknown, b"599 Odd reply\r\n".to_vec());
        let e = map_ftp_error(FtpError::UnexpectedResponse(reply), "h");
        assert_eq!(e.kind, ErrorKind::Protocol);
        assert_eq!(e.detail.as_deref(), Some("599 Odd reply"));
        let reply = Response::new(Status::NotLoggedIn, b"530 Login incorrect.\r\n".to_vec());
        let e = map_ftp_error(FtpError::UnexpectedResponse(reply), "h");
        assert_eq!(e.kind, ErrorKind::Auth);
        assert_eq!(e.detail.as_deref(), Some("530 Login incorrect."));
    }

    #[test]
    fn replies_carry_a_code_and_the_context() {
        let login = map_reply(530, "Login incorrect.", "me@host");
        assert_eq!(login.code, Some("ftp-login-failed"));
        assert_eq!(login.args, vec![("context", "me@host".to_owned())]);
        assert_eq!(login.message, "Login failed: me@host");
        assert_eq!(
            map_reply(550, "Permission denied.", "/a").code,
            Some("ftp-reply-permission-denied")
        );
        assert_eq!(map_reply(999, "?", "/a").code, Some("ftp-unexpected-reply"));
    }

    #[test]
    fn a_refused_data_connection_is_coded() {
        let addr: std::net::SocketAddr = "127.0.0.1:9".parse().unwrap();
        let refused = std::io::Error::from(std::io::ErrorKind::ConnectionRefused);
        let e = map_ftp_error(data_connect_failed(addr, refused), "/a");
        assert_eq!(e.code, Some("ftp-cannot-open-data-connection"));
        assert_eq!(e.args, vec![("context", "/a".to_owned())]);
    }
}
