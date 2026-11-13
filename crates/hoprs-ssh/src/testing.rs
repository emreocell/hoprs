//! Helpers for the Docker integration tests (`docker/compose.yml`). Test-only.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};

use crate::config::{KeySource, SshConfig};

/// Host port the test `sshd` container listens on.
pub const SSH_PORT: u16 = 2222;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Compose project name; matches `name:` in `docker/compose.yml`.
const PROJECT: &str = "hoprs-test";

fn compose(args: &[&str]) -> ExitStatus {
    compose_with_env(&[], args)
}

fn compose_with_env(env: &[(&str, &str)], args: &[&str]) -> ExitStatus {
    let file = repo_root().join("docker/compose.yml");
    Command::new("docker")
        .envs(env.iter().copied())
        .arg("compose")
        .arg("-p")
        .arg(PROJECT)
        .arg("-f")
        .arg(&file)
        .args(args)
        .stdin(Stdio::null())
        .status()
        .expect("failed to run docker")
}

/// Runs `docker compose -p hoprs-test -f <repo>/docker/compose.yml <args>`; panics if it fails.
pub fn docker(args: &[&str]) {
    let status = compose(args);
    assert!(status.success(), "docker compose {args:?} failed: {status}");
}

/// Like [`docker`], with extra environment variables for compose's `${VAR}` interpolation.
pub fn docker_with_env(env: &[(&str, &str)], args: &[&str]) {
    let status = compose_with_env(env, args);
    assert!(
        status.success(),
        "docker compose {args:?} with {env:?} failed: {status}"
    );
}

/// Kills every per-connection sshd process, dropping client connections.
/// Having no session to kill is not an error.
///
/// Matches every sshd process title except the listener's (`sshd: /usr/sbin/sshd ...`):
/// `sshd: bob [priv]`, `sshd: bob` (authenticated, no command yet), `sshd: bob@notty`.
/// `sshd: .*@` alone would miss connections that never ran a command or subsystem.
/// Assumes OpenSSH < 9.8 (the container runs Debian bookworm's 9.2): from 9.8 on,
/// per-connection processes are titled `sshd-session: ...` and this pattern must change.
pub fn kill_ssh_sessions() {
    let args = ["exec", "-T", "sshd", "pkill", "-f", "^sshd: [^/]"];
    let status = compose(&args);
    // pkill exits 1 when nothing matched.
    assert!(
        matches!(status.code(), Some(0 | 1)),
        "docker compose {args:?} failed: {status}"
    );
}

/// Config for `user` on the test container, with `known_hosts` inside `known_hosts_dir`.
/// No agent, no key, no keyboard-interactive.
pub fn test_config(user: &str, known_hosts_dir: &Path) -> SshConfig {
    let mut cfg = SshConfig::new("127.0.0.1", SSH_PORT, user);
    cfg.known_hosts = known_hosts_dir.join("known_hosts");
    cfg.auth.use_agent = false;
    cfg.auth.key = KeySource::None;
    cfg.auth.allow_keyboard_interactive = false;
    cfg
}

/// Path of a file under `docker/fixtures`.
pub fn fixture(name: &str) -> PathBuf {
    repo_root().join("docker/fixtures").join(name)
}

/// The error a session reports when `known_hosts` has another key of the same type for
/// `host` at `line` (what [`crate::host_key_change`] recognizes), for UI tests.
pub fn changed_host_key_error(host: &str, line: usize) -> hoprs_core::Error {
    crate::session::known_hosts_changed(host, line)
}

/// The error a session reports when the server's key differs from the one the session
/// verified on its first connection (`HostKeyChange { line: None }`), for UI tests.
pub fn pinned_key_changed_error(host: &str) -> hoprs_core::Error {
    crate::session::pinned_key_changed(host, "ssh-ed25519", "SHA256:other")
}

/// A shell whose server floods output (32 KiB chunks, as SSH packets carry, for ever) and ignores input, for
/// tests of a slow consumer: the real output pump, without a server. Its output yields
/// `Overflowed` then `Closed` once the consumer lets the pump's limit pile up.
pub fn flooding_shell() -> crate::ShellChannel {
    use std::sync::Arc;

    use async_trait::async_trait;
    use russh::ChannelMsg;

    use crate::shell::{ShellReader, ShellWriter};

    struct Flood;

    #[async_trait]
    impl ShellReader for Flood {
        async fn wait(&mut self) -> Option<ChannelMsg> {
            tokio::task::yield_now().await;
            Some(ChannelMsg::Data {
                data: vec![b'y'; 32 * 1024].into(),
            })
        }
    }

    struct Ignore;

    #[async_trait]
    impl ShellWriter for Ignore {
        async fn data(&self, _: Vec<u8>) -> std::result::Result<(), russh::Error> {
            Ok(())
        }
        async fn window_change(&self, _: u32, _: u32) -> std::result::Result<(), russh::Error> {
            Ok(())
        }
        async fn close(&self) -> std::result::Result<(), russh::Error> {
            Ok(())
        }
    }

    crate::ShellChannel::start(Box::new(Flood), Arc::new(Ignore), None)
}
