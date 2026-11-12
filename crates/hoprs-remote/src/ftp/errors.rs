//! FTP replies and suppaftp errors → [`hoprs_core::Error`].

use hoprs_core::{Error, ErrorKind};
use suppaftp::FtpError;
use suppaftp::types::Response;

/// Maps a negative (or unexpected) FTP reply. `context` names what was being done (a path,
/// `user@host`, ...). `450`/`550` are classified by their text; the caller may still
/// disambiguate with a `stat`.
pub(crate) fn map_reply(code: u32, text: &str, context: &str) -> Error {
    let lower = text.to_lowercase();
    let (kind, what) = match code {
        530 => (ErrorKind::Auth, "Login failed"),
        421 => (ErrorKind::Network, "Server closed the connection"),
        425 => (ErrorKind::Network, "Cannot open data connection"),
        426 => (ErrorKind::Network, "Transfer aborted"),
        450 | 550 if lower.contains("no such file") || lower.contains("not found") => {
            (ErrorKind::NotFound, "No such file or directory")
        }
        450 | 550 if lower.contains("denied") || lower.contains("permission") => {
            (ErrorKind::Permission, "Permission denied")
        }
        450 | 550 => (ErrorKind::Io, "File unavailable"),
        452 | 552 => (ErrorKind::Io, "Server storage full or quota exceeded"),
        553 => (ErrorKind::Io, "File name not allowed"),
        _ => (ErrorKind::Protocol, "Unexpected FTP reply"),
    };
    let detail = match code {
        425 | 426 => format!("{DATA_CHANNEL}{code} {text}"),
        _ => format!("{code} {text}"),
    };
    Error::new(kind, format!("{what}: {context}")).with_detail(detail)
}

/// Detail prefix of the `Network` errors for data channel replies (`425`, `426`): they come
/// over a healthy control connection.
const DATA_CHANNEL: &str = "data channel: ";

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

/// Maps any suppaftp error. Replies go through [`map_reply`].
pub(crate) fn map_ftp_error(err: FtpError, context: &str) -> Error {
    match err {
        FtpError::UnexpectedResponse(reply) => {
            let (code, text) = reply_parts(&reply);
            map_reply(code, &text, context)
        }
        FtpError::ConnectionError(e) => {
            Error::new(ErrorKind::Network, format!("Connection lost: {context}"))
                .with_detail(e.to_string())
        }
        FtpError::SecureError(e) => {
            Error::new(ErrorKind::Tls, format!("TLS failed: {context}")).with_detail(e)
        }
        e @ (FtpError::BadResponse
        | FtpError::InvalidAddress(_)
        | FtpError::DataConnectionAlreadyOpen) => Error::new(
            ErrorKind::Protocol,
            format!("Unexpected FTP reply: {context}"),
        )
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
    fn other_codes_are_protocol() {
        for code in [400, 451, 500, 501, 502, 503, 504, 532, 551, 599] {
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
}
