//! FTP control connection against the containers in `docker/compose.yml`.

use std::time::{Duration, Instant};

use hoprs_core::{Error, ErrorKind};
use hoprs_remote::testing::{TestControl, ftp_config, kill_ftp_sessions, open_test_control};
use hoprs_remote::{FtpSecurity, ScriptedFtpPrompter};
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
    let mut c = open_test_control(&ftp_config(FtpSecurity::Plain), &p)
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
    let e = assert_err(open_test_control(&cfg, &p).await, ErrorKind::Auth, "");
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
    let mut c = open_test_control(&cfg, &p).await.unwrap();
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
        open_test_control(&cfg, &p).await,
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
    assert_err(open_test_control(&cfg, &p).await, ErrorKind::Network, "");
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d --build --wait"]
async fn killed_session_is_network() {
    let p = ScriptedFtpPrompter::new();
    let mut c = open_test_control(&ftp_config(FtpSecurity::Plain), &p)
        .await
        .unwrap();
    kill_ftp_sessions();
    let e = c.quote("NOOP").await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::Network, "{e:?}");
    // The listener survived.
    let mut c = open_test_control(&ftp_config(FtpSecurity::Plain), &p)
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
    let result = tokio::time::timeout(Duration::from_secs(5), open_test_control(&cfg, &p))
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
