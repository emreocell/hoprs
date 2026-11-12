//! FTP control connection: connect, TLS (FTPS), greeting, login, `TYPE I`, `FEAT`.

use hoprs_core::{Error, ErrorKind, Result};
use secrecy::{ExposeSecret, SecretString};
use suppaftp::FtpError;
use suppaftp::Status;
use suppaftp::tokio::{AsyncRustlsConnector, AsyncRustlsFtpStream};
use suppaftp::types::FileType;

use super::config::{FtpConfig, FtpSecurity};
use super::errors::{map_ftp_error, reply_parts};
use super::prompt::FtpPrompter;
use super::tls::{RejectedCert, RejectedSlot, connector, fingerprint};

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

/// Opens a control connection in any [`FtpSecurity`] mode.
///
/// When `cfg.password` is `None` the password is asked *before* connecting.
/// `cfg.connect_timeout` bounds each attempt's TCP connect, TLS handshake, greeting, login,
/// `TYPE I` and `FEAT`; time spent in prompts never counts against it.
///
/// FTPS: a certificate that is neither trusted (system roots plus `cfg.extra_roots`, valid
/// for `cfg.host`) nor equal to `cfg.pinned_fingerprint` goes to
/// [`FtpPrompter::confirm_certificate`]. If the user accepts, the fingerprint is stored with
/// [`FtpPrompter::store_pin`], written to `cfg.pinned_fingerprint`, and the connection is
/// opened again with that pin, once and without asking again. If the user refuses, the
/// result is a `Tls` error "Certificate not trusted" with the fingerprint in the detail.
pub(crate) async fn open_control(
    cfg: &mut FtpConfig,
    prompter: &dyn FtpPrompter,
) -> Result<Control> {
    let password = match &cfg.password {
        Some(p) => p.clone(),
        None => prompter
            .password(&cfg.user, &cfg.host)
            .await
            .ok_or_else(|| Error::new(ErrorKind::Auth, "Cancelled by user"))?,
    };

    let rejected = match attempt(cfg, &password).await? {
        Attempt::Connected(control) => return Ok(control),
        Attempt::CertificateRejected(rejected) => rejected,
    };
    let fp = fingerprint(&rejected.der);
    if !prompter
        .confirm_certificate(&cfg.host, cfg.port, &fp, &rejected.problem)
        .await
    {
        return Err(not_trusted(&fp));
    }
    prompter.store_pin(&cfg.host, cfg.port, &fp).await;
    cfg.pinned_fingerprint = Some(fp);
    match attempt(cfg, &password).await? {
        Attempt::Connected(control) => Ok(control),
        // The server switched certificates between the two connections.
        Attempt::CertificateRejected(rejected) => Err(not_trusted(&fingerprint(&rejected.der))),
    }
}

fn not_trusted(fp: &str) -> Error {
    Error::new(ErrorKind::Tls, "Certificate not trusted").with_detail(fp.to_owned())
}

enum Attempt {
    Connected(Control),
    /// The TLS handshake failed on this certificate, which needs the user's decision.
    CertificateRejected(RejectedCert),
}

/// One connection attempt, bounded by `cfg.connect_timeout`.
async fn attempt(cfg: &FtpConfig, password: &SecretString) -> Result<Attempt> {
    let target = format!("{}:{}", cfg.host, cfg.port);
    let slot = RejectedSlot::default();
    let tls = match cfg.security {
        FtpSecurity::Plain => None,
        FtpSecurity::ExplicitTls | FtpSecurity::ImplicitTls => Some(connector(cfg, slot.clone())?),
    };
    let result = tokio::time::timeout(cfg.connect_timeout, connect_and_login(cfg, password, tls))
        .await
        .unwrap_or_else(|_| {
            Err(Error::new(ErrorKind::Network, "Connection timed out").with_detail(target))
        });
    match result {
        Ok(control) => Ok(Attempt::Connected(control)),
        // The verifier fills the slot only when it refuses a certificate, which aborts the
        // handshake; any other TLS failure leaves it empty and is returned as is.
        Err(e) if e.kind == ErrorKind::Tls => {
            match slot.lock().unwrap_or_else(|p| p.into_inner()).take() {
                Some(rejected) => Ok(Attempt::CertificateRejected(rejected)),
                None => Err(e),
            }
        }
        Err(e) => Err(e),
    }
}

async fn connect_and_login(
    cfg: &FtpConfig,
    password: &SecretString,
    tls: Option<AsyncRustlsConnector>,
) -> Result<Control> {
    let target = format!("{}:{}", cfg.host, cfg.port);
    let addr = (cfg.host.as_str(), cfg.port);
    let connect_error = |e| match e {
        FtpError::ConnectionError(e) => Error::new(ErrorKind::Network, "Connection failed")
            .with_detail(format!("{target}: {e}")),
        e => map_ftp_error(e, &target),
    };
    // A refused `AUTH TLS`, `PBSZ` or `PROT` means the server does not offer FTPS.
    let refused = |e| match e {
        FtpError::UnexpectedResponse(reply) => {
            let (code, text) = reply_parts(&reply);
            Error::new(ErrorKind::Tls, format!("Server refused FTPS: {target}"))
                .with_detail(format!("{code} {text}"))
        }
        e => connect_error(e),
    };
    // Connects and reads the greeting. The TLS server name is `cfg.host`, a DNS name or an
    // IP address; both are checked against the certificate.
    let mut stream = match (cfg.security, tls) {
        (FtpSecurity::ImplicitTls, Some(tls)) => {
            let mut stream = FtpStream::connect_secure_implicit(addr, tls, &cfg.host)
                .await
                .map_err(connect_error)?;
            // Unlike `into_secure`, suppaftp does not protect the data channel here: without
            // `PROT P` data connections would be plain (and refused by most servers).
            for command in ["PBSZ 0", "PROT P"] {
                stream
                    .custom_command(command, &[Status::CommandOk])
                    .await
                    .map_err(refused)?;
            }
            stream
        }
        (FtpSecurity::ExplicitTls, Some(tls)) => {
            let stream = FtpStream::connect(addr).await.map_err(connect_error)?;
            // suppaftp sends `AUTH TLS`, does the handshake, then `PBSZ 0` and `PROT P`
            // right away, *before* login (RFC 4217 allows either order).
            stream.into_secure(tls, &cfg.host).await.map_err(refused)?
        }
        (FtpSecurity::Plain, _) => FtpStream::connect(addr).await.map_err(connect_error)?,
        // Never fall back to plain FTP when FTPS was asked for.
        (FtpSecurity::ExplicitTls | FtpSecurity::ImplicitTls, None) => {
            return Err(Error::new(ErrorKind::Tls, "FTPS needs a TLS configuration"));
        }
    };

    let who = format!("{}@{target}", cfg.user);
    // suppaftp logs every command at trace level, `PASS <password>` included: a logger must
    // keep the `suppaftp` target below trace (see the Plan 3 follow-ups).
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
