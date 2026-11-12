//! FTP control connections and sessions against the containers in `docker/compose.yml`.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use hoprs_core::{
    ConnectionState, EntryKind, Error, ErrorKind, ReconnectPolicy, RemoteEntry, RemotePath,
};
use hoprs_remote::testing::{
    FtpSession, OtherCertificate, TestControl, ftp_config, ftps_fixture, kill_ftp_sessions,
    open_test_control, pooled_quote, server_fingerprint, session_command,
    session_disconnect_inside_command, session_fail, session_quote,
};
use hoprs_remote::{
    CertProblem, Connection, FtpConfig, FtpFs, FtpPrompter, FtpSecurity, RemoteFs,
    ScriptedFtpPrompter, cert_fingerprint,
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
/// connections. It answers `NOOP` and `PWD`, answers `DATA425` with `425`, never answers
/// `HANG`, and answers `PASV` with a port nobody listens on. Returns its port.
async fn fake_server() -> u16 {
    fake_server_with(false).await.0
}

/// [`fake_server`]; with `hang_noop` it never answers `NOOP` either. Also returns the number
/// of client connections currently open.
async fn fake_server_with(hang_noop: bool) -> (u16, Arc<AtomicUsize>) {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    // A port that refuses connections: bound once, then closed.
    let closed = TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let pasv = format!(
        "227 Entering Passive Mode (127,0,0,1,{},{}).\r\n",
        closed / 256,
        closed % 256
    );
    let live = Arc::new(AtomicUsize::new(0));
    let open = live.clone();
    tokio::spawn(async move {
        while let Ok((sock, _)) = listener.accept().await {
            let pasv = pasv.clone();
            let live = live.clone();
            live.fetch_add(1, Ordering::SeqCst);
            tokio::spawn(async move {
                let (read, mut write) = sock.into_split();
                if write.write_all(b"220 fake\r\n").await.is_ok() {
                    let mut lines = BufReader::new(read).lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        let verb = line.split(' ').next().unwrap_or("").to_ascii_uppercase();
                        let reply: &[u8] = match verb.as_str() {
                            "USER" => b"331 Password please.\r\n",
                            "PASS" => b"230 Logged in.\r\n",
                            "TYPE" => b"200 Binary.\r\n",
                            "NOOP" if hang_noop => continue,
                            "NOOP" => b"200 NOOP ok.\r\n",
                            "PWD" => b"257 \"/\"\r\n",
                            "PASV" => pasv.as_bytes(),
                            "DATA425" => b"425 Can't open data connection.\r\n",
                            "HANG" => continue,
                            _ => b"502 Not implemented.\r\n",
                        };
                        if write.write_all(reply).await.is_err() {
                            break;
                        }
                    }
                }
                live.fetch_sub(1, Ordering::SeqCst);
            });
        }
    });
    (port, open)
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

/// Waits until `live` (open connections on a fake server) reaches `want`; panics after 3 s.
async fn wait_live(live: &AtomicUsize, want: usize) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while live.load(Ordering::SeqCst) != want {
        assert!(
            Instant::now() < deadline,
            "{} connections open, expected {want}",
            live.load(Ordering::SeqCst)
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// A TLS failure inside a control command (the data channel handshake runs after the server's
/// `150`, so its final reply is still unread) discards the control connection.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tls_error_inside_command_is_control_loss() {
    let s = fake_session(Duration::from_secs(5)).await;
    let rec = record_until(s.state(), |st| *st == ConnectionState::Connected);
    let e = session_fail(&s, Error::new(ErrorKind::Tls, "TLS failed: data")).await;
    assert_eq!(e.unwrap_err().kind, ErrorKind::Tls);
    let states = recorded(rec).await;
    assert_eq!(
        states,
        vec![
            ConnectionState::Reconnecting { attempt: 1 },
            ConnectionState::Connected
        ]
    );
    assert_eq!(s.generation(), 2);
}

/// A passive data connection that cannot be opened is a data channel error: `Network` for
/// the caller, but the control connection stays.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refused_data_connection_keeps_session() {
    let mut cfg = session_config();
    cfg.port = fake_server().await;
    cfg.keepalive_interval = Duration::from_secs(60);
    let p = Arc::new(ScriptedFtpPrompter::new());
    let fs = FtpFs::connect(cfg, p as Arc<dyn FtpPrompter>)
        .await
        .unwrap();
    let mut rx = Connection::state(&fs);
    rx.mark_unchanged();
    let e = fs.list(&RemotePath::new("/x").unwrap()).await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::Network, "{e:?}");
    assert!(
        e.detail
            .as_deref()
            .unwrap_or("")
            .starts_with("data channel: "),
        "{e:?}"
    );
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        !rx.has_changed().unwrap(),
        "state changed: {:?}",
        *rx.borrow()
    );
    assert_eq!(Connection::generation(&fs), 1);
    fs.disconnect().await;
}

/// `disconnect` does not wait for a keepalive `NOOP` the server never answers, and closes
/// the control connection.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn disconnect_during_keepalive() {
    let (port, live) = fake_server_with(true).await;
    let mut cfg = session_config();
    cfg.port = port;
    cfg.keepalive_interval = Duration::from_millis(200);
    cfg.connect_timeout = Duration::from_secs(20);
    let (s, _p) = session_with(cfg).await;
    wait_live(&live, 1).await;
    // The keepalive NOOP is now waiting for its reply.
    tokio::time::sleep(Duration::from_millis(600)).await;
    tokio::time::timeout(Duration::from_secs(1), s.disconnect())
        .await
        .expect("disconnect waited for the keepalive NOOP");
    let by_user =
        ConnectionState::Disconnected(Error::new(ErrorKind::Network, "Disconnected by user"));
    assert_eq!(*s.state().borrow(), by_user);
    wait_live(&live, 0).await;
}

/// A command that finishes while the session is being disconnected does not keep the control
/// connection open.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reply_racing_disconnect_closes_control() {
    let (port, live) = fake_server_with(false).await;
    let mut cfg = session_config();
    cfg.port = port;
    cfg.keepalive_interval = Duration::from_secs(60);
    let (s, _p) = session_with(cfg).await;
    wait_live(&live, 1).await;
    let _ = session_disconnect_inside_command(&s).await;
    assert!(matches!(
        *s.state().borrow(),
        ConnectionState::Disconnected(_)
    ));
    wait_live(&live, 0).await;
}

// ---------------------------------------------------------------------------------------
// FtpFs: the RemoteFs contract (mirrors tests/sftp.rs). Every test works in its own
// directory, `/home/frank/t-<nanos>-<counter>`, created through `FtpFs` and removed (best
// effort) when the test ends. FIFOs, symlinks and fixture files are made in the container as
// frank.
// ---------------------------------------------------------------------------------------

#[test]
fn ftp_fs_is_clone_send_sync() {
    fn check<T: Clone + Send + Sync + 'static>() {}
    check::<FtpFs>();
}

fn path(s: &str) -> RemotePath {
    RemotePath::new(s).unwrap()
}

/// Removes the work directory when dropped. Never panics.
struct Cleanup(String);

impl Drop for Cleanup {
    fn drop(&mut self) {
        let dir = self.0.clone();
        if std::panic::catch_unwind(|| docker(&["exec", "-T", "ftp", "rm", "-rf", &dir])).is_err() {
            eprintln!("could not remove {dir}");
        }
    }
}

struct Ctx {
    fs: FtpFs,
    work: RemotePath,
    _cleanup: Cleanup,
}

impl Ctx {
    /// `work` joined with a relative path.
    fn at(&self, rel: &str) -> RemotePath {
        path(&format!("{}/{rel}", self.work))
    }

    /// Runs `cmd` as frank inside the work directory.
    fn sh(&self, cmd: &str) {
        let script = format!("cd {} && {cmd}", self.work);
        docker(&["exec", "-T", "ftp", "su", "frank", "-c", &script]);
    }
}

/// A name no other test uses: `<nanos>-<counter>`.
fn unique() -> String {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    format!("{nanos}-{n}")
}

async fn setup() -> Ctx {
    let p = Arc::new(ScriptedFtpPrompter::new());
    let fs = FtpFs::connect(ftp_config(FtpSecurity::Plain), p as Arc<dyn FtpPrompter>)
        .await
        .unwrap();
    let work = format!("/home/frank/t-{}", unique());
    let cleanup = Cleanup(work.clone());
    fs.mkdir(&path(&work)).await.unwrap();
    Ctx {
        fs,
        work: path(&work),
        _cleanup: cleanup,
    }
}

fn names(list: &[RemoteEntry]) -> Vec<String> {
    let mut v: Vec<String> = list.iter().map(|e| e.name.clone()).collect();
    v.sort();
    v
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d --build --wait"]
async fn list_returns_direct_children_without_dot_entries() {
    let c = setup().await;
    c.fs.mkdir(&c.at("a")).await.unwrap();
    c.fs.mkdir(&c.at("a/sub")).await.unwrap();
    c.sh("printf hello > a/f.txt && : > a/sub/deep");
    let list = c.fs.list(&c.at("a")).await.unwrap();
    assert_eq!(names(&list), ["f.txt", "sub"]);
    let f = list.iter().find(|e| e.name == "f.txt").unwrap();
    assert_eq!(f.path, c.at("a/f.txt"));
    assert_eq!(f.kind, EntryKind::File);
    assert_eq!(f.size, 5);
    assert!(f.modified.is_some());
    assert_eq!(f.mode, Some(0o644));
    let sub = list.iter().find(|e| e.name == "sub").unwrap();
    assert_eq!(sub.kind, EntryKind::Dir);
    assert_eq!(sub.path, c.at("a/sub"));

    let st = c.fs.stat(&c.at("a/f.txt")).await.unwrap();
    assert_eq!(st.name, "f.txt");
    assert_eq!(st.path, c.at("a/f.txt"));
    assert_eq!(st.kind, EntryKind::File);
    assert_eq!(st.size, 5);
    let st = c.fs.stat(&c.at("a/sub")).await.unwrap();
    assert_eq!((st.name.as_str(), st.kind), ("sub", EntryKind::Dir));
    let root = c.fs.stat(&RemotePath::root()).await.unwrap();
    assert_eq!(root.kind, EntryKind::Dir);
    assert_eq!(root.name, "/");
    assert!(!c.fs.list(&RemotePath::root()).await.unwrap().is_empty());
    // An empty directory.
    c.fs.mkdir(&c.at("empty")).await.unwrap();
    assert!(c.fs.list(&c.at("empty")).await.unwrap().is_empty());
}

/// `stat` reports a file's exact modification time (`MDTM`), even when the listing only
/// shows a day.
#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d --build --wait"]
async fn stat_has_exact_mtime() {
    let c = setup().await;
    c.sh("printf x > old && touch -d @1000003 old");
    let st = c.fs.stat(&c.at("old")).await.unwrap();
    assert_eq!(
        st.modified,
        Some(UNIX_EPOCH + Duration::from_secs(1_000_003))
    );
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d --build --wait"]
async fn stat_missing_is_not_found() {
    let c = setup().await;
    let missing = c.at("nope");
    let e = c.fs.stat(&missing).await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::NotFound);
    assert!(e.message.contains(missing.as_str()), "{e:?}");
    assert_eq!(
        c.fs.stat(&c.at("nope/deeper")).await.unwrap_err().kind,
        ErrorKind::NotFound
    );
    assert_eq!(
        c.fs.list(&missing).await.unwrap_err().kind,
        ErrorKind::NotFound
    );
    assert_eq!(
        c.fs.list(&c.at("nope/deeper")).await.unwrap_err().kind,
        ErrorKind::NotFound
    );
    assert_eq!(
        c.fs.chmod(&missing, 0o600).await.unwrap_err().kind,
        ErrorKind::NotFound
    );
    assert_eq!(
        c.fs.remove_file(&missing).await.unwrap_err().kind,
        ErrorKind::NotFound
    );
    assert_eq!(
        c.fs.remove_dir(&missing).await.unwrap_err().kind,
        ErrorKind::NotFound
    );
    assert_eq!(
        c.fs.rename(&missing, &c.at("z")).await.unwrap_err().kind,
        ErrorKind::NotFound
    );
    // vsftpd has no MFMT: setting the time is a no-op, even for a missing file.
    c.fs.set_modified(&missing, UNIX_EPOCH).await.unwrap();
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d --build --wait"]
async fn list_on_file_is_io() {
    let c = setup().await;
    c.sh(": > f");
    let e = c.fs.list(&c.at("f")).await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::Io, "{e:?}");
    assert!(e.message.contains(c.at("f").as_str()), "{e:?}");
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d --build --wait"]
async fn mkdir_existing_ok() {
    let c = setup().await;
    c.fs.mkdir(&c.at("d")).await.unwrap();
    c.fs.mkdir(&c.at("d")).await.unwrap();
    assert_eq!(c.fs.stat(&c.at("d")).await.unwrap().kind, EntryKind::Dir);
    c.sh(": > f");
    assert_eq!(
        c.fs.mkdir(&c.at("f")).await.unwrap_err().kind,
        ErrorKind::Io
    );
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d --build --wait"]
async fn mkdir_missing_parent_not_found() {
    let c = setup().await;
    assert_eq!(
        c.fs.mkdir(&c.at("x/y")).await.unwrap_err().kind,
        ErrorKind::NotFound
    );
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d --build --wait"]
async fn rename_file_and_dir() {
    let c = setup().await;
    c.sh("printf x > a.txt");
    c.fs.rename(&c.at("a.txt"), &c.at("b.txt")).await.unwrap();
    assert_eq!(
        c.fs.stat(&c.at("a.txt")).await.unwrap_err().kind,
        ErrorKind::NotFound
    );
    assert_eq!(c.fs.stat(&c.at("b.txt")).await.unwrap().size, 1);

    // vsftpd uses rename(2): an existing target is replaced.
    c.sh("printf abc > c.txt");
    c.fs.rename(&c.at("c.txt"), &c.at("b.txt")).await.unwrap();
    assert_eq!(c.fs.stat(&c.at("b.txt")).await.unwrap().size, 3);
    assert_eq!(
        c.fs.stat(&c.at("c.txt")).await.unwrap_err().kind,
        ErrorKind::NotFound
    );

    c.fs.mkdir(&c.at("d")).await.unwrap();
    c.fs.mkdir(&c.at("d/e")).await.unwrap();
    c.sh("printf y > d/e/f.txt");
    c.fs.rename(&c.at("d"), &c.at("g")).await.unwrap();
    assert_eq!(c.fs.stat(&c.at("g/e/f.txt")).await.unwrap().size, 1);
    assert_eq!(
        c.fs.stat(&c.at("d")).await.unwrap_err().kind,
        ErrorKind::NotFound
    );
    assert_eq!(
        c.fs.rename(&c.at("nope"), &c.at("z"))
            .await
            .unwrap_err()
            .kind,
        ErrorKind::NotFound
    );
    assert_eq!(
        c.fs.rename(&c.at("b.txt"), &c.at("nope/z"))
            .await
            .unwrap_err()
            .kind,
        ErrorKind::NotFound
    );
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d --build --wait"]
async fn remove_dir_non_empty_is_io() {
    let c = setup().await;
    c.fs.mkdir(&c.at("d")).await.unwrap();
    c.sh(": > d/f");
    assert_eq!(
        c.fs.remove_dir(&c.at("d")).await.unwrap_err().kind,
        ErrorKind::Io
    );
    assert_eq!(
        c.fs.remove_file(&c.at("d")).await.unwrap_err().kind,
        ErrorKind::Io
    );
    assert_eq!(
        c.fs.remove_dir(&c.at("d/f")).await.unwrap_err().kind,
        ErrorKind::Io
    );
    c.fs.remove_file(&c.at("d/f")).await.unwrap();
    c.fs.remove_dir(&c.at("d")).await.unwrap();
    assert_eq!(
        c.fs.remove_file(&c.at("d/f")).await.unwrap_err().kind,
        ErrorKind::NotFound
    );
    assert_eq!(
        c.fs.stat(&c.at("d")).await.unwrap_err().kind,
        ErrorKind::NotFound
    );
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d --build --wait"]
async fn chmod_roundtrip() {
    let c = setup().await;
    c.sh("printf data > m");
    c.fs.chmod(&c.at("m"), 0o640).await.unwrap();
    assert_eq!(c.fs.stat(&c.at("m")).await.unwrap().mode, Some(0o640));
    c.sh("test \"$(stat -c %a m)\" = 640");
    c.fs.mkdir(&c.at("d")).await.unwrap();
    c.fs.chmod(&c.at("d"), 0o700).await.unwrap();
    assert_eq!(c.fs.stat(&c.at("d")).await.unwrap().mode, Some(0o700));
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d --build --wait"]
async fn symlink_listed_not_followed() {
    let c = setup().await;
    c.fs.mkdir(&c.at("target")).await.unwrap();
    c.sh(
        ": > target/inside && ln -s target link && ln -s missing dangling && ln -s /etc/passwd pw",
    );
    let list = c.fs.list(&c.work).await.unwrap();
    assert_eq!(names(&list), ["dangling", "link", "pw", "target"]);
    for name in ["link", "dangling", "pw"] {
        let e = list.iter().find(|e| e.name == name).unwrap();
        assert_eq!(e.kind, EntryKind::Symlink, "{name}");
        assert_eq!(e.size, 0, "{name}");
    }
    for name in ["link", "dangling"] {
        assert_eq!(
            c.fs.stat(&c.at(name)).await.unwrap().kind,
            EntryKind::Symlink,
            "{name}"
        );
    }
    // Listing through a symlink to a directory shows the target's children; a symlink to a
    // file is not a directory.
    assert_eq!(names(&c.fs.list(&c.at("link")).await.unwrap()), ["inside"]);
    let e = c.fs.list(&c.at("pw")).await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::Io, "{e:?}");
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d --build --wait"]
async fn special_files_hidden() {
    let c = setup().await;
    c.sh("mkfifo fifo && : > plain");
    let list = c.fs.list(&c.work).await.unwrap();
    assert_eq!(names(&list), ["plain"]);
    let e = c.fs.stat(&c.at("fifo")).await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::Io, "{e:?}");
    assert!(e.message.contains("Not a regular file"), "{e:?}");
    assert!(e.message.contains(c.at("fifo").as_str()), "{e:?}");
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d --build --wait"]
async fn names_roundtrip_through_list() {
    let c = setup().await;
    c.sh("printf 1 > 'relatório final.pdf' && printf 22 > .env && printf 333 > 'a b'");
    c.fs.mkdir(&c.at("pasta ção")).await.unwrap();
    let list = c.fs.list(&c.work).await.unwrap();
    assert_eq!(
        names(&list),
        [".env", "a b", "pasta ção", "relatório final.pdf"]
    );
    for (name, size) in [(".env", 2), ("a b", 3), ("relatório final.pdf", 1)] {
        let st = c.fs.stat(&c.at(name)).await.unwrap();
        assert_eq!((st.name.as_str(), st.size), (name, size));
        assert_eq!(st.kind, EntryKind::File);
    }
    let inside = c.at("pasta ção");
    c.fs.mkdir(&inside.join("sub dir")).await.unwrap();
    assert_eq!(names(&c.fs.list(&inside).await.unwrap()), ["sub dir"]);
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d --build --wait"]
async fn lists_2k_entries() {
    let c = setup().await;
    c.fs.mkdir(&c.at("big")).await.unwrap();
    c.sh("cd big && seq -f 'f%g' 1 2000 | xargs touch");
    let list = c.fs.list(&c.at("big")).await.unwrap();
    assert_eq!(list.len(), 2000);
    let mut n = names(&list);
    n.dedup();
    assert_eq!(n.len(), 2000);
    assert!(list.iter().all(|e| e.kind == EntryKind::File));
}

/// vsftpd lists an unreadable directory as its own entry (it falls back to listing the
/// parent filtered by the name); that is reported as `Permission`, not as a child.
#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d --build --wait"]
async fn unreadable_dir_is_permission() {
    let c = setup().await;
    c.fs.mkdir(&c.at("nr")).await.unwrap();
    c.fs.chmod(&c.at("nr"), 0o000).await.unwrap();
    let e = c.fs.list(&c.at("nr")).await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::Permission, "{e:?}");
    // Children of an unreadable directory cannot be looked up: Permission, never NotFound
    // (the transfer engine would take NotFound as "absent") nor the parent's own entry.
    c.sh("chmod 700 nr && : > nr/x && mkdir nr/nr && chmod 000 nr");
    for child in ["nr/x", "nr/nr", "nr/missing"] {
        let e = c.fs.stat(&c.at(child)).await.unwrap_err();
        assert_eq!(e.kind, ErrorKind::Permission, "{child}: {e:?}");
    }
    for e in [
        c.fs.remove_file(&c.at("nr/x")).await.unwrap_err(),
        c.fs.chmod(&c.at("nr/x"), 0o600).await.unwrap_err(),
        c.fs.rename(&c.at("nr/x"), &c.at("y")).await.unwrap_err(),
    ] {
        assert_eq!(e.kind, ErrorKind::Permission, "{e:?}");
    }
    c.sh("chmod 700 nr");
    // A child of a file does not exist (SftpFs: NoSuchFile, NotFound), even one named like it.
    c.sh(": > f");
    for child in ["f/x", "f/f"] {
        let e = c.fs.stat(&c.at(child)).await.unwrap_err();
        assert_eq!(e.kind, ErrorKind::NotFound, "{child}: {e:?}");
    }
    // A readable directory holding one directory of the same name is listed normally.
    c.fs.mkdir(&c.at("same")).await.unwrap();
    c.fs.mkdir(&c.at("same/same")).await.unwrap();
    assert_eq!(names(&c.fs.list(&c.at("same")).await.unwrap()), ["same"]);
}
