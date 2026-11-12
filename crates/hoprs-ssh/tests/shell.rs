//! PTY shell channel against the container in `docker/compose.yml`.

use std::sync::Arc;
use std::time::Duration;

use hoprs_ssh::testing::{fixture, kill_ssh_sessions, test_config};
use hoprs_ssh::{
    KeySource, Prompter, PtyRequest, ScriptedPrompter, ShellChannel, ShellOutput, SshSession,
};
use tempfile::TempDir;
use tokio::time::{Instant, timeout};

const WAIT: Duration = Duration::from_secs(5);

async fn open() -> (SshSession, ShellChannel, TempDir) {
    let dir = TempDir::new().unwrap();
    let mut cfg = test_config("alice", dir.path());
    cfg.auth.key = KeySource::File {
        path: fixture("id_ed25519"),
        passphrase: None,
    };
    let p = Arc::new(ScriptedPrompter::new().accept_host(true));
    let session = SshSession::connect(cfg, p as Arc<dyn Prompter>)
        .await
        .unwrap();
    let shell = session.open_shell(PtyRequest::default()).await.unwrap();
    (session, shell, dir)
}

/// Accumulates `Data` until the text satisfies `pred`; panics on timeout or early close.
async fn read_until(shell: &mut ShellChannel, pred: impl Fn(&str) -> bool) -> String {
    let mut text = String::new();
    let deadline = Instant::now() + WAIT;
    while !pred(&text) {
        match timeout(deadline - Instant::now(), shell.output().recv()).await {
            Ok(Some(ShellOutput::Data(d))) => text.push_str(&String::from_utf8_lossy(&d)),
            Ok(Some(other)) => panic!("unexpected {other:?}; output so far: {text:?}"),
            Ok(None) => panic!("output ended; output so far: {text:?}"),
            Err(_) => panic!("timed out; output so far: {text:?}"),
        }
    }
    text
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn echo_roundtrip() {
    let (_s, mut shell, _d) = open().await;
    shell.write(b"echo hoprs-$((40+2))\n").await.unwrap();
    // The typed command contains "hoprs-$((", so only the result contains "hoprs-42".
    read_until(&mut shell, |t| t.contains("hoprs-42")).await;
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn term_is_xterm_256color() {
    let (_s, mut shell, _d) = open().await;
    shell.write(b"echo term=[$TERM]\n").await.unwrap();
    read_until(&mut shell, |t| t.contains("term=[xterm-256color]")).await;
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn resize_propagates() {
    let (_s, mut shell, _d) = open().await;
    shell.resize(120, 40).await.unwrap();
    // Printed through a format string so the echoed command line cannot match.
    shell
        .write(b"printf 'size=<%s>\\n' \"$(stty size)\"\n")
        .await
        .unwrap();
    read_until(&mut shell, |t| t.contains("size=<40 120>")).await;
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn exit_reports_status() {
    let (_s, mut shell, _d) = open().await;
    shell.write(b"exit 3\n").await.unwrap();
    let mut got_exit = false;
    let deadline = Instant::now() + WAIT;
    loop {
        match timeout(deadline - Instant::now(), shell.output().recv())
            .await
            .expect("timed out waiting for shell end")
        {
            Some(ShellOutput::Data(_)) => {}
            Some(ShellOutput::Exit(code)) => {
                assert!(!got_exit, "second Exit");
                assert_eq!(code, Some(3));
                got_exit = true;
            }
            Some(ShellOutput::Closed) => {
                assert!(got_exit, "Closed before Exit");
                break;
            }
            None => panic!("output ended without Closed"),
        }
    }
    assert!(
        shell.output().recv().await.is_none(),
        "Closed is the last item"
    );
    let e = shell.write(b"x").await.unwrap_err();
    assert_eq!(e.message, "Shell closed");
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn connection_drop_closes_shell() {
    let (_s, mut shell, _d) = open().await;
    shell.write(b"echo ready-$((1+1))\n").await.unwrap();
    read_until(&mut shell, |t| t.contains("ready-2")).await;
    kill_ssh_sessions();
    let deadline = Instant::now() + WAIT;
    loop {
        match timeout(deadline - Instant::now(), shell.output().recv())
            .await
            .expect("timed out waiting for Closed")
        {
            Some(ShellOutput::Closed) => break,
            Some(_) => {}
            None => panic!("output ended without Closed"),
        }
    }
    assert!(shell.output().recv().await.is_none());
}
