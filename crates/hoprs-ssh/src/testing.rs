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
    let file = repo_root().join("docker/compose.yml");
    Command::new("docker")
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
