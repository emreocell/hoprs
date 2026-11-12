//! FTP control connection: connect, greeting, login, `TYPE I`, `FEAT`.

use hoprs_core::{Error, ErrorKind, Result};
use secrecy::{ExposeSecret, SecretString};
use suppaftp::FtpError;
use suppaftp::Status;
use suppaftp::tokio::AsyncRustlsFtpStream;
use suppaftp::types::FileType;

use super::config::{FtpConfig, FtpSecurity};
use super::errors::{map_ftp_error, reply_parts};
use super::prompt::FtpPrompter;

/// The suppaftp stream type for every security mode. The rustls variant also speaks plain
/// FTP, and `into_secure` (explicit FTPS) keeps the same type.
pub(crate) type FtpStream = AsyncRustlsFtpStream;

/// A logged-in control connection in binary mode.
pub(crate) struct Control {
    pub(crate) stream: FtpStream,
    /// `FEAT` lists `MFMT`.
    pub(crate) has_mfmt: bool,
    /// `FEAT` lists `MLST` (servers announce `MLSD` support this way).
    pub(crate) has_mlsd: bool,
    /// `FEAT` lists `UTF8` (and `OPTS UTF8 ON` was sent).
    pub(crate) has_utf8: bool,
}

/// Opens a control connection. Only [`FtpSecurity::Plain`] so far.
///
/// When `cfg.password` is `None` the password is asked *before* connecting, so the prompt
/// never counts against `cfg.connect_timeout`, which bounds TCP connect, greeting, login,
/// `TYPE I` and `FEAT`.
pub(crate) async fn open_control(cfg: &FtpConfig, prompter: &dyn FtpPrompter) -> Result<Control> {
    let password = match &cfg.password {
        Some(p) => p.clone(),
        None => prompter
            .password(&cfg.user, &cfg.host)
            .await
            .ok_or_else(|| Error::new(ErrorKind::Auth, "Cancelled by user"))?,
    };
    if cfg.security != FtpSecurity::Plain {
        return Err(Error::new(
            ErrorKind::Tls,
            format!("{} is not supported yet", cfg.security.label()),
        ));
    }
    tokio::time::timeout(cfg.connect_timeout, connect_and_login(cfg, &password))
        .await
        .unwrap_or_else(|_| Err(Error::new(ErrorKind::Network, "Connection timed out")))
}

async fn connect_and_login(cfg: &FtpConfig, password: &SecretString) -> Result<Control> {
    let target = format!("{}:{}", cfg.host, cfg.port);
    // Connects and reads the greeting.
    let mut stream = FtpStream::connect((cfg.host.as_str(), cfg.port))
        .await
        .map_err(|e| match e {
            FtpError::ConnectionError(e) => Error::new(ErrorKind::Network, "Connection failed")
                .with_detail(format!("{target}: {e}")),
            e => map_ftp_error(e, &target),
        })?;

    let who = format!("{}@{target}", cfg.user);
    stream
        .login(cfg.user.as_str(), password.expose_secret())
        .await
        .map_err(|e| map_ftp_error(e, &who))?;
    stream
        .transfer_type(FileType::Binary)
        .await
        .map_err(|e| map_ftp_error(e, &who))?;

    // A server without FEAT (a negative reply) just has no optional features.
    let features = match stream.feat().await {
        Ok(f) => f,
        Err(FtpError::UnexpectedResponse(_)) => Default::default(),
        Err(e) => return Err(map_ftp_error(e, &who)),
    };
    let has = |name: &str| features.keys().any(|k| k.eq_ignore_ascii_case(name));
    let (has_mfmt, has_mlsd, has_utf8) = (has("MFMT"), has("MLST"), has("UTF8"));

    if has_utf8 {
        // Best effort: servers that are always UTF-8 may refuse it.
        match stream
            .custom_command("OPTS UTF8 ON", &[Status::CommandOk])
            .await
        {
            Ok(_) | Err(FtpError::UnexpectedResponse(_)) => {}
            Err(e) => return Err(map_ftp_error(e, &who)),
        }
    }

    Ok(Control {
        stream,
        has_mfmt,
        has_mlsd,
        has_utf8,
    })
}

/// Sends a raw command and returns the reply's code and text, whatever the code.
pub(crate) async fn quote(stream: &mut FtpStream, command: &str) -> Result<(u32, String)> {
    match stream.custom_command(command, &[]).await {
        Ok(reply) | Err(FtpError::UnexpectedResponse(reply)) => Ok(reply_parts(&reply)),
        Err(e) => Err(map_ftp_error(e, "raw command")),
    }
}
