//! FTP control connection against the containers in `docker/compose.yml`.

use std::time::{Duration, Instant};

use hoprs_core::{Error, ErrorKind};
use hoprs_remote::testing::{
    OtherCertificate, TestControl, ftp_config, ftps_fixture, kill_ftp_sessions, open_test_control,
    server_fingerprint,
};
use hoprs_remote::{CertProblem, FtpConfig, FtpSecurity, ScriptedFtpPrompter, cert_fingerprint};
use rustls::pki_types::CertificateDer;
use rustls::pki_types::pem::PemObject;
use secrecy::SecretString;
use tokio::net::TcpListener;

fn assert_err(result: Result<TestControl, Error>, kind: ErrorKind, message: &str) -> Error {
    let Err(e) = result else {
        panic!("expected {kind:?} error, got a connection");
    };
    assert_eq!(e.kind, kind, "{e:?}");
    assert!(e.message.contains(message), "{e:?}");
    e
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d --build --wait"]
async fn plain_login() {
    let p = ScriptedFtpPrompter::new();
    let mut c = open_test_control(&mut ftp_config(FtpSecurity::Plain), &p)
        .await
        .unwrap();
    // vsftpd 3.0.3 announces UTF8 but neither MFMT nor MLST.
    assert!(c.has_utf8());
    assert!(!c.has_mfmt());
    assert!(!c.has_mlsd());
    assert_eq!(c.quote("TYPE I").await.unwrap().0, 200);
    assert_eq!(c.quote("NOOP").await.unwrap().0, 200);
    let (code, text) = c.quote("PWD").await.unwrap();
    assert_eq!(code, 257);
    assert!(text.contains("/home/frank"), "{text}");
    assert_eq!(p.calls().password, 0);
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d --build --wait"]
async fn wrong_password_is_auth() {
    let mut cfg = ftp_config(FtpSecurity::Plain);
    cfg.password = Some(SecretString::from("not-franks-pass"));
    let p = ScriptedFtpPrompter::new();
    let e = assert_err(open_test_control(&mut cfg, &p).await, ErrorKind::Auth, "");
    let shown = format!("{e:?}");
    assert!(!shown.contains("not-franks-pass"), "{shown}");
    assert!(shown.contains("530"), "{shown}");
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d --build --wait"]
async fn password_prompted() {
    let mut cfg = ftp_config(FtpSecurity::Plain);
    cfg.password = None;
    let p = ScriptedFtpPrompter::new().with_password(Some("frank-pass"));
    let mut c = open_test_control(&mut cfg, &p).await.unwrap();
    assert_eq!(c.quote("NOOP").await.unwrap().0, 200);
    assert_eq!(p.calls().password, 1);
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d --build --wait"]
async fn password_cancelled_is_auth() {
    let mut cfg = ftp_config(FtpSecurity::Plain);
    cfg.password = None;
    let p = ScriptedFtpPrompter::new();
    assert_err(
        open_test_control(&mut cfg, &p).await,
        ErrorKind::Auth,
        "Cancelled by user",
    );
    assert_eq!(p.calls().password, 1);
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d --build --wait"]
async fn connection_refused_is_network() {
    let mut cfg = ftp_config(FtpSecurity::Plain);
    cfg.port = 2199;
    let p = ScriptedFtpPrompter::new();
    assert_err(
        open_test_control(&mut cfg, &p).await,
        ErrorKind::Network,
        "",
    );
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d --build --wait"]
async fn killed_session_is_network() {
    let p = ScriptedFtpPrompter::new();
    let mut c = open_test_control(&mut ftp_config(FtpSecurity::Plain), &p)
        .await
        .unwrap();
    kill_ftp_sessions();
    let e = c.quote("NOOP").await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::Network, "{e:?}");
    // The listener survived.
    let mut c = open_test_control(&mut ftp_config(FtpSecurity::Plain), &p)
        .await
        .unwrap();
    assert_eq!(c.quote("NOOP").await.unwrap().0, 200);
}

#[tokio::test]
async fn silent_server_times_out() {
    // Accepts TCP and never speaks, like an HTTP or database port.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((sock, _)) = listener.accept().await {
            held.push(sock);
        }
    });
    let mut cfg = ftp_config(FtpSecurity::Plain);
    cfg.port = port;
    cfg.connect_timeout = Duration::from_millis(500);
    let p = ScriptedFtpPrompter::new();

    let start = Instant::now();
    let result = tokio::time::timeout(Duration::from_secs(5), open_test_control(&mut cfg, &p))
        .await
        .expect("open_control hung on a silent server");
    let e = assert_err(result, ErrorKind::Network, "Connection timed out");
    assert_eq!(e.message, "Connection timed out");
    assert!(
        start.elapsed() < Duration::from_millis(1500),
        "{:?}",
        start.elapsed()
    );
}

fn tls_config(security: FtpSecurity, trust_test_ca: bool) -> FtpConfig {
    let mut cfg = ftp_config(security);
    if trust_test_ca {
        cfg.extra_roots = vec![ftps_fixture("ca.pem")];
    }
    cfg
}

fn other_fingerprint() -> String {
    cert_fingerprint(&CertificateDer::from_pem_file(ftps_fixture("other.pem")).unwrap())
}

/// `STAT` (vsftpd) says whether the control connection is encrypted and whether data
/// connections will be (`PROT P`).
async fn assert_encrypted(c: &mut TestControl, encrypted: bool) {
    let (code, text) = c.quote("STAT").await.unwrap();
    assert_eq!(code, 211, "{text}");
    let state = if encrypted { "encrypted" } else { "plain text" };
    assert!(
        text.contains(&format!("Control connection is {state}")),
        "{text}"
    );
    assert!(
        text.contains(&format!("Data connections will be {state}")),
        "{text}"
    );
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d --build --wait"]
async fn explicit_tls_with_trusted_ca_does_not_prompt() {
    let mut cfg = tls_config(FtpSecurity::ExplicitTls, true);
    let p = ScriptedFtpPrompter::new();
    let mut c = open_test_control(&mut cfg, &p).await.unwrap();
    assert_encrypted(&mut c, true).await;
    // Data connections are TLS too (vsftpd refuses a plain one after PROT P).
    c.nlst().await.unwrap();
    assert_eq!(p.calls().certificate, 0);
    assert!(p.pins().is_empty());
    assert!(cfg.pinned_fingerprint.is_none());
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d --build --wait"]
async fn explicit_tls_untrusted_prompts_and_pins() {
    let fp = server_fingerprint();
    let mut cfg = tls_config(FtpSecurity::ExplicitTls, false);
    let p = ScriptedFtpPrompter::new().accept_certificate(true);
    let mut c = open_test_control(&mut cfg, &p).await.unwrap();
    assert_encrypted(&mut c, true).await;
    c.nlst().await.unwrap();
    let prompts = p.certificate_prompts();
    assert_eq!(prompts.len(), 1, "{prompts:?}");
    assert_eq!(prompts[0].host, "127.0.0.1");
    assert_eq!(prompts[0].port, 2121);
    assert_eq!(prompts[0].fingerprint, fp);
    assert_eq!(prompts[0].problem, CertProblem::Untrusted);
    assert_eq!(p.pins(), vec![("127.0.0.1".to_owned(), 2121, fp.clone())]);
    // The in-memory config now carries the pin.
    assert_eq!(cfg.pinned_fingerprint.as_deref(), Some(fp.as_str()));

    // A new connection with the pin does not ask again, data connections included.
    let mut cfg = tls_config(FtpSecurity::ExplicitTls, false);
    cfg.pinned_fingerprint = Some(fp);
    let p = ScriptedFtpPrompter::new();
    let mut c = open_test_control(&mut cfg, &p).await.unwrap();
    c.nlst().await.unwrap();
    assert_eq!(p.calls().certificate, 0);
    assert!(p.pins().is_empty());
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d --build --wait"]
async fn untrusted_rejected_is_tls() {
    let mut cfg = tls_config(FtpSecurity::ExplicitTls, false);
    let p = ScriptedFtpPrompter::new().accept_certificate(false);
    let e = assert_err(
        open_test_control(&mut cfg, &p).await,
        ErrorKind::Tls,
        "Certificate not trusted",
    );
    assert_eq!(e.message, "Certificate not trusted");
    assert!(
        e.detail
            .as_deref()
            .unwrap_or("")
            .contains(&server_fingerprint()),
        "{e:?}"
    );
    assert_eq!(p.calls().certificate, 1);
    assert!(p.pins().is_empty());
    assert!(cfg.pinned_fingerprint.is_none());
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d --build --wait"]
async fn changed_certificate_prompts_again() {
    let _other = OtherCertificate::serve();
    let pinned = server_fingerprint();
    let other = other_fingerprint();
    assert_ne!(pinned, other);

    // Rejected: Tls, and the old pin stays.
    let mut cfg = tls_config(FtpSecurity::ImplicitTls, true);
    cfg.pinned_fingerprint = Some(pinned.clone());
    let p = ScriptedFtpPrompter::new().accept_certificate(false);
    let e = assert_err(
        open_test_control(&mut cfg, &p).await,
        ErrorKind::Tls,
        "Certificate not trusted",
    );
    assert!(e.detail.as_deref().unwrap_or("").contains(&other), "{e:?}");
    let prompts = p.certificate_prompts();
    assert_eq!(prompts.len(), 1, "{prompts:?}");
    assert_eq!(prompts[0].port, 2990);
    assert_eq!(prompts[0].fingerprint, other);
    assert_eq!(
        prompts[0].problem,
        CertProblem::Changed {
            pinned: pinned.clone()
        }
    );
    assert!(p.pins().is_empty());
    assert_eq!(cfg.pinned_fingerprint.as_deref(), Some(pinned.as_str()));

    // Accepted: the new certificate replaces the pin.
    let p = ScriptedFtpPrompter::new().accept_certificate(true);
    let mut c = open_test_control(&mut cfg, &p).await.unwrap();
    assert_encrypted(&mut c, true).await;
    assert_eq!(p.calls().certificate, 1);
    assert_eq!(
        p.pins(),
        vec![("127.0.0.1".to_owned(), 2990, other.clone())]
    );
    assert_eq!(cfg.pinned_fingerprint.as_deref(), Some(other.as_str()));
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d --build --wait"]
async fn implicit_tls_connects() {
    let mut cfg = tls_config(FtpSecurity::ImplicitTls, true);
    let p = ScriptedFtpPrompter::new();
    let mut c = open_test_control(&mut cfg, &p).await.unwrap();
    assert_encrypted(&mut c, true).await;
    assert_eq!(c.quote("NOOP").await.unwrap().0, 200);
    assert_eq!(p.calls().certificate, 0);
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d --build --wait"]
async fn plain_has_no_tls() {
    // Even trusting nothing and rejecting every certificate, plain FTP connects.
    let mut cfg = tls_config(FtpSecurity::Plain, false);
    let p = ScriptedFtpPrompter::new().accept_certificate(false);
    let mut c = open_test_control(&mut cfg, &p).await.unwrap();
    assert_encrypted(&mut c, false).await;
    c.nlst().await.unwrap();
    assert_eq!(p.calls().certificate, 0);
}

#[tokio::test]
async fn timeout_detail_names_the_target() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((sock, _)) = listener.accept().await {
            held.push(sock);
        }
    });
    for security in [
        FtpSecurity::Plain,
        FtpSecurity::ExplicitTls,
        FtpSecurity::ImplicitTls,
    ] {
        let mut cfg = ftp_config(security);
        cfg.port = port;
        cfg.connect_timeout = Duration::from_millis(300);
        let p = ScriptedFtpPrompter::new();
        let e = assert_err(
            open_test_control(&mut cfg, &p).await,
            ErrorKind::Network,
            "Connection timed out",
        );
        assert_eq!(e.detail, Some(format!("127.0.0.1:{port}")), "{security:?}");
        assert_eq!(p.calls().certificate, 0);
    }
}
