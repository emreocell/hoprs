//! Integration tests against the OpenSSH container in `docker/compose.yml`.

use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
#[cfg(unix)]
use std::path::PathBuf;
#[cfg(unix)]
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;

use hoprs_core::{Error, ErrorKind};
use hoprs_ssh::testing::{fixture, test_config};
use hoprs_ssh::{KbdPrompt, KeySource, Prompter, ScriptedPrompter, SshConfig, SshSession};
use secrecy::SecretString;
use tempfile::TempDir;
use tokio::net::TcpListener;

async fn connect(cfg: SshConfig, prompter: &Arc<ScriptedPrompter>) -> Result<SshSession, Error> {
    SshSession::connect(cfg, prompter.clone() as Arc<dyn Prompter>).await
}

fn known_hosts_lines(dir: &Path) -> Vec<String> {
    fs::read_to_string(dir.join("known_hosts"))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// Learns the container's host key so auth tests start from a known host.
async fn trusted_dir() -> TempDir {
    let dir = TempDir::new().unwrap();
    let mut cfg = test_config("bob", dir.path());
    cfg.auth.password = Some(SecretString::from("bob-pass"));
    let p = Arc::new(ScriptedPrompter::new().accept_host(true));
    connect(cfg, &p).await.unwrap().disconnect().await;
    dir
}

fn assert_err(result: Result<SshSession, Error>, kind: ErrorKind, message: &str) -> Error {
    let Err(e) = result else {
        panic!("expected {kind:?} error, got a session");
    };
    assert_eq!(e.kind, kind, "{e:?}");
    assert_eq!(e.message, message, "{e:?}");
    e
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn unknown_host_prompts_and_learns() {
    let dir = TempDir::new().unwrap();
    let mut cfg = test_config("bob", dir.path());
    cfg.auth.password = Some(SecretString::from("bob-pass"));

    let p = Arc::new(ScriptedPrompter::new().accept_host(true));
    let session = connect(cfg.clone(), &p).await.unwrap();
    assert_eq!(session.host(), "127.0.0.1");
    assert_eq!(session.port(), 2222);
    assert_eq!(session.user(), "bob");
    session.disconnect().await;
    assert_eq!(p.calls().host, 1);
    let lines = known_hosts_lines(dir.path());
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(
        lines[0].starts_with("[127.0.0.1]:2222 ssh-ed25519 "),
        "{lines:?}"
    );

    let p2 = Arc::new(ScriptedPrompter::new());
    connect(cfg, &p2).await.unwrap().disconnect().await;
    assert_eq!(p2.calls().host, 0);
    assert_eq!(known_hosts_lines(dir.path()).len(), 1);
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn unknown_host_rejected() {
    let dir = TempDir::new().unwrap();
    let mut cfg = test_config("bob", dir.path());
    cfg.auth.password = Some(SecretString::from("bob-pass"));
    let p = Arc::new(ScriptedPrompter::new().accept_host(false));
    assert_err(
        connect(cfg, &p).await,
        ErrorKind::HostKey,
        "Cancelled by user",
    );
    assert_eq!(p.calls().host, 1);
    assert!(known_hosts_lines(dir.path()).is_empty());
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn changed_host_key_blocks() {
    let dir = TempDir::new().unwrap();
    // A different ed25519 key recorded for the server.
    let other = fs::read_to_string(fixture("id_ed25519.pub")).unwrap();
    let blob = other.split_whitespace().nth(1).unwrap();
    fs::write(
        dir.path().join("known_hosts"),
        format!("[127.0.0.1]:2222 ssh-ed25519 {blob}\n"),
    )
    .unwrap();
    let mut cfg = test_config("bob", dir.path());
    cfg.auth.password = Some(SecretString::from("bob-pass"));
    let p = Arc::new(ScriptedPrompter::new().accept_host(true));
    let e = assert_err(
        connect(cfg, &p).await,
        ErrorKind::HostKey,
        "Host key for 127.0.0.1 has changed",
    );
    assert_eq!(p.calls().host, 0);
    let detail = e.detail.unwrap_or_default();
    assert!(detail.contains("line 1"), "{detail}");
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn revoked_host_key_blocks() {
    let dir = trusted_dir().await;
    let learned = known_hosts_lines(dir.path()).remove(0);
    fs::write(
        dir.path().join("known_hosts"),
        format!("@revoked {learned}\n"),
    )
    .unwrap();
    let mut cfg = test_config("bob", dir.path());
    cfg.auth.password = Some(SecretString::from("bob-pass"));
    let p = Arc::new(ScriptedPrompter::new().accept_host(true));
    let e = assert_err(
        connect(cfg, &p).await,
        ErrorKind::HostKey,
        "Host key for 127.0.0.1 has been revoked",
    );
    assert_eq!(p.calls().host, 0);
    assert_eq!(e.detail.as_deref(), Some("known_hosts line 1"));
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn password_auth_from_config() {
    let dir = trusted_dir().await;
    let mut cfg = test_config("bob", dir.path());
    cfg.auth.password = Some(SecretString::from("bob-pass"));
    let p = Arc::new(ScriptedPrompter::new());
    connect(cfg, &p).await.unwrap().disconnect().await;
    assert_eq!(p.calls().password, 0);
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn password_auth_prompted() {
    let dir = trusted_dir().await;
    let cfg = test_config("bob", dir.path());
    let p = Arc::new(ScriptedPrompter::new().with_password(Some("bob-pass")));
    connect(cfg, &p).await.unwrap().disconnect().await;
    assert_eq!(p.calls().password, 1);
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn password_prompt_cancelled() {
    let dir = trusted_dir().await;
    let cfg = test_config("bob", dir.path());
    let p = Arc::new(ScriptedPrompter::new().with_password(None));
    assert_err(connect(cfg, &p).await, ErrorKind::Auth, "Cancelled by user");
    assert_eq!(p.calls().password, 1);
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn wrong_password_fails() {
    let dir = trusted_dir().await;
    let mut cfg = test_config("bob", dir.path());
    cfg.auth.password = Some(SecretString::from("not-bobs-pass"));
    let p = Arc::new(ScriptedPrompter::new());
    let e = assert_err(
        connect(cfg, &p).await,
        ErrorKind::Auth,
        "Authentication failed",
    );
    let shown = format!("{e:?}");
    assert!(shown.contains("password"), "{shown}");
    assert!(!shown.contains("not-bobs-pass"), "{shown}");
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn key_auth() {
    let dir = trusted_dir().await;
    let mut cfg = test_config("alice", dir.path());
    cfg.auth.key = KeySource::File {
        path: fixture("id_ed25519"),
        passphrase: None,
    };
    let p = Arc::new(ScriptedPrompter::new());
    connect(cfg, &p).await.unwrap().disconnect().await;
    assert_eq!(p.calls().password, 0);
    assert_eq!(p.calls().passphrase, 0);
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn encrypted_key_with_configured_passphrase() {
    let dir = trusted_dir().await;
    let mut cfg = test_config("alice", dir.path());
    cfg.auth.key = KeySource::File {
        path: fixture("id_ed25519_pp"),
        passphrase: Some(SecretString::from("fixture-pp")),
    };
    let p = Arc::new(ScriptedPrompter::new());
    connect(cfg, &p).await.unwrap().disconnect().await;
    assert_eq!(p.calls().passphrase, 0);
    assert_eq!(p.calls().password, 0);
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn encrypted_key_prompts_passphrase() {
    let dir = trusted_dir().await;
    let mut cfg = test_config("alice", dir.path());
    cfg.auth.key = KeySource::File {
        path: fixture("id_ed25519_pp"),
        passphrase: None,
    };
    let p = Arc::new(ScriptedPrompter::new().with_passphrase(Some("fixture-pp")));
    connect(cfg, &p).await.unwrap().disconnect().await;
    assert_eq!(p.calls().passphrase, 1);
    assert_eq!(p.calls().password, 0);
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn connection_refused_is_network() {
    let dir = TempDir::new().unwrap();
    let mut cfg = test_config("bob", dir.path());
    cfg.port = 2299;
    let p = Arc::new(ScriptedPrompter::new().accept_host(true));
    let Err(e) = connect(cfg, &p).await else {
        panic!("expected an error");
    };
    assert_eq!(e.kind, ErrorKind::Network, "{e:?}");
    assert_eq!(p.calls().host, 0);
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn passphrase_cancelled_falls_back_to_password() {
    let dir = trusted_dir().await;
    let mut cfg = test_config("alice", dir.path());
    cfg.auth.key = KeySource::File {
        path: fixture("id_ed25519_pp"),
        passphrase: None,
    };
    let p = Arc::new(
        ScriptedPrompter::new()
            .with_passphrase(None)
            .with_password(Some("alice-pass")),
    );
    connect(cfg, &p).await.unwrap().disconnect().await;
    assert_eq!(p.calls().passphrase, 1);
    assert_eq!(p.calls().password, 1);
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn skipped_key_is_reported() {
    let dir = trusted_dir().await;
    let key = fixture("id_ed25519_pp");
    let mut cfg = test_config("alice", dir.path());
    cfg.auth.key = KeySource::File {
        path: key.clone(),
        passphrase: Some(SecretString::from("not-the-pp")),
    };
    cfg.auth.password = Some(SecretString::from("not-alices-pass"));
    let p = Arc::new(ScriptedPrompter::new());
    let e = assert_err(
        connect(cfg, &p).await,
        ErrorKind::Auth,
        "Authentication failed",
    );
    let detail = e.detail.clone().unwrap_or_default();
    let expected = format!("publickey (skipped {}: wrong passphrase)", key.display());
    assert!(detail.contains(&expected), "{detail}");
    assert!(detail.contains("password"), "{detail}");
    let shown = format!("{e:?}");
    assert!(!shown.contains("not-the-pp") && !shown.contains("not-alices-pass"));
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
    let dir = TempDir::new().unwrap();
    let mut cfg = test_config("bob", dir.path());
    cfg.port = port;
    cfg.connect_timeout = Duration::from_millis(500);
    let p = Arc::new(ScriptedPrompter::new().accept_host(true));

    let start = Instant::now();
    let result = tokio::time::timeout(Duration::from_secs(5), connect(cfg, &p))
        .await
        .expect("connect hung on a silent server");
    assert_err(result, ErrorKind::Network, "Connection timed out");
    assert!(
        start.elapsed() < Duration::from_millis(1500),
        "{:?}",
        start.elapsed()
    );
}

/// Accepts every host key after a delay longer than the connect timeout.
struct SlowHostPrompter;

#[async_trait]
impl Prompter for SlowHostPrompter {
    async fn confirm_host_key(&self, _: &str, _: u16, _: &str, _: &str) -> bool {
        tokio::time::sleep(Duration::from_millis(1500)).await;
        true
    }
    async fn password(&self, _: &str, _: &str) -> Option<SecretString> {
        None
    }
    async fn key_passphrase(&self, _: &Path) -> Option<SecretString> {
        None
    }
    async fn keyboard_interactive(
        &self,
        _: &str,
        _: &str,
        _: &[KbdPrompt],
    ) -> Option<Vec<SecretString>> {
        None
    }
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn slow_prompt_does_not_time_out() {
    let dir = TempDir::new().unwrap();
    let mut cfg = test_config("bob", dir.path());
    cfg.auth.password = Some(SecretString::from("bob-pass"));
    cfg.connect_timeout = Duration::from_millis(500);
    SshSession::connect(cfg, Arc::new(SlowHostPrompter))
        .await
        .unwrap()
        .disconnect()
        .await;
    assert_eq!(known_hosts_lines(dir.path()).len(), 1);
}

#[cfg(unix)]
/// A private `ssh-agent` holding one fixture key; killed on drop.
struct TestAgent {
    child: Child,
    socket: PathBuf,
    _dir: TempDir,
}

#[cfg(unix)]
impl TestAgent {
    fn start(key: &str) -> Self {
        let dir = TempDir::new().unwrap();
        let socket = dir.path().join("agent.sock");
        let child = Command::new("ssh-agent")
            .arg("-D")
            .arg("-a")
            .arg(&socket)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .spawn()
            .expect("ssh-agent is required for the agent tests");
        let agent = Self {
            child,
            socket,
            _dir: dir,
        };
        for _ in 0..100 {
            if agent.socket.exists() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(agent.socket.exists(), "ssh-agent did not create its socket");
        // ssh-add refuses keys that are group/world readable, and git does not keep the
        // fixture's mode: add a 0600 copy.
        let copy = agent._dir.path().join("key");
        fs::copy(fixture(key), &copy).unwrap();
        fs::set_permissions(&copy, fs::Permissions::from_mode(0o600)).unwrap();
        let status = Command::new("ssh-add")
            .arg(&copy)
            .env("SSH_AUTH_SOCK", &agent.socket)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("ssh-add is required for the agent tests");
        assert!(status.success(), "ssh-add failed: {status}");
        agent
    }
}

#[cfg(unix)]
impl Drop for TestAgent {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn agent_auth() {
    let dir = trusted_dir().await;
    let agent = TestAgent::start("id_ed25519");
    let mut cfg = test_config("alice", dir.path());
    cfg.auth.use_agent = true;
    cfg.auth.agent_socket = Some(agent.socket.clone());
    let p = Arc::new(ScriptedPrompter::new());
    connect(cfg, &p).await.unwrap().disconnect().await;
    assert_eq!(p.calls().passphrase, 0);
    assert_eq!(p.calls().password, 0);
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn agent_missing_falls_through() {
    let dir = trusted_dir().await;
    let mut cfg = test_config("bob", dir.path());
    cfg.auth.use_agent = true;
    cfg.auth.agent_socket = Some(dir.path().join("no-such-agent.sock"));
    cfg.auth.password = Some(SecretString::from("bob-pass"));
    let p = Arc::new(ScriptedPrompter::new());
    connect(cfg, &p).await.unwrap().disconnect().await;
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn order_is_agent_key_password() {
    let dir = trusted_dir().await;
    let agent = TestAgent::start("id_ed25519");
    let mut cfg = test_config("alice", dir.path());
    cfg.auth.use_agent = true;
    cfg.auth.agent_socket = Some(agent.socket.clone());
    // Wrong on purpose: only reaching the password step would fail the login.
    cfg.auth.password = Some(SecretString::from("not-alices-pass"));
    cfg.auth.key = KeySource::File {
        path: fixture("id_ed25519_pp"),
        passphrase: None,
    };
    let p = Arc::new(ScriptedPrompter::new());
    connect(cfg, &p).await.unwrap().disconnect().await;
    assert_eq!(p.calls().password, 0);
    assert_eq!(p.calls().passphrase, 0, "key step must not run after agent");
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn keyboard_interactive_prompted() {
    let dir = trusted_dir().await;
    let mut cfg = test_config("carol", dir.path());
    cfg.auth.allow_keyboard_interactive = true;
    let p = Arc::new(ScriptedPrompter::new().with_kbd_answers(Some(vec!["carol-pass"])));
    connect(cfg, &p).await.unwrap().disconnect().await;
    assert!(p.calls().kbd >= 1);
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn keyboard_interactive_uses_configured_password() {
    let dir = trusted_dir().await;
    let mut cfg = test_config("carol", dir.path());
    cfg.auth.allow_keyboard_interactive = true;
    cfg.auth.password = Some(SecretString::from("carol-pass"));
    let p = Arc::new(ScriptedPrompter::new());
    connect(cfg, &p).await.unwrap().disconnect().await;
    assert_eq!(p.calls().kbd, 0);
    assert_eq!(p.calls().password, 0);
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn keyboard_interactive_cancelled() {
    let dir = trusted_dir().await;
    let mut cfg = test_config("carol", dir.path());
    cfg.auth.allow_keyboard_interactive = true;
    let p = Arc::new(ScriptedPrompter::new().with_kbd_answers(None));
    assert_err(connect(cfg, &p).await, ErrorKind::Auth, "Cancelled by user");
    assert_eq!(p.calls().kbd, 1);
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn keyboard_interactive_disabled_is_not_tried() {
    let dir = trusted_dir().await;
    let cfg = test_config("carol", dir.path());
    let p = Arc::new(ScriptedPrompter::new().with_kbd_answers(Some(vec!["carol-pass"])));
    let e = assert_err(
        connect(cfg, &p).await,
        ErrorKind::Auth,
        "Authentication failed",
    );
    assert_eq!(p.calls().kbd, 0);
    let detail = e.detail.unwrap_or_default();
    assert!(detail.contains("keyboard-interactive"), "{detail}");
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn keyboard_interactive_wrong_answer_fails() {
    let dir = trusted_dir().await;
    let mut cfg = test_config("carol", dir.path());
    cfg.auth.allow_keyboard_interactive = true;
    let p = Arc::new(ScriptedPrompter::new().with_kbd_answers(Some(vec!["nope"])));
    let e = assert_err(
        connect(cfg, &p).await,
        ErrorKind::Auth,
        "Authentication failed",
    );
    let detail = e.detail.unwrap_or_default();
    assert!(detail.contains("tried: keyboard-interactive"), "{detail}");
    assert!(!detail.contains("nope"), "{detail}");
}
