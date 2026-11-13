//! Session state, keepalive and reconnection against the container in `docker/compose.yml`.
//!
//! Every test that changes the container's state restores it in a drop guard, so a failed
//! assertion does not break the tests that follow.

use std::sync::Arc;
use std::time::{Duration, Instant};

use hoprs_core::{Error, ErrorKind};
use hoprs_ssh::testing::{docker, kill_ssh_sessions, test_config};
use hoprs_ssh::{Prompter, ReconnectPolicy, ScriptedPrompter, SessionState, SshSession};
use secrecy::SecretString;
use tempfile::TempDir;
use tokio::sync::watch;

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

/// Connects as bob with the test policy (5 attempts, 100 ms .. 400 ms) and keepalive 1 s x 3.
async fn connected() -> (SshSession, Arc<ScriptedPrompter>, TempDir) {
    let dir = TempDir::new().unwrap();
    let mut cfg = test_config("bob", dir.path());
    cfg.auth.password = Some(SecretString::from("bob-pass"));
    cfg.keepalive_interval = Duration::from_secs(1);
    cfg.keepalive_max = 3;
    cfg.reconnect = ReconnectPolicy {
        max_attempts: 5,
        base_delay: Duration::from_millis(100),
        max_delay: Duration::from_millis(400),
    };
    let p = Arc::new(ScriptedPrompter::new().accept_host(true));
    let s = SshSession::connect(cfg, p.clone() as Arc<dyn Prompter>)
        .await
        .unwrap();
    assert_eq!(*s.state().borrow(), SessionState::Connected);
    assert_eq!(s.generation(), 1);
    (s, p, dir)
}

/// Waits until the state satisfies `pred`; panics after `timeout`.
async fn wait_state(
    rx: &mut watch::Receiver<SessionState>,
    pred: impl FnMut(&SessionState) -> bool,
    timeout: Duration,
) -> SessionState {
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
    mut rx: watch::Receiver<SessionState>,
    stop: fn(&SessionState) -> bool,
) -> tokio::task::JoinHandle<Vec<SessionState>> {
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
    rx: watch::Receiver<SessionState>,
) -> tokio::task::JoinHandle<Vec<SessionState>> {
    record_until(rx, |s| matches!(s, SessionState::Disconnected(_)))
}

async fn recorded(rec: tokio::task::JoinHandle<Vec<SessionState>>) -> Vec<SessionState> {
    tokio::time::timeout(Duration::from_secs(30), rec)
        .await
        .expect("recording did not finish")
        .unwrap()
}

fn attempts(states: &[SessionState]) -> Vec<u32> {
    states
        .iter()
        .filter_map(|s| match s {
            SessionState::Reconnecting { attempt } => Some(*attempt),
            _ => None,
        })
        .collect()
}

fn disconnected_error(states: &[SessionState]) -> Error {
    match states.last() {
        Some(SessionState::Disconnected(e)) => e.clone(),
        other => panic!("expected Disconnected, got {other:?}"),
    }
}

/// Asserts a refused host key rotation. The attempt count is deliberately not asserted:
/// sshd re-execs on SIGHUP and can drop the session before the listener is back, so early
/// attempts may fail with a network error first. What matters is that the session ends in
/// `Disconnected(HostKey)`, never reconnects and never prompts.
fn assert_host_key_refused(states: &[SessionState], host_prompts: u32, prompts_before: u32) {
    let tries = attempts(states);
    assert!(!tries.is_empty(), "{states:?}");
    let expected: Vec<u32> = (1..=tries.len() as u32).collect();
    assert_eq!(tries, expected, "attempts must increase from 1: {states:?}");
    assert!(
        !states.iter().any(|s| matches!(s, SessionState::Connected)),
        "must not reconnect: {states:?}"
    );
    let (last, earlier) = states.split_last().expect("no states recorded");
    assert!(
        earlier
            .iter()
            .all(|s| matches!(s, SessionState::Reconnecting { .. })),
        "only Reconnecting may precede the final state: {states:?}"
    );
    let e = disconnected_error(std::slice::from_ref(last));
    assert_eq!(e.kind, ErrorKind::HostKey, "{e:?}");
    assert_eq!(e.message, "Host key for 127.0.0.1 has changed", "{e:?}");
    assert_eq!(
        host_prompts, prompts_before,
        "a changed key must never prompt"
    );
}

fn assert_not_connected(result: Result<russh::Channel<russh::client::Msg>, Error>) {
    let Err(e) = result else {
        panic!("expected Not connected, got a channel");
    };
    assert_eq!(e.kind, ErrorKind::Network, "{e:?}");
    assert_eq!(e.message, "Not connected", "{e:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn reconnects_after_kill() {
    let (s, _p, _dir) = connected().await;
    let (_ch, tag) = s.open_channel_tagged().await.unwrap();
    assert_eq!(tag, 1);
    let rec = record_until(s.state(), |st| *st == SessionState::Connected);
    kill_ssh_sessions();
    let states = recorded(rec).await;
    assert_eq!(
        states,
        vec![
            SessionState::Reconnecting { attempt: 1 },
            SessionState::Connected
        ]
    );
    assert_eq!(s.generation(), 2);

    let (mut ch, tag) = s.open_channel_tagged().await.unwrap();
    assert_eq!(tag, 2);
    ch.exec(true, "echo alive").await.unwrap();
    let mut out = Vec::new();
    while let Some(msg) = ch.wait().await {
        if let russh::ChannelMsg::Data { data } = msg {
            out.extend_from_slice(&data);
        }
    }
    assert_eq!(out, b"alive\n");
    s.disconnect().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn open_channel_while_reconnecting_is_network() {
    let (s, _p, _dir) = connected().await;
    let mut rx = s.state();

    // A paused server keeps TCP open but never answers: the reconnect attempt hangs
    // waiting for the SSH banner, so the session stays in Reconnecting.
    let _restore = restore(&[&["unpause", "sshd"]]);
    docker(&["pause", "sshd"]);
    wait_state(
        &mut rx,
        |st| matches!(st, SessionState::Reconnecting { .. }),
        Duration::from_secs(10),
    )
    .await;

    let start = Instant::now();
    assert_not_connected(s.open_channel().await);
    assert!(
        start.elapsed() < Duration::from_millis(500),
        "open_channel waited"
    );
    assert!(matches!(*rx.borrow(), SessionState::Reconnecting { .. }));

    s.disconnect().await;
    let by_user =
        SessionState::Disconnected(Error::new(ErrorKind::Network, "Disconnected by user"));
    assert_eq!(*rx.borrow_and_update(), by_user);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        !rx.has_changed().unwrap(),
        "state changed after disconnect: {:?}",
        *rx.borrow()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn gives_up_after_max_attempts() {
    let (s, _p, _dir) = connected().await;
    let rec = record_until_disconnected(s.state());

    let _restore = restore(&[&["start", "sshd", "--wait"]]);
    docker(&["stop", "sshd"]);
    let states = recorded(rec).await;

    assert_eq!(attempts(&states), vec![1, 2, 3, 4, 5], "{states:?}");
    let e = disconnected_error(&states);
    assert_eq!(e.kind, ErrorKind::Network, "{e:?}");
    assert_eq!(s.generation(), 1);
    assert_not_connected(s.open_channel().await);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn hung_connection_detected_by_keepalive() {
    let (s, _p, _dir) = connected().await;
    let mut rx = s.state();

    let _restore = restore(&[&["unpause", "sshd"]]);
    docker(&["pause", "sshd"]);
    // The server is frozen once `pause` returns. russh counts keepalives from the last
    // reply received and gives up once more than `keepalive_max` go unanswered, so within
    // keepalive_interval x (keepalive_max + 1) = 1 s x 4 of the freeze. The 200 ms are
    // timer and scheduling slack only.
    let start = Instant::now();
    let state = wait_state(
        &mut rx,
        |st| *st != SessionState::Connected,
        Duration::from_millis(4200),
    )
    .await;
    let elapsed = start.elapsed();
    eprintln!("left Connected after {elapsed:?}: {state:?}");
    assert!(matches!(state, SessionState::Reconnecting { attempt: 1 }));
    s.disconnect().await;
}

/// Host keys the listener presents right now (empty while it is not accepting connections).
fn scan_host_keys() -> String {
    let out = std::process::Command::new("ssh-keyscan")
        .args([
            "-T",
            "2",
            "-p",
            &hoprs_ssh::testing::SSH_PORT.to_string(),
            "127.0.0.1",
        ])
        .stderr(std::process::Stdio::null())
        .output()
        .expect("ssh-keyscan must be installed");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| !l.starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Waits until the listener, after a SIGHUP re-exec, accepts connections again and presents
/// other host keys than `old`. Without this, reconnect attempt 1 can race the re-exec and
/// fail with a network error before reaching the host key check.
fn wait_for_rotated_listener(old: &str) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let now = scan_host_keys();
        if !now.is_empty() && now != old {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "sshd did not present a new host key in time"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn host_key_rotated_blocks_reconnect() {
    let (s, p, _dir) = connected().await;
    let prompts_before = p.calls().host;
    let rec = record_until_disconnected(s.state());

    let _restore = restore(&[&["down", "-v"], &["up", "-d", "--build", "--wait"]]);
    let old_keys = scan_host_keys();
    docker(&[
        "exec",
        "-T",
        "sshd",
        "sh",
        "-c",
        "rm /etc/ssh/keys/ssh_host_ed25519_key* && ssh-keygen -q -N \"\" -t ed25519 -f /etc/ssh/keys/ssh_host_ed25519_key && pkill -HUP -x sshd",
    ]);
    wait_for_rotated_listener(&old_keys);
    kill_ssh_sessions();
    let states = recorded(rec).await;

    assert_host_key_refused(&states, p.calls().host, prompts_before);
    assert_eq!(s.generation(), 1);
}

/// A server that comes back with a key of another algorithm (here ecdsa only, for a host
/// known by its ed25519 key) is refused as a changed key, without prompting: the session
/// pins the key verified on its first connection.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn host_key_algorithm_change_blocks_reconnect() {
    let (s, p, _dir) = connected().await;
    let prompts_before = p.calls().host;
    let rec = record_until_disconnected(s.state());

    let _restore = restore(&[&["down", "-v"], &["up", "-d", "--build", "--wait"]]);
    // sshd reads the key type from the file, not its name: this serves an ecdsa key only.
    let old_keys = scan_host_keys();
    docker(&[
        "exec",
        "-T",
        "sshd",
        "sh",
        "-c",
        "rm /etc/ssh/keys/ssh_host_ed25519_key* && ssh-keygen -q -N \"\" -t ecdsa -f /etc/ssh/keys/ssh_host_ed25519_key && pkill -HUP -x sshd",
    ]);
    wait_for_rotated_listener(&old_keys);
    kill_ssh_sessions();
    let states = recorded(rec).await;

    assert_host_key_refused(&states, p.calls().host, prompts_before);
    assert_eq!(s.generation(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn explicit_disconnect_does_not_reconnect() {
    let (s, _p, _dir) = connected().await;
    let mut rx = s.state();

    s.disconnect().await;
    let expected =
        SessionState::Disconnected(Error::new(ErrorKind::Network, "Disconnected by user"));
    assert_eq!(*rx.borrow_and_update(), expected);

    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(
        !rx.has_changed().unwrap(),
        "state changed: {:?}",
        *rx.borrow()
    );
    assert_eq!(*rx.borrow(), expected);
    assert_not_connected(s.open_channel().await);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn reconnect_now_after_disconnect() {
    let (s, _p, _dir) = connected().await;
    let mut rx = s.state();
    s.disconnect().await;

    s.reconnect_now().await;
    wait_state(
        &mut rx,
        |st| *st == SessionState::Connected,
        Duration::from_secs(10),
    )
    .await;
    assert_eq!(s.generation(), 2);
    s.open_channel().await.unwrap();
    s.disconnect().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn supervisor_ends_when_last_handle_drops() {
    let (s, _p, _dir) = connected().await;
    let mut rx = s.state();
    let clone = s.clone();
    drop(s);
    assert!(
        clone.open_channel().await.is_ok(),
        "a clone keeps the session"
    );
    drop(clone);
    let closed = tokio::time::timeout(Duration::from_secs(1), rx.changed())
        .await
        .expect("supervisor still running 1 s after the last handle dropped");
    assert!(closed.is_err(), "expected the state channel to close");
}

// ---------------------------------------------------------------------------------------
// Observable start, WaitingForUser and remembered secrets.
// ---------------------------------------------------------------------------------------

/// A known_hosts directory that already trusts the container, so only auth prompts remain.
async fn known_host_dir() -> TempDir {
    let dir = TempDir::new().unwrap();
    let mut cfg = test_config("bob", dir.path());
    cfg.auth.password = Some(SecretString::from("bob-pass"));
    let p = Arc::new(ScriptedPrompter::new().accept_host(true));
    SshSession::connect(cfg, p as Arc<dyn Prompter>)
        .await
        .unwrap()
        .disconnect()
        .await;
    dir
}

/// bob with the test policy, password prompted (not configured).
fn prompted_config(dir: &TempDir) -> hoprs_ssh::SshConfig {
    let mut cfg = test_config("bob", dir.path());
    cfg.keepalive_interval = Duration::from_secs(1);
    cfg.keepalive_max = 3;
    cfg.reconnect = ReconnectPolicy {
        max_attempts: 5,
        base_delay: Duration::from_millis(100),
        max_delay: Duration::from_millis(400),
    };
    cfg
}

/// Answers the password prompt with bob's password, but only once allowed: until then it
/// cancels (`gate: false`) or waits for [`release`](Self::release) (`gate: true`).
struct HeldPassword {
    allowed: std::sync::atomic::AtomicBool,
    gate: Option<tokio::sync::Semaphore>,
    calls: std::sync::atomic::AtomicU32,
}

impl HeldPassword {
    fn cancelling() -> Arc<Self> {
        Arc::new(Self {
            allowed: false.into(),
            gate: None,
            calls: 0.into(),
        })
    }

    fn gated() -> Arc<Self> {
        Arc::new(Self {
            allowed: true.into(),
            gate: Some(tokio::sync::Semaphore::new(0)),
            calls: 0.into(),
        })
    }

    fn allow(&self) {
        self.allowed
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    fn release(&self) {
        if let Some(gate) = &self.gate {
            gate.add_permits(1);
        }
    }

    fn calls(&self) -> u32 {
        self.calls.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl Prompter for HeldPassword {
    async fn confirm_host_key(&self, _: &str, _: u16, _: &str, _: &str, _: &[String]) -> bool {
        true
    }

    async fn password(&self, _: &str, _: &str) -> Option<SecretString> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if let Some(gate) = &self.gate {
            gate.acquire().await.unwrap().forget();
        }
        self.allowed
            .load(std::sync::atomic::Ordering::SeqCst)
            .then(|| SecretString::from("bob-pass"))
    }

    async fn key_passphrase(&self, _: &std::path::Path) -> Option<SecretString> {
        None
    }

    async fn keyboard_interactive(
        &self,
        _: &str,
        _: &str,
        _: &[hoprs_ssh::KbdPrompt],
    ) -> Option<Vec<SecretString>> {
        None
    }
}

/// Current-thread runtime: the supervisor cannot run before the test yields, so the first
/// state is deterministic.
#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn start_is_observable_then_connects() {
    let dir = known_host_dir().await;
    let mut cfg = prompted_config(&dir);
    cfg.auth.password = Some(SecretString::from("bob-pass"));
    let p = Arc::new(ScriptedPrompter::new());
    let s = SshSession::start(cfg, p.clone() as Arc<dyn Prompter>);
    let mut rx = s.state();
    assert_eq!(*rx.borrow(), SessionState::Connecting);
    assert_eq!(s.generation(), 0);
    assert_not_connected(s.open_channel().await);

    wait_state(
        &mut rx,
        |st| *st == SessionState::Connected,
        Duration::from_secs(10),
    )
    .await;
    assert_eq!(s.generation(), 1);
    s.open_channel().await.unwrap();
    assert_eq!(p.calls().host, 0);
    s.disconnect().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn failed_first_connect_can_reconnect_now() {
    let dir = known_host_dir().await;
    let p = HeldPassword::cancelling();
    let s = SshSession::start(prompted_config(&dir), p.clone() as Arc<dyn Prompter>);
    let mut rx = s.state();
    let state = wait_state(
        &mut rx,
        |st| matches!(st, SessionState::Disconnected(_)),
        Duration::from_secs(10),
    )
    .await;
    let e = disconnected_error(&[state]);
    assert_eq!(e.kind, ErrorKind::Auth, "{e:?}");
    assert_eq!(e.message, "Cancelled by user", "{e:?}");
    assert_eq!(s.generation(), 0);
    assert_not_connected(s.open_channel().await);

    p.allow();
    s.reconnect_now().await;
    assert!(
        !matches!(*rx.borrow(), SessionState::Disconnected(_)),
        "reconnect_now returned before the state showed it: {:?}",
        *rx.borrow()
    );
    wait_state(
        &mut rx,
        |st| *st == SessionState::Connected,
        Duration::from_secs(10),
    )
    .await;
    assert_eq!(s.generation(), 1);
    assert_eq!(p.calls(), 2);
    s.open_channel().await.unwrap();
    s.disconnect().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn waiting_for_user_while_prompting() {
    let dir = known_host_dir().await;
    let p = HeldPassword::gated();
    let s = SshSession::start(prompted_config(&dir), p.clone() as Arc<dyn Prompter>);
    let rec = record_until(s.state(), |st| *st == SessionState::Connected);
    let mut rx = s.state();
    wait_state(
        &mut rx,
        |st| *st == SessionState::WaitingForUser,
        Duration::from_secs(10),
    )
    .await;
    assert_not_connected(s.open_channel().await);
    // The prompt holds the attempt without timing it out.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(*rx.borrow(), SessionState::WaitingForUser);

    p.release();
    let states = recorded(rec).await;
    assert_eq!(
        states.first(),
        Some(&SessionState::WaitingForUser),
        "{states:?}"
    );
    assert_eq!(states.last(), Some(&SessionState::Connected), "{states:?}");
    assert!(
        states[1..states.len() - 1]
            .iter()
            .all(|st| *st == SessionState::Connecting),
        "the prompt must hand back to Connecting: {states:?}"
    );
    assert_eq!(p.calls(), 1);
    s.disconnect().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn prompted_password_reused_on_reconnect() {
    let dir = known_host_dir().await;
    let p = Arc::new(
        ScriptedPrompter::new()
            .accept_host(true)
            .with_password(Some("bob-pass")),
    );
    let s = SshSession::connect(prompted_config(&dir), p.clone() as Arc<dyn Prompter>)
        .await
        .unwrap();
    assert_eq!(p.calls().password, 1);

    let rec = record_until(s.state(), |st| *st == SessionState::Connected);
    kill_ssh_sessions();
    let states = recorded(rec).await;
    assert_eq!(
        states,
        vec![
            SessionState::Reconnecting { attempt: 1 },
            SessionState::Connected
        ]
    );
    assert_eq!(s.generation(), 2);
    assert_eq!(
        p.calls().password,
        1,
        "the prompted password must be reused"
    );
    s.open_channel().await.unwrap();
    s.disconnect().await;
}
