//! Helpers for the Docker integration tests (`docker/compose.yml`). Test-only.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::{KeySource, SshConfig};

/// Host port the test `sshd` container listens on.
pub const SSH_PORT: u16 = 2222;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Runs `docker compose -f <repo>/docker/compose.yml <args>`; panics if it fails.
pub fn docker(args: &[&str]) {
    let compose = repo_root().join("docker/compose.yml");
    let status = Command::new("docker")
        .arg("compose")
        .arg("-f")
        .arg(&compose)
        .args(args)
        .status()
        .expect("failed to run docker");
    assert!(status.success(), "docker compose {args:?} failed: {status}");
}

/// Kills every authenticated sshd session process, dropping client connections.
pub fn kill_ssh_sessions() {
    docker(&["exec", "-T", "sshd", "pkill", "-f", "sshd: .*@"]);
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
