//! FTP control connections and sessions against the containers in `docker/compose.yml`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use hoprs_core::{ConnectionState, Error, ErrorKind, ReconnectPolicy};
use hoprs_remote::testing::{
    FtpSession, OtherCertificate, TestControl, ftp_config, ftps_fixture, kill_ftp_sessions,
    open_test_control, pooled_quote, server_fingerprint, session_command, session_quote,
};
use hoprs_remote::{
    CertProblem, Connection, FtpConfig, FtpPrompter, FtpSecurity, ScriptedFtpPrompter,
    cert_fingerprint,
};
use hoprs_ssh::testing::docker;
use rustls::pki_types::CertificateDer;
use rustls::pki_types::pem::PemObject;
use secrecy::SecretString;
use tokio::net::TcpListener;
use tokio::sync::watch;

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

/// A server that greets, then refuses `AUTH TLS`: explicit FTPS fails with `Tls` and never
/// reaches a certificate (no handshake happens).
#[tokio::test]
async fn refused_auth_tls_is_tls() {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (sock, _) = listener.accept().await.unwrap();
        let (read, mut write) = sock.into_split();
        write.write_all(b"220 ok\r\n").await.unwrap();
        let mut lines = BufReader::new(read).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let reply: &[u8] = if line.eq_ignore_ascii_case("AUTH TLS") {
                b"500 nope\r\n"
            } else {
                b"502 not implemented\r\n"
            };
            if write.write_all(reply).await.is_err() {
                break;
            }
        }
    });
    let mut cfg = ftp_config(FtpSecurity::ExplicitTls);
    cfg.port = port;
    cfg.connect_timeout = Duration::from_secs(5);
    let p = ScriptedFtpPrompter::new().accept_certificate(true);
    let e = assert_err(
        open_test_control(&mut cfg, &p).await,
        ErrorKind::Tls,
        "Server refused FTPS",
    );
    assert!(e.detail.as_deref().unwrap_or("").contains("500"), "{e:?}");
    assert_eq!(p.calls().certificate, 0);
    assert!(p.pins().is_empty());
}

// ---------------------------------------------------------------------------------------
// FtpSession: pool, keepalive and reconnection. Every test that changes a container's state
// restores it in a drop guard, so a failed assertion does not break the tests that follow.
// ---------------------------------------------------------------------------------------

/// Runs docker compose commands when dropped. Never panics, even while unwinding.
struct Restore(Vec<Vec<&'static str>>);

impl Drop for Restore {
    fn drop(&mut self) {
        for args in &self.0 {
            if std::panic::catch_unwind(|| docker(args)).is_err() {
                eprintln!("restore step docker compose {args:?} failed");
            }
        }
    }
}

fn restore(steps: &[&[&'static str]]) -> Restore {
    Restore(steps.iter().map(|s| s.to_vec()).collect())
}

/// Plain FTP as frank with the test policy (5 attempts, 100 ms .. 400 ms) and keepalive 1 s.
/// The connect timeout (which also bounds the keepalive `NOOP`) stays above the ~3.4 s
/// vsftpd (PAM) takes to refuse a wrong password.
fn session_config() -> FtpConfig {
    let mut cfg = ftp_config(FtpSecurity::Plain);
    cfg.keepalive_interval = Duration::from_secs(1);
    cfg.connect_timeout = Duration::from_secs(6);
    cfg.reconnect = ReconnectPolicy {
        max_attempts: 5,
        base_delay: Duration::from_millis(100),
        max_delay: Duration::from_millis(400),
    };
    cfg
}

async fn session_with(cfg: FtpConfig) -> (FtpSession, Arc<ScriptedFtpPrompter>) {
    let p = Arc::new(ScriptedFtpPrompter::new());
    let s = FtpSession::connect(cfg, p.clone() as Arc<dyn FtpPrompter>)
        .await
        .unwrap();
    assert_eq!(*s.state().borrow(), ConnectionState::Connected);
    assert_eq!(s.generation(), 1);
    (s, p)
}

async fn session() -> (FtpSession, Arc<ScriptedFtpPrompter>) {
    session_with(session_config()).await
}

/// Waits until the state satisfies `pred`; panics after `timeout`.
async fn wait_state(
    rx: &mut watch::Receiver<ConnectionState>,
    pred: impl FnMut(&ConnectionState) -> bool,
    timeout: Duration,
) -> ConnectionState {
    let found = match tokio::time::timeout(timeout, rx.wait_for(pred)).await {
        Ok(Ok(state)) => Some(state.clone()),
        Ok(Err(_)) => panic!("session state channel closed"),
        Err(_) => None,
    };
    found.unwrap_or_else(|| panic!("timed out after {timeout:?}; state is {:?}", *rx.borrow()))
}

/// Records every state change until one satisfies `stop`, in a separate task so blocking
/// docker calls made by the test do not hide short-lived states.
fn record_until(
    mut rx: watch::Receiver<ConnectionState>,
    stop: fn(&ConnectionState) -> bool,
) -> tokio::task::JoinHandle<Vec<ConnectionState>> {
    rx.mark_unchanged();
    tokio::spawn(async move {
        let mut seen = Vec::new();
        loop {
            rx.changed().await.expect("session state channel closed");
            let state = rx.borrow_and_update().clone();
            let done = stop(&state);
            seen.push(state);
            if done {
                return seen;
            }
        }
    })
}

fn record_until_disconnected(
    rx: watch::Receiver<ConnectionState>,
) -> tokio::task::JoinHandle<Vec<ConnectionState>> {
    record_until(rx, |s| matches!(s, ConnectionState::Disconnected(_)))
}

async fn recorded(rec: tokio::task::JoinHandle<Vec<ConnectionState>>) -> Vec<ConnectionState> {
    tokio::time::timeout(Duration::from_secs(30), rec)
        .await
        .expect("recording did not finish")
        .unwrap()
}

fn attempts(states: &[ConnectionState]) -> Vec<u32> {
    states
        .iter()
        .filter_map(|s| match s {
            ConnectionState::Reconnecting { attempt } => Some(*attempt),
            _ => None,
        })
        .collect()
}

fn disconnected_error(states: &[ConnectionState]) -> Error {
    match states.last() {
        Some(ConnectionState::Disconnected(e)) => e.clone(),
        other => panic!("expected Disconnected, got {other:?}"),
    }
}

fn assert_not_connected<T>(result: Result<T, Error>) {
    let Err(e) = result else {
        panic!("expected Not connected, got Ok");
    };
    assert_eq!(e.kind, ErrorKind::Network, "{e:?}");
    assert_eq!(e.message, "Not connected", "{e:?}");
}

async fn assert_pwd(s: &FtpSession) {
    let (code, text) = session_quote(s, "PWD").await.unwrap();
    assert_eq!(code, 257, "{text}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d --build --wait"]
async fn reconnects_after_kill() {
    let (s, _p) = session().await;
    assert_pwd(&s).await;
    let rec = record_until(s.state(), |st| *st == ConnectionState::Connected);
    kill_ftp_sessions();
    let states = recorded(rec).await;
    assert_eq!(
        states,
        vec![
            ConnectionState::Reconnecting { attempt: 1 },
            ConnectionState::Connected
        ]
    );
    assert_eq!(s.generation(), 2);
    assert_eq!(Connection::generation(&s), 2);
    assert_pwd(&s).await;
    s.disconnect().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d --build --wait"]
async fn command_while_reconnecting_is_network() {
    let (s, _p) = session().await;
    let mut rx = s.state();

    // A paused server keeps the listening socket (the kernel still accepts) but never
    // answers: the keepalive NOOP times out, and each reconnect attempt waits for a greeting.
    let _restore = restore(&[&["unpause", "ftp"]]);
    docker(&["pause", "ftp"]);
    // Keepalive after 1 s idle, then the NOOP times out after 6 s.
    wait_state(
        &mut rx,
        |st| matches!(st, ConnectionState::Reconnecting { .. }),
        Duration::from_secs(15),
    )
    .await;

    let start = Instant::now();
    assert_not_connected(session_quote(&s, "PWD").await);
    assert_not_connected(s.checkout().await);
    assert!(
        start.elapsed() < Duration::from_millis(500),
        "a command waited: {:?}",
        start.elapsed()
    );
    assert!(matches!(*rx.borrow(), ConnectionState::Reconnecting { .. }));

    s.disconnect().await;
    let by_user =
        ConnectionState::Disconnected(Error::new(ErrorKind::Network, "Disconnected by user"));
    assert_eq!(*rx.borrow_and_update(), by_user);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        !rx.has_changed().unwrap(),
        "state changed after disconnect: {:?}",
        *rx.borrow()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d --build --wait"]
async fn gives_up_after_max_attempts() {
    let (s, _p) = session().await;
    let rec = record_until_disconnected(s.state());

    let _restore = restore(&[&["start", "--wait", "ftp"]]);
    docker(&["stop", "ftp"]);
    let states = recorded(rec).await;

    assert_eq!(attempts(&states), vec![1, 2, 3, 4, 5], "{states:?}");
    let e = disconnected_error(&states);
    assert_eq!(e.kind, ErrorKind::Network, "{e:?}");
    assert_eq!(s.generation(), 1);
    assert_not_connected(session_quote(&s, "PWD").await);
    assert_not_connected(s.checkout().await);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d --build --wait"]
async fn keepalive_prevents_idle_timeout() {
    // vsftpd's idle_session_timeout is 8 s; the keepalive is 1 s.
    let (s, _p) = session().await;
    let mut rx = s.state();
    rx.mark_unchanged();
    tokio::time::sleep(Duration::from_secs(12)).await;
    assert!(
        !rx.has_changed().unwrap(),
        "state changed while idle: {:?}",
        *rx.borrow()
    );
    assert_eq!(*rx.borrow(), ConnectionState::Connected);
    assert_eq!(s.generation(), 1);
    assert_pwd(&s).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d --build --wait"]
async fn pool_respects_limit() {
    let mut cfg = session_config();
    cfg.transfer_connections = 2;
    let (s, _p) = session_with(cfg).await;

    let mut a = s.checkout().await.unwrap();
    let mut b = s.checkout().await.unwrap();
    assert_eq!(pooled_quote(&mut a, "NOOP").await.unwrap().0, 200);
    assert_eq!(pooled_quote(&mut b, "NOOP").await.unwrap().0, 200);
    assert_eq!((a.generation(), b.generation()), (1, 1));

    // The pool is full: a third checkout waits (and can be cancelled)...
    assert!(
        tokio::time::timeout(Duration::from_millis(500), s.checkout())
            .await
            .is_err(),
        "a third connection was handed out"
    );
    // ...without blocking the control connection.
    assert_pwd(&s).await;

    let waiting = tokio::spawn({
        let s = s.clone();
        async move { s.checkout().await }
    });
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!waiting.is_finished(), "the third checkout did not wait");
    a.mark_clean();
    drop(a);
    let mut c = tokio::time::timeout(Duration::from_secs(5), waiting)
        .await
        .expect("the third checkout still waits after a connection came back")
        .unwrap()
        .unwrap();
    assert_eq!(pooled_quote(&mut c, "NOOP").await.unwrap().0, 200);
    drop((b, c));
    s.disconnect().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d --build --wait"]
async fn pooled_connection_from_old_generation_is_not_reused() {
    let mut cfg = session_config();
    cfg.transfer_connections = 1;
    let (s, _p) = session_with(cfg).await;

    let mut old = s.checkout().await.unwrap();
    assert_eq!(old.generation(), 1);
    let rec = record_until(s.state(), |st| *st == ConnectionState::Connected);
    kill_ftp_sessions();
    recorded(rec).await;
    assert_eq!(s.generation(), 2);

    // Returned clean, but it belongs to the dead generation 1: it is closed, not pooled.
    old.mark_clean();
    drop(old);
    let mut new = s.checkout().await.unwrap();
    assert_eq!(new.generation(), 2);
    assert_eq!(pooled_quote(&mut new, "NOOP").await.unwrap().0, 200);
    drop(new);
    s.disconnect().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d --build --wait"]
async fn auth_failure_on_reconnect_stops() {
    let (s, p) = session().await;
    let rec = record_until_disconnected(s.state());

    let _restore = restore(&[&[
        "exec",
        "-T",
        "ftp",
        "sh",
        "-c",
        "echo 'frank:frank-pass' | chpasswd",
    ]]);
    docker(&[
        "exec",
        "-T",
        "ftp",
        "sh",
        "-c",
        "echo 'frank:changed-pass' | chpasswd",
    ]);
    kill_ftp_sessions();
    let states = recorded(rec).await;

    assert_eq!(attempts(&states), vec![1], "{states:?}");
    let e = disconnected_error(&states);
    assert_eq!(e.kind, ErrorKind::Auth, "{e:?}");
    assert_eq!(s.generation(), 1);
    assert_eq!(p.calls().password, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d --build --wait"]
async fn explicit_disconnect_does_not_reconnect() {
    let (s, _p) = session().await;
    let mut rx = s.state();
    s.disconnect().await;
    let expected =
        ConnectionState::Disconnected(Error::new(ErrorKind::Network, "Disconnected by user"));
    assert_eq!(*rx.borrow_and_update(), expected);
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(
        !rx.has_changed().unwrap(),
        "state changed: {:?}",
        *rx.borrow()
    );
    assert_not_connected(session_quote(&s, "PWD").await);
    assert_not_connected(s.checkout().await);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d --build --wait"]
async fn supervisor_ends_when_last_handle_drops() {
    let (s, _p) = session().await;
    let mut rx = s.state();
    let clone = s.clone();
    drop(s);
    assert_pwd(&clone).await;
    drop(clone);
    let closed = tokio::time::timeout(Duration::from_secs(1), rx.changed())
        .await
        .expect("supervisor still running 1 s after the last handle dropped");
    assert!(closed.is_err(), "expected the state channel to close");
}

/// A minimal FTP server on 127.0.0.1 that logs anyone in and accepts any number of
/// connections. It answers `NOOP` and `PWD`, answers `DATA425` with `425`, and never answers
/// `HANG`. Returns its port.
async fn fake_server() -> u16 {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((sock, _)) = listener.accept().await {
            tokio::spawn(async move {
                let (read, mut write) = sock.into_split();
                if write.write_all(b"220 fake\r\n").await.is_err() {
                    return;
                }
                let mut lines = BufReader::new(read).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let verb = line.split(' ').next().unwrap_or("").to_ascii_uppercase();
                    let reply: &[u8] = match verb.as_str() {
                        "USER" => b"331 Password please.\r\n",
                        "PASS" => b"230 Logged in.\r\n",
                        "TYPE" => b"200 Binary.\r\n",
                        "NOOP" => b"200 NOOP ok.\r\n",
                        "PWD" => b"257 \"/\"\r\n",
                        "DATA425" => b"425 Can't open data connection.\r\n",
                        "HANG" => continue,
                        _ => b"502 Not implemented.\r\n",
                    };
                    if write.write_all(reply).await.is_err() {
                        break;
                    }
                }
            });
        }
    });
    port
}

/// A session on [`fake_server`] with the given command timeout and no keepalive in sight.
async fn fake_session(command_timeout: Duration) -> FtpSession {
    let mut cfg = session_config();
    cfg.port = fake_server().await;
    cfg.keepalive_interval = Duration::from_secs(60);
    cfg.command_timeout = command_timeout;
    session_with(cfg).await.0
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hung_control_command_times_out() {
    let s = fake_session(Duration::from_secs(1)).await;
    let rec = record_until(s.state(), |st| *st == ConnectionState::Connected);

    let start = Instant::now();
    let e = session_command(&s, "HANG").await.unwrap_err();
    let elapsed = start.elapsed();
    assert_eq!(e.kind, ErrorKind::Network, "{e:?}");
    assert_eq!(e.message, "Control command timed out", "{e:?}");
    assert!(
        elapsed >= Duration::from_secs(1) && elapsed < Duration::from_millis(2500),
        "{elapsed:?}"
    );
    // A timeout is control loss: the session reconnects (the fake server accepts again).
    let states = recorded(rec).await;
    assert_eq!(
        states,
        vec![
            ConnectionState::Reconnecting { attempt: 1 },
            ConnectionState::Connected
        ]
    );
    assert_eq!(s.generation(), 2);
    assert_eq!(session_command(&s, "PWD").await.unwrap(), "\"/\"");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn disconnect_while_command_hangs() {
    let s = fake_session(Duration::from_secs(30)).await;
    let mut rx = s.state();
    let hung = tokio::spawn({
        let s = s.clone();
        async move { session_command(&s, "HANG").await }
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!hung.is_finished());

    tokio::time::timeout(Duration::from_secs(1), s.disconnect())
        .await
        .expect("disconnect waited for the hung command");
    let by_user =
        ConnectionState::Disconnected(Error::new(ErrorKind::Network, "Disconnected by user"));
    assert_eq!(*rx.borrow_and_update(), by_user);
    let result = tokio::time::timeout(Duration::from_secs(1), hung)
        .await
        .expect("the hung command was not cancelled")
        .unwrap();
    assert_not_connected(result);
    assert_not_connected(session_command(&s, "PWD").await);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(*rx.borrow(), by_user);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn data_channel_reply_keeps_session() {
    let s = fake_session(Duration::from_secs(5)).await;
    let mut rx = s.state();
    rx.mark_unchanged();
    let e = session_command(&s, "DATA425").await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::Network, "{e:?}");
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        !rx.has_changed().unwrap(),
        "state changed: {:?}",
        *rx.borrow()
    );
    assert_eq!(*rx.borrow(), ConnectionState::Connected);
    assert_eq!(s.generation(), 1);
    assert_eq!(session_command(&s, "PWD").await.unwrap(), "\"/\"");
}

/// A connection loss seen on a transfer connection makes the session probe its control
/// connection at once, instead of after up to `keepalive_interval`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d --build --wait"]
async fn pooled_connection_loss_probes_control() {
    let mut cfg = session_config();
    cfg.keepalive_interval = Duration::from_secs(60);
    let (s, _p) = session_with(cfg).await;
    let mut rx = s.state();
    let mut conn = s.checkout().await.unwrap();
    kill_ftp_sessions();

    let e = pooled_quote(&mut conn, "NOOP").await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::Network, "{e:?}");
    conn.report(&e);
    drop(conn);
    wait_state(
        &mut rx,
        |st| matches!(st, ConnectionState::Reconnecting { .. }),
        Duration::from_secs(3),
    )
    .await;
    wait_state(
        &mut rx,
        |st| *st == ConnectionState::Connected,
        Duration::from_secs(10),
    )
    .await;
    assert_eq!(s.generation(), 2);
    s.disconnect().await;
}
