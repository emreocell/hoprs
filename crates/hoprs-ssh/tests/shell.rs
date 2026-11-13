//! PTY shell channel against the container in `docker/compose.yml`.

use std::sync::Arc;
use std::time::Duration;

use futures_util::{Stream, StreamExt};
use hoprs_core::ErrorKind;
use hoprs_ssh::testing::{fixture, kill_ssh_sessions, test_config};
use hoprs_ssh::{
    KeySource, Prompter, PtyRequest, ScriptedPrompter, ShellHandle, ShellOutput, SshConfig,
    SshSession,
};
use tempfile::TempDir;
use tokio::time::{Instant, timeout};

const WAIT: Duration = Duration::from_secs(5);

fn alice(dir: &TempDir) -> SshConfig {
    let mut cfg = test_config("alice", dir.path());
    cfg.auth.key = KeySource::File {
        path: fixture("id_ed25519"),
        passphrase: None,
    };
    cfg
}

async fn connect(cfg: SshConfig) -> SshSession {
    let p = Arc::new(ScriptedPrompter::new().accept_host(true));
    SshSession::connect(cfg, p as Arc<dyn Prompter>)
        .await
        .unwrap()
}

async fn open() -> (
    SshSession,
    ShellHandle,
    impl Stream<Item = ShellOutput> + Unpin,
    TempDir,
) {
    let dir = TempDir::new().unwrap();
    let session = connect(alice(&dir)).await;
    let (shell, output) = session
        .open_shell(PtyRequest::default())
        .await
        .unwrap()
        .into_output();
    (session, shell, output, dir)
}

/// Accumulates `Data` until the text satisfies `pred`; panics on timeout or early close.
async fn read_until(
    output: &mut (impl Stream<Item = ShellOutput> + Unpin),
    pred: impl Fn(&str) -> bool,
) -> String {
    let mut text = String::new();
    let deadline = Instant::now() + WAIT;
    while !pred(&text) {
        match timeout(deadline - Instant::now(), output.next()).await {
            Ok(Some(ShellOutput::Data(d))) => text.push_str(&String::from_utf8_lossy(&d)),
            Ok(Some(other)) => panic!("unexpected {other:?}; output so far: {text:?}"),
            Ok(None) => panic!("output ended; output so far: {text:?}"),
            Err(_) => panic!("timed out; output so far: {text:?}"),
        }
    }
    text
}

/// Skips output until `Closed`, which must be followed by the end of the stream.
async fn wait_closed(output: &mut (impl Stream<Item = ShellOutput> + Unpin), within: Duration) {
    let deadline = Instant::now() + within;
    loop {
        match timeout(deadline - Instant::now(), output.next())
            .await
            .expect("timed out waiting for Closed")
        {
            Some(ShellOutput::Closed) => break,
            Some(_) => {}
            None => panic!("output ended without Closed"),
        }
    }
    assert!(output.next().await.is_none(), "Closed is the last item");
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn shell_output_stream_is_owned() {
    let dir = TempDir::new().unwrap();
    let session = connect(alice(&dir)).await;
    let shell = session.open_shell(PtyRequest::default()).await.unwrap();
    let (handle, output) = shell.into_output();
    // The stream is owned and 'static: it can move into a task (an iced subscription).
    let reader = tokio::spawn(async move {
        let mut output = output;
        read_until(&mut output, |t| t.contains("hi\r\n")).await;
        output
    });
    handle.clone().write(b"echo h''i\n").await.unwrap();
    let mut output = timeout(WAIT * 2, reader).await.unwrap().unwrap();
    handle.close();
    wait_closed(&mut output, WAIT).await;
    assert_eq!(session.open_channels(), 0, "the PTY slot is released");
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn echo_roundtrip() {
    let (_s, shell, mut output, _d) = open().await;
    shell.write(b"echo hoprs-$((40+2))\n").await.unwrap();
    // The typed command contains "hoprs-$((", so only the result contains "hoprs-42".
    read_until(&mut output, |t| t.contains("hoprs-42")).await;
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn term_is_xterm_256color() {
    let (_s, shell, mut output, _d) = open().await;
    shell.write(b"echo term=[$TERM]\n").await.unwrap();
    read_until(&mut output, |t| t.contains("term=[xterm-256color]")).await;
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn colorterm_env_is_sent_or_ignored() {
    assert_eq!(
        PtyRequest::default().env,
        vec![("COLORTERM".to_owned(), "truecolor".to_owned())]
    );
    let dir = TempDir::new().unwrap();
    let session = connect(alice(&dir)).await;
    // A variable no AcceptEnv lets through: the server refuses it, which must not matter.
    let mut req = PtyRequest::default();
    req.env.push(("HOPRS_REFUSED".to_owned(), "x".to_owned()));
    let (shell, mut output) = session.open_shell(req).await.unwrap().into_output();
    shell
        .write(b"printf 'ct=<%s|%s>\\n' \"$COLORTERM\" \"$HOPRS_REFUSED\"\n")
        .await
        .unwrap();
    let text = read_until(&mut output, |t| {
        t.contains("ct=<truecolor|>") || t.contains("ct=<|>")
    })
    .await;
    eprintln!(
        "COLORTERM {}",
        if text.contains("ct=<truecolor|>") {
            "accepted by the server"
        } else {
            "ignored by the server"
        }
    );
    // The shell still works after the refused variables.
    shell.write(b"echo after-$((2+3))\n").await.unwrap();
    read_until(&mut output, |t| t.contains("after-5")).await;
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn pty_resize_reaches_the_server() {
    let (_s, shell, mut output, _d) = open().await;
    shell.resize(120, 40).await.unwrap();
    // Printed through a format string so the echoed command line cannot match.
    shell
        .write(b"printf 'size=<%s>\\n' \"$(stty size)\"\n")
        .await
        .unwrap();
    read_until(&mut output, |t| t.contains("size=<40 120>")).await;
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn too_many_channels_is_a_clear_error() {
    // Our own limit, below the server's. PTYs stop one short of it (the last slot stays
    // free for the SFTP metadata channel): a limit of 3 takes two.
    let dir = TempDir::new().unwrap();
    let mut cfg = alice(&dir);
    cfg.max_channels = 3;
    let session = connect(cfg).await;
    let first = session.open_shell(PtyRequest::default()).await.unwrap();
    let second = session.open_shell(PtyRequest::default()).await.unwrap();
    let start = Instant::now();
    let e = session.open_shell(PtyRequest::default()).await.unwrap_err();
    assert!(
        start.elapsed() < Duration::from_secs(1),
        "the refusal waited"
    );
    assert_eq!(e.kind, ErrorKind::Protocol, "{e:?}");
    assert_eq!(e.message, "Too many channels on this connection (limit 3)");
    assert!(hoprs_ssh::is_channel_limit(&e));
    // Closing one makes room again.
    let (handle, mut output) = first.into_output();
    handle.close();
    wait_closed(&mut output, WAIT).await;
    let third = session.open_shell(PtyRequest::default()).await.unwrap();
    drop((second, third));

    // The server's own limit (MaxSessions 10 in the container), above ours: its refusal
    // gives the same error, with the server's detail.
    let dir = TempDir::new().unwrap();
    let mut cfg = alice(&dir);
    cfg.max_channels = 20;
    let session = connect(cfg).await;
    let mut shells = Vec::new();
    let e = loop {
        match session.open_shell(PtyRequest::default()).await {
            Ok(shell) => shells.push(shell),
            Err(e) => break e,
        }
        assert!(shells.len() <= 20, "the server never refused");
    };
    assert_eq!(shells.len(), 10, "MaxSessions 10");
    assert_eq!(e.kind, ErrorKind::Protocol, "{e:?}");
    assert_eq!(e.message, "Too many channels on this connection (limit 20)");
    assert!(e.detail.is_some(), "{e:?}");
    assert_eq!(
        session.open_channels(),
        10,
        "the refused channel is not counted"
    );
    // The connection is still fine.
    let (shell, mut output) = shells.pop().unwrap().into_output();
    shell.write(b"echo still-$((1+1))\n").await.unwrap();
    read_until(&mut output, |t| t.contains("still-2")).await;
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn exit_reports_status() {
    let (_s, shell, mut output, _d) = open().await;
    shell.write(b"exit 3\n").await.unwrap();
    let mut got_exit = false;
    let deadline = Instant::now() + WAIT;
    loop {
        match timeout(deadline - Instant::now(), output.next())
            .await
            .expect("timed out waiting for shell end")
        {
            Some(ShellOutput::Data(_)) => {}
            Some(ShellOutput::Overflowed) => panic!("a short shell never overflows"),
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
    assert!(output.next().await.is_none(), "Closed is the last item");
    let e = shell.write(b"x").await.unwrap_err();
    assert_eq!(e.message, "Shell closed");
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn connection_drop_closes_shell() {
    let (_s, shell, mut output, _d) = open().await;
    shell.write(b"echo ready-$((1+1))\n").await.unwrap();
    read_until(&mut output, |t| t.contains("ready-2")).await;
    kill_ssh_sessions();
    wait_closed(&mut output, WAIT).await;
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn close_yields_closed_then_none() {
    let (_s, shell, mut output, _d) = open().await;
    shell.close();
    wait_closed(&mut output, WAIT).await;
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn dropping_the_handles_closes_the_shell() {
    let (session, shell, mut output, _d) = open().await;
    let copy = shell.clone();
    drop(shell);
    copy.write(b"echo alive-$((3+4))\n").await.unwrap();
    read_until(&mut output, |t| t.contains("alive-7")).await;
    drop(copy);
    wait_closed(&mut output, WAIT).await;
    assert_eq!(session.open_channels(), 0);
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn dropping_the_output_closes_the_shell() {
    let (session, shell, output, _d) = open().await;
    drop(output);
    let deadline = Instant::now() + WAIT;
    while session.open_channels() != 0 {
        assert!(Instant::now() < deadline, "the shell stayed open");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(shell.write(b"x").await.unwrap_err().message, "Shell closed");
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn write_after_connection_drop_is_network_error() {
    let (_s, shell, mut output, _d) = open().await;
    kill_ssh_sessions();
    wait_closed(&mut output, WAIT).await;
    let e = shell.write(b"x").await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::Network, "{e:?}");
}

/// Reads `other`'s output until it contains `ok-7`.
async fn exec_ok(session: &SshSession) {
    let mut other = timeout(Duration::from_secs(10), session.open_channel())
        .await
        .expect("open_channel blocked")
        .unwrap();
    timeout(Duration::from_secs(10), other.exec(true, "printf ok-%s 7"))
        .await
        .expect("exec blocked")
        .unwrap();
    let mut out = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !String::from_utf8_lossy(&out).contains("ok-7") {
        match timeout(deadline - Instant::now(), other.wait())
            .await
            .expect("second channel starved")
        {
            Some(russh::ChannelMsg::Data { data }) => out.extend_from_slice(&data),
            Some(_) => {}
            None => panic!("second channel ended early: {out:?}"),
        }
    }
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn stalled_consumer_does_not_block_connection() {
    let (session, shell, mut output, _d) = open().await;
    // 64 MiB of output, far above the pending cap; nobody reads it.
    shell
        .write(b"yes hoprs | head -c 67108864; exit\n")
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_secs(1)).await;

    // The connection must stay usable meanwhile.
    exec_ok(&session).await;

    // Draining eventually reaches Closed (torn down at the cap, or the shell exited).
    wait_closed(&mut output, Duration::from_secs(60)).await;
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn overflow_tears_down_shell_without_blocking_connection() {
    let (session, shell, mut output, _d) = open().await;
    // 192 MiB of output; nothing reads until the pump has given up on this consumer.
    shell
        .write(b"yes hoprs | head -c 201326592; exit\n")
        .await
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    while !shell.overflowed() {
        assert!(Instant::now() < deadline, "pump never overflowed");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // The connection stays usable while the stalled shell is torn down.
    exec_ok(&session).await;

    // Draining yields the buffered data, then Closed, then the end of the stream.
    let mut total = 0usize;
    let mut told = false;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match timeout(deadline - Instant::now(), output.next())
            .await
            .unwrap_or_else(|_| panic!("shell never closed; drained {total} bytes"))
        {
            Some(ShellOutput::Closed) => {
                assert!(
                    told,
                    "the consumer is told about the overflow before Closed"
                );
                break;
            }
            Some(ShellOutput::Overflowed) => told = true,
            Some(ShellOutput::Data(d)) => {
                assert!(!told, "no data after Overflowed");
                total += d.len();
            }
            Some(_) => {}
            None => panic!("output ended without Closed"),
        }
    }
    assert!(total > 0 && total < 192 * 1024 * 1024, "drained {total}");
    assert!(output.next().await.is_none());
}
