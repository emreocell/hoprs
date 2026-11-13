//! Plan 6 Task 6: the technical log panel (poll, filters, Copy all, Clear) and the
//! guarantee that no secret reaches the log.

use std::time::SystemTime;

use hoprs_core::OpenAs;
use tracing::Level;

use super::*;
use crate::app::log::{LogLevel, LogMsg};
use crate::backend::broker::{PromptKind, PromptRequest};
use crate::backend::{BrokerEvent, PromptId};
use crate::logbuf::LogBuffer;
use crate::terminal::input::{Key, Modifiers};

fn log(app: &mut App, msg: LogMsg) -> Vec<Effect> {
    app.update(Message::Log(msg))
}

fn push(buffer: &LogBuffer, level: Level, target: &str, message: &str) {
    buffer.push(
        SystemTime::now(),
        level,
        target.to_owned(),
        message.to_owned(),
    );
}

/// The messages of the lines the panel shows, in order.
fn shown(app: &App) -> Vec<String> {
    (0..app.log.shown_len())
        .map(|i| app.log.shown_row(i).unwrap().line.message.clone())
        .collect()
}

fn scrolls(effects: &[Effect]) -> bool {
    matches!(effects, [Effect::ScrollLogToEnd])
}

#[test]
fn panel_filters_by_level_and_text() {
    let buffer = LogBuffer::new();
    let mut app = App::new();
    app.attach_log(buffer.clone());
    assert!(!app.log.open, "collapsed by default");
    push(
        &buffer,
        Level::ERROR,
        "hoprs_app::backend",
        "cannot open the queue",
    );
    push(
        &buffer,
        Level::WARN,
        "hoprs_app::backend::sessions",
        "keepalive not answered",
    );
    push(
        &buffer,
        Level::INFO,
        "hoprs_app::backend::sessions",
        "connection state changed site=\"Shop\"",
    );
    push(&buffer, Level::DEBUG, "hoprs_ssh", "channel opened");
    push(&buffer, Level::TRACE, "hoprs_ssh", "window adjusted");

    // Closed, the poll copies nothing.
    assert!(log(&mut app, LogMsg::Tick).is_empty());
    assert_eq!(app.log.shown_len(), 0);
    // Opening shows the lines at once, at the newest one.
    assert!(scrolls(&log(&mut app, LogMsg::Toggle)));
    assert!(app.log.open);
    assert_eq!(
        app.log.shown_len(),
        5,
        "Debug shows everything, trace included"
    );

    // Level: at least that severe.
    for (level, expected) in [
        (LogLevel::Error, vec!["cannot open the queue"]),
        (
            LogLevel::Warn,
            vec!["cannot open the queue", "keepalive not answered"],
        ),
        (
            LogLevel::Info,
            vec![
                "cannot open the queue",
                "keepalive not answered",
                "connection state changed site=\"Shop\"",
            ],
        ),
    ] {
        log(&mut app, LogMsg::Level(level));
        assert_eq!(shown(&app), expected, "{level:?}");
    }

    // Text: any case, over the level, target and message; with the level filter.
    log(&mut app, LogMsg::Level(LogLevel::Debug));
    log(&mut app, LogMsg::Filter("  SHOP ".into()));
    assert_eq!(shown(&app), ["connection state changed site=\"Shop\""]);
    log(&mut app, LogMsg::Filter("hoprs_ssh".into()));
    assert_eq!(shown(&app), ["channel opened", "window adjusted"]);
    log(&mut app, LogMsg::Filter("trace".into()));
    assert_eq!(shown(&app), ["window adjusted"], "the level is searchable");
    log(&mut app, LogMsg::Level(LogLevel::Info));
    log(&mut app, LogMsg::Filter("hoprs_ssh".into()));
    assert!(shown(&app).is_empty());
    log(&mut app, LogMsg::Filter("zzz".into()));
    assert!(shown(&app).is_empty());
    assert!(!app.log.is_empty(), "filtered out, not gone");

    // New lines arrive on the tick, through the filters; only the new ones are copied.
    log(&mut app, LogMsg::Filter("queue".into()));
    log(&mut app, LogMsg::Level(LogLevel::Debug));
    assert_eq!(shown(&app), ["cannot open the queue"]);
    push(&buffer, Level::INFO, "hoprs_app", "queue resumed");
    push(&buffer, Level::INFO, "hoprs_app", "unrelated");
    assert!(
        scrolls(&log(&mut app, LogMsg::Tick)),
        "follows the newest line"
    );
    assert_eq!(shown(&app), ["cannot open the queue", "queue resumed"]);
    assert!(log(&mut app, LogMsg::Tick).is_empty(), "nothing new");
    // Scrolled up, new lines don't move the list.
    log(
        &mut app,
        LogMsg::Scrolled {
            offset_y: 0.0,
            at_end: false,
        },
    );
    push(&buffer, Level::INFO, "hoprs_app", "queue paused");
    assert!(log(&mut app, LogMsg::Tick).is_empty());
    assert_eq!(shown(&app).len(), 3);

    // Copy all: the shown lines, one per line, oldest first.
    let effects = log(&mut app, LogMsg::CopyAll);
    let [Effect::WriteClipboard(text)] = &effects[..] else {
        panic!("{effects:?}");
    };
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 3, "{text}");
    assert!(
        lines[0].ends_with("ERROR hoprs_app::backend cannot open the queue"),
        "{text}"
    );
    assert!(
        lines[1].ends_with("INFO  hoprs_app queue resumed"),
        "{text}"
    );
    assert!(lines[2].ends_with("queue paused"), "{text}");
    // `HH:MM:SS.mmm` first.
    assert_eq!(lines[0].as_bytes()[2], b':');
    assert_eq!(lines[0].as_bytes()[8], b'.');

    // Clear empties the panel; what is logged afterwards still comes.
    log(&mut app, LogMsg::Clear);
    assert!(app.log.is_empty());
    assert_eq!(app.log.shown_len(), 0);
    push(&buffer, Level::WARN, "hoprs_app", "queue lost");
    assert!(scrolls(&log(&mut app, LogMsg::Tick)), "Clear follows again");
    assert_eq!(shown(&app), ["queue lost"]);

    // The panel keeps at most as many lines as the buffer.
    log(&mut app, LogMsg::Filter(String::new()));
    for i in 0..crate::logbuf::CAPACITY + 10 {
        push(&buffer, Level::INFO, "hoprs_app", &format!("bulk {i}"));
    }
    log(&mut app, LogMsg::Tick);
    assert_eq!(app.log.shown_len(), crate::logbuf::CAPACITY);
    assert_eq!(shown(&app)[0], "bulk 10");

    // Hide closes it; nothing is copied while closed.
    log(&mut app, LogMsg::Toggle);
    assert!(!app.log.open);
    push(&buffer, Level::ERROR, "hoprs_app", "while closed");
    log(&mut app, LogMsg::Tick);
    assert!(!shown(&app).contains(&"while closed".to_owned()));
    log(&mut app, LogMsg::Toggle);
    assert_eq!(shown(&app).last().unwrap(), "while closed");

    // The follow rule: at the end within a pixel.
    assert!(crate::app::log::at_end(400.0, 180.0, 580.0));
    assert!(crate::app::log::at_end(399.5, 180.0, 580.0));
    assert!(!crate::app::log::at_end(300.0, 180.0, 580.0));
    assert!(
        crate::app::log::at_end(0.0, 180.0, 60.0),
        "fits: at the end"
    );
}

/// Runs the effects this flow causes against the fake (shells, listings, the answer
/// to the password dialog, the upload's enqueue) and feeds their results back.
async fn drive(app: &mut App, fake: &Arc<FakeBackend>, effects: Vec<Effect>) {
    let backend: Arc<dyn BackendApi> = fake.clone();
    let mut pending = effects;
    while !pending.is_empty() {
        let mut next = Vec::new();
        for effect in pending {
            let messages = match effect {
                Effect::OpenShell {
                    tab,
                    request,
                    session,
                    cols,
                    rows,
                } => vec![Message::ShellOpened {
                    tab,
                    request,
                    result: backend.open_shell(session, cols, rows).await,
                }],
                Effect::ShellInput { tab, shell, data } => {
                    backend.shell_input(shell, data).await.unwrap();
                    vec![Message::Terminal {
                        tab,
                        msg: TerminalMsg::InputSent(shell),
                    }]
                }
                Effect::Enqueue(specs) => {
                    vec![Message::Enqueued(
                        effects::enqueue_all(backend.clone(), specs).await,
                    )]
                }
                Effect::AnswerPrompt { id, answer } => {
                    effects::answer_prompt(backend.clone(), id, answer).await;
                    Vec::new()
                }
                list @ Effect::List { .. } => run_listings(fake, &[list]).await,
                _ => Vec::new(),
            };
            for message in messages {
                next.extend(app.update(message));
            }
        }
        pending = next;
    }
}

/// The whole flow with secrets in it, logged at trace through the subscriber `main`
/// installs: a typed password (saved in the keyring), bytes typed and pasted into the
/// terminal (and echoed back), a file uploaded. None of them reaches the log buffer or the panel.
// The std lock serializes this test with the other subscriber tests; the test's
// current-thread runtime runs everything on this thread, which also keeps the
// thread-local subscriber in force across the awaits.
#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn no_secret_reaches_the_log() {
    const PASSWORD: &str = "pw-Zx81-secret-hoprs";
    const TYPED: &str = "typed-Qv47-hoprs";
    const PASTED: &str = "pasted-Kj52-hoprs";
    const CONTENT: &[u8] = b"content-Wm93-hoprs";
    let secrets = [
        PASSWORD,
        TYPED,
        PASTED,
        std::str::from_utf8(CONTENT).unwrap(),
    ];

    let _lock = crate::subscriber_test_lock();
    let (buffer, capture) = crate::capture_log(&crate::log_filter(Some("trace")));

    let mut s = site("Shop", "shop.example", "deploy", Protocol::Ssh);
    s.ssh.open_as = OpenAs::Both;
    s.remote_dir = Some("/srv".into());
    let fake = Arc::new(FakeBackend::new().with_site(s.clone()));
    fake.remote_fs(&s.id).add_dir("/srv");
    fake.local_fs()
        .add_file("/home/me/notes.txt", CONTENT, SystemTime::UNIX_EPOCH);
    let mut app = loaded(&fake).await;
    app.attach_log(buffer.clone());
    log(&mut app, LogMsg::Toggle);
    app.home = Some(rpath("/home/me"));
    let mut prompts = fake.prompt_events().unwrap();

    // Connect: the session asks for the password, typed in the dialog and saved.
    let release = fake.hold_connect(&s.id);
    let effects = app.update(Message::ConnectSite(s.id.clone()));
    let sites = match effects
        .iter()
        .find(|e| matches!(e, Effect::ConnectSites(_)))
    {
        Some(Effect::ConnectSites(sites)) => sites.clone(),
        _ => panic!("{effects:?}"),
    };
    drive(&mut app, &fake, effects).await;
    let backend: Arc<dyn BackendApi> = fake.clone();
    let connecting = tokio::spawn(effects::connect_sites(backend, sites).collect::<Vec<_>>());
    let mut session = None;
    for _ in 0..100 {
        session = (1..10)
            .map(SessionId)
            .find(|id| fake.session_site(*id).as_ref() == Some(&s.id));
        if session.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let session = session.expect("the session started");
    fake.push_prompt(BrokerEvent::Request(PromptRequest {
        id: PromptId(7),
        session,
        site: s.id.clone(),
        key: "shop.example:22".into(),
        kind: PromptKind::Password {
            user: "deploy".into(),
            host: "shop.example".into(),
        },
    }));
    let event = prompts.recv().await.unwrap();
    app.update(Message::Prompt(event));
    app.update(Message::Dialog(DialogMsg::Input(0, Typed(PASSWORD.into()))));
    app.update(Message::Dialog(DialogMsg::Save(true)));
    let effects = app.update(Message::Dialog(DialogMsg::Submit));
    drive(&mut app, &fake, effects).await;
    release.send(()).unwrap();
    for message in connecting.await.unwrap() {
        let effects = app.update(message);
        drive(&mut app, &fake, effects).await;
    }
    assert_eq!(
        fake.password(&s.id).map(|p| p.expose_secret().to_owned()),
        Some(PASSWORD.to_owned()),
        "the password went through"
    );
    assert_eq!(
        app.tabs.iter().map(|t| t.kind()).collect::<Vec<_>>(),
        [TabKind::Terminal, TabKind::Files]
    );

    // Type into the terminal; the fake echoes it back through the shell's feed.
    let tab = app.tabs[0].id;
    let shell = app.tabs[0]
        .terminal()
        .and_then(|t| t.open_shell())
        .expect("the shell opened");
    let mut feed = Box::pin(effects::shell_feed(fake.as_ref(), tab, shell));
    let none = Modifiers {
        shift: false,
        ctrl: false,
        alt: false,
    };
    for c in TYPED.chars() {
        let effects = app.update(Message::Terminal {
            tab,
            msg: TerminalMsg::Key {
                key: Key::Char(c),
                mods: none,
            },
        });
        drive(&mut app, &fake, effects).await;
    }
    // Keys go out one at a time; a paste goes out whole (one write, one echo).
    let effects = app.update(Message::Terminal {
        tab,
        msg: TerminalMsg::Pasted(Some(PASTED.to_owned())),
    });
    drive(&mut app, &fake, effects).await;
    let mut echoed = Vec::new();
    while !String::from_utf8_lossy(&echoed).contains(TYPED)
        || !String::from_utf8_lossy(&echoed).contains(PASTED)
    {
        let message = tokio::time::timeout(Duration::from_secs(5), feed.next())
            .await
            .expect("the echo arrives")
            .expect("the shell runs");
        if let Message::TerminalOutput { bytes, .. } = &message {
            echoed.extend_from_slice(bytes);
        }
        let effects = app.update(message);
        drive(&mut app, &fake, effects).await;
    }

    // Upload the file from the files tab's local pane.
    app.update(Message::SelectTab(1));
    let files = app.tabs[1].id;
    let pane = app.tabs[1].pane(Side::Local).unwrap();
    let row = usize::from(pane.has_up())
        + pane
            .entries
            .iter()
            .position(|e| e.name == "notes.txt")
            .expect("the local pane lists the file");
    app.update(pane_msg(files, Side::Local, PaneMsg::Click(row)));
    let effects = app.update(pane_msg(files, Side::Local, PaneMsg::Transfer));
    drive(&mut app, &fake, effects).await;
    let enqueued = fake.enqueued();
    assert_eq!(enqueued.len(), 1, "the upload was queued");
    assert_eq!(enqueued[0].remote.as_str(), "/srv/notes.txt");

    // The capture is live on this thread (the check is not vacuous).
    tracing::info!(target: "hoprs_app", "log-capture-canary");
    log(&mut app, LogMsg::Tick);
    drop(capture);

    let text = buffer.text();
    assert!(text.contains("log-capture-canary"), "{text}");
    let copied = match &log(&mut app, LogMsg::CopyAll)[..] {
        [Effect::WriteClipboard(copied)] => copied.clone(),
        other => panic!("{other:?}"),
    };
    assert!(copied.contains("log-capture-canary"));
    for secret in secrets {
        assert!(!text.contains(secret), "{secret} reached the log: {text}");
        assert!(!copied.contains(secret), "{secret} reached the panel");
    }
}
