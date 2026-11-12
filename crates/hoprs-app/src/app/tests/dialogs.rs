//! Task 9: the security and credential dialogs, and the changed host key in the tab
//! (Review Focus 4).

use std::path::PathBuf;

use hoprs_remote::{CertProblem, CertReason};
use hoprs_vault::{MemoryStore, Vault};

use super::*;
use crate::backend::broker::{KbdField, PromptBroker, PromptKind, PromptRequest};
use crate::backend::{BrokerEvent, KnownHostEntry, PromptAnswer, PromptId, VaultHandle};
use crate::ui::dialogs;

fn request(id: u64, session: u64, key: &str, kind: PromptKind) -> PromptRequest {
    PromptRequest {
        id: PromptId(id),
        session: SessionId(session),
        key: key.into(),
        kind,
    }
}

fn host_key(other_key_types: &[&str]) -> PromptKind {
    PromptKind::HostKey {
        host: "h".into(),
        port: 22,
        key_type: "ssh-ed25519".into(),
        fingerprint: "SHA256:new".into(),
        other_key_types: other_key_types.iter().map(|t| (*t).to_owned()).collect(),
    }
}

fn password() -> PromptKind {
    PromptKind::Password {
        user: "u".into(),
        host: "h".into(),
    }
}

fn certificate(problem: CertProblem) -> PromptKind {
    PromptKind::Certificate {
        host: "ftp.example.com".into(),
        port: 21,
        fingerprint: "SHA256:BBBB".into(),
        problem,
    }
}

fn show(app: &mut App, request: PromptRequest) -> Vec<Effect> {
    app.update(Message::Prompt(BrokerEvent::Request(request)))
}

fn dialog_msg(app: &mut App, msg: DialogMsg) -> Vec<Effect> {
    app.update(Message::Dialog(msg))
}

/// The id of the dialog shown.
fn shown(app: &App) -> Option<PromptId> {
    app.dialogs.current.as_ref().map(|d| d.request.id)
}

/// The single answer among `effects`.
fn answer(effects: &[Effect]) -> (PromptId, &PromptAnswer) {
    let answers: Vec<_> = effects
        .iter()
        .filter_map(|e| match e {
            Effect::AnswerPrompt { id, answer } => Some((*id, answer)),
            _ => None,
        })
        .collect();
    match answers[..] {
        [one] => one,
        _ => panic!("expected one answer, got {effects:?}"),
    }
}

/// The broker's next event, if one comes within 100 ms.
async fn next(
    events: &mut tokio::sync::mpsc::UnboundedReceiver<BrokerEvent>,
) -> Result<Option<BrokerEvent>, tokio::time::error::Elapsed> {
    tokio::time::timeout(Duration::from_millis(100), events.recv()).await
}

#[test]
fn prompts_are_queued_one_at_a_time() {
    let mut app = App::new();
    let effects = show(&mut app, request(1, 1, "h:22", host_key(&[])));
    assert!(effects.is_empty(), "a decision has no field to focus");
    // Two more arrive while the first is open: they wait, in order.
    let effects = show(&mut app, request(2, 2, "other:22", password()));
    assert!(effects.is_empty());
    show(
        &mut app,
        request(
            3,
            3,
            "ftp.example.com:21",
            certificate(CertProblem::Untrusted {
                reason: CertReason::Expired,
            }),
        ),
    );
    show(&mut app, request(4, 4, "x:22", password()));
    assert_eq!(shown(&app), Some(PromptId(1)));
    let queued: Vec<_> = app.dialogs.queued().map(|r| r.id).collect();
    assert_eq!(queued, [PromptId(2), PromptId(3), PromptId(4)]);

    // Answering the first shows the next one, and focuses its field.
    let effects = dialog_msg(&mut app, DialogMsg::Trust);
    assert!(matches!(
        answer(&effects),
        (PromptId(1), PromptAnswer::Decision(true))
    ));
    assert!(effects.iter().any(|e| matches!(e, Effect::FocusDialog)));
    assert_eq!(shown(&app), Some(PromptId(2)));

    // A queued request that is withdrawn is gone without an answer.
    let effects = app.update(Message::Prompt(BrokerEvent::Withdrawn(PromptId(3))));
    assert!(effects.is_empty());
    assert_eq!(shown(&app), Some(PromptId(2)));
    let queued: Vec<_> = app.dialogs.queued().map(|r| r.id).collect();
    assert_eq!(queued, [PromptId(4)]);

    // The shown one withdrawn: it closes without an answer, and the next shows.
    let effects = app.update(Message::Prompt(BrokerEvent::Withdrawn(PromptId(2))));
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::AnswerPrompt { .. }))
    );
    assert_eq!(shown(&app), Some(PromptId(4)));

    let effects = dialog_msg(&mut app, DialogMsg::Cancel);
    assert!(matches!(
        answer(&effects),
        (PromptId(4), PromptAnswer::Cancel)
    ));
    assert_eq!(shown(&app), None);
    assert_eq!(app.dialogs.queued().count(), 0);
    // Messages for no dialog are ignored.
    assert!(dialog_msg(&mut app, DialogMsg::Trust).is_empty());
}

/// Review Focus 4: two sessions ask about the same host key at once. Through the real
/// broker and the app, the user sees one dialog; its answer reaches both sessions.
#[tokio::test]
async fn same_host_prompt_answered_once() {
    use hoprs_ssh::Prompter;

    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::open(dir.path(), Arc::new(MemoryStore::new())).unwrap();
    let broker = PromptBroker::new(VaultHandle::new(vault));
    let mut events = broker.take_events().unwrap();
    let site = SiteId::new();
    let ask = |session: u64| {
        let prompter = broker.session(SessionId(session), site.clone(), "h", 22);
        tokio::spawn(async move {
            prompter
                .confirm_host_key("h", 22, "ssh-ed25519", "SHA256:new", &[])
                .await
        })
    };
    let first = ask(1);
    let second = ask(2);

    // Everything the broker sends reaches the app, as the subscription does.
    let mut app = App::new();
    while let Ok(Some(event)) = next(&mut events).await {
        app.update(Message::Prompt(event));
    }
    let id = shown(&app).expect("one dialog");
    assert_eq!(app.dialogs.queued().count(), 0, "not a second one");

    let effects = dialog_msg(&mut app, DialogMsg::Trust);
    let (answered, decision) = answer(&effects);
    assert_eq!(answered, id);
    assert!(broker.answer(answered, decision.clone()));
    for ask in [first, second] {
        let trusted = tokio::time::timeout(Duration::from_secs(1), ask)
            .await
            .expect("the session got the answer")
            .unwrap();
        assert!(trusted);
    }
    assert!(next(&mut events).await.is_err(), "no other dialog");
    assert_eq!(shown(&app), None);
}

#[test]
fn escape_rejects() {
    let mut app = App::new();
    let esc = Message::KeyPressed {
        key: FileKey::Escape,
        shift: false,
    };
    let enter = Message::KeyPressed {
        key: FileKey::Enter,
        shift: false,
    };

    show(&mut app, request(1, 1, "h:22", host_key(&[])));
    // While a dialog is open, the panes get no key.
    assert!(
        app.update(Message::Key {
            key: FileKey::Down,
            shift: false
        })
        .is_empty()
    );
    let effects = app.update(esc.clone());
    assert!(matches!(
        answer(&effects),
        (PromptId(1), PromptAnswer::Cancel)
    ));
    assert_eq!(shown(&app), None);

    // Enter in a trust decision is the safe button: Cancel, never Trust.
    show(
        &mut app,
        request(
            2,
            1,
            "h:21",
            certificate(CertProblem::Untrusted {
                reason: CertReason::UnknownIssuer,
            }),
        ),
    );
    let effects = app.update(enter);
    assert!(matches!(
        answer(&effects),
        (PromptId(2), PromptAnswer::Cancel)
    ));

    // A credential dialog too: Esc refuses, whatever was typed.
    show(&mut app, request(3, 1, "h:22", password()));
    dialog_msg(&mut app, DialogMsg::Input(0, Typed("typed".into())));
    let effects = app.update(esc);
    assert!(matches!(
        answer(&effects),
        (PromptId(3), PromptAnswer::Cancel)
    ));
    assert_eq!(shown(&app), None);
}

/// A connect refused because the host key changed (as the SSH session reports it).
async fn changed_host_key_tab(fake: &Arc<FakeBackend>, site: &Site) -> App {
    let mut app = loaded(fake).await;
    fake.fail_next_connect(
        &site.id,
        hoprs_ssh::testing::changed_host_key_error(&site.host, 3),
    );
    let sites = connect_effect(app.update(Message::ConnectSite(site.id.clone())));
    drive(&mut app, fake, sites).await;
    app
}

/// Review Focus 4: a changed host key is explained in the tab, blocking: no dialog, no
/// way to accept the new key, only "Remove the old entry…" (and Edit site).
#[tokio::test]
async fn changed_host_key_has_no_accept() {
    let s = site("s", "h", "u", Protocol::Ssh);
    let fake = Arc::new(FakeBackend::new().with_site(s.clone()));
    let mut app = changed_host_key_tab(&fake, &s).await;

    let tab = &app.tabs[app.tab_of(&s.id).unwrap()];
    assert!(matches!(tab.state, TabState::Failed(_)));
    assert_eq!(
        tab.host_key_change(),
        Some(hoprs_ssh::HostKeyChange { line: Some(3) })
    );
    assert_eq!(
        app.failure_actions(tab),
        [FailureAction::RemoveOldEntry, FailureAction::EditSite],
        "no Reconnect, and nothing that accepts the key"
    );
    assert!(!app.dialogs.is_open(), "not a dialog");
    // "Trust" has nothing to act on.
    assert!(dialog_msg(&mut app, DialogMsg::Trust).is_empty());

    // Another failure keeps Reconnect.
    let other = site("o", "h2", "u", Protocol::Ssh);
    fake.save_site(other.clone()).await.unwrap();
    app.update(Message::ProfilesLoaded(fake.profiles().await));
    fake.fail_next_connect(&other.id, Error::new(ErrorKind::Network, "down"));
    let sites = connect_effect(app.update(Message::ConnectSite(other.id.clone())));
    drive(&mut app, &fake, sites).await;
    let tab = &app.tabs[app.tab_of(&other.id).unwrap()];
    assert_eq!(tab.host_key_change(), None);
    assert_eq!(
        app.failure_actions(tab),
        [FailureAction::Reconnect, FailureAction::EditSite]
    );
}

fn entry(line: usize, covers_other_hosts: bool) -> KnownHostEntry {
    KnownHostEntry {
        line,
        key_type: "ssh-ed25519".into(),
        covers_other_hosts,
        hashed: false,
    }
}

/// Review Focus 4: "Remove the old entry…" shows the lines first, warns when one also
/// covers other hosts, removes only on confirmation, then connects again.
#[tokio::test]
async fn remove_entry_warns_on_multi_host_line() {
    let mut s = site("s", "h", "u", Protocol::Ssh);
    s.port = 2222;
    let fake = Arc::new(FakeBackend::new().with_site(s.clone()));
    let mut app = changed_host_key_tab(&fake, &s).await;
    let tab = app.tabs[app.tab_of(&s.id).unwrap()].id;
    let index = app.tab_of(&s.id).unwrap();
    fake.set_known_host_entries(vec![entry(3, true), entry(7, false)]);
    let backend: Arc<dyn BackendApi> = fake.clone();

    let effects = app.update(Message::KnownHosts {
        tab,
        msg: KnownHostsMsg::Start,
    });
    let Ok([Effect::KnownHostEntries { host, port, .. }]) = <[Effect; 1]>::try_from(effects) else {
        panic!("one KnownHostEntries effect");
    };
    assert_eq!((host.as_str(), port), ("h", 2222));
    assert_eq!(app.tabs[index].known_hosts, KnownHostsFix::Checking);
    let message = effects::known_host_entries(backend.clone(), tab, host, port).await;
    assert!(app.update(message).is_empty());
    let KnownHostsFix::Confirm(entries) = &app.tabs[index].known_hosts else {
        panic!("{:?}", app.tabs[index].known_hosts);
    };
    let warning = dialogs::known_hosts_warning(entries).expect("a warning");
    assert!(
        warning.starts_with("Line 3 also covers other hosts"),
        "{warning}"
    );
    assert!(!warning.contains('7'), "{warning}");
    assert!(dialogs::known_hosts_warning(&[entry(7, false)]).is_none());
    assert!(fake.known_host_removals().is_empty(), "nothing removed yet");

    // Keep: back to the explanation, nothing removed.
    app.update(Message::KnownHosts {
        tab,
        msg: KnownHostsMsg::Cancel,
    });
    assert_eq!(app.tabs[index].known_hosts, KnownHostsFix::Idle);

    // Again, and confirm: removed, then the tab connects again (the next connect asks
    // about the new key in a dialog).
    let effects = app.update(Message::KnownHosts {
        tab,
        msg: KnownHostsMsg::Start,
    });
    let Ok([Effect::KnownHostEntries { host, port, .. }]) = <[Effect; 1]>::try_from(effects) else {
        panic!("one KnownHostEntries effect");
    };
    let message = effects::known_host_entries(backend.clone(), tab, host, port).await;
    app.update(message);
    let effects = app.update(Message::KnownHosts {
        tab,
        msg: KnownHostsMsg::Confirm,
    });
    let Ok([Effect::RemoveKnownHost { host, port, .. }]) = <[Effect; 1]>::try_from(effects) else {
        panic!("one RemoveKnownHost effect");
    };
    assert_eq!(app.tabs[index].known_hosts, KnownHostsFix::Removing);
    let message = effects::remove_known_host(backend, tab, host, port).await;
    let effects = app.update(message);
    assert_eq!(fake.known_host_removals(), [("h".to_owned(), 2222)]);
    assert_eq!(connect_effect(effects), [s.id.clone()]);
    assert_eq!(app.tabs[index].state, TabState::Connecting);
    assert_eq!(app.tabs[index].known_hosts, KnownHostsFix::Idle);

    // A failed removal shows inline and keeps the action.
    let mut app = changed_host_key_tab(&fake, &s).await;
    let tab = app.tabs[0].id;
    app.update(Message::KnownHosts {
        tab,
        msg: KnownHostsMsg::Start,
    });
    app.update(Message::KnownHostEntries {
        tab,
        result: Ok(vec![entry(3, false)]),
    });
    app.update(Message::KnownHosts {
        tab,
        msg: KnownHostsMsg::Confirm,
    });
    let error = Error::new(ErrorKind::Permission, "Permission denied");
    assert!(
        app.update(Message::KnownHostRemoved {
            tab,
            result: Err(error.clone()),
        })
        .is_empty()
    );
    assert_eq!(app.tabs[0].known_hosts, KnownHostsFix::Failed(error));
    assert!(matches!(app.tabs[0].state, TabState::Failed(_)));
}

/// Review Focus 4: a changed certificate shows the fingerprint trusted before and the one
/// presented now.
#[test]
fn cert_changed_shows_both_fingerprints() {
    let mut app = App::new();
    show(
        &mut app,
        request(
            1,
            1,
            "ftp.example.com:21",
            certificate(CertProblem::Changed {
                pinned: "SHA256:AAAA".into(),
                reason: CertReason::UnknownIssuer,
            }),
        ),
    );
    let content = dialogs::content(app.dialogs.current.as_ref().unwrap());
    assert_eq!(content.title, strings::CERT_CHANGED_TITLE);
    assert!(
        content
            .rows
            .contains(&(strings::LABEL_TRUSTED_BEFORE, "SHA256:AAAA".into()))
    );
    assert!(
        content
            .rows
            .contains(&(strings::LABEL_PRESENTED_NOW, "SHA256:BBBB".into()))
    );
    assert!(
        content
            .rows
            .contains(&(strings::LABEL_HOST, "ftp.example.com:21".into()))
    );
    assert!(content.warning.is_some());
    assert_eq!(content.action, strings::TRUST_CERTIFICATE);
    assert!(content.decision);

    // An untrusted one shows its fingerprint and reason; no "trusted before".
    let untrusted = PromptRequest {
        kind: certificate(CertProblem::Untrusted {
            reason: CertReason::WrongName,
        }),
        ..request(2, 1, "ftp.example.com:21", password())
    };
    dialog_msg(&mut app, DialogMsg::Cancel);
    show(&mut app, untrusted);
    let content = dialogs::content(app.dialogs.current.as_ref().unwrap());
    assert_eq!(content.title, strings::CERT_UNTRUSTED_TITLE);
    assert!(
        content
            .rows
            .contains(&(strings::LABEL_FINGERPRINT, "SHA256:BBBB".into()))
    );
    assert!(
        content
            .rows
            .contains(&(strings::LABEL_REASON, "Issued for a different name".into()))
    );
    assert!(
        !content
            .rows
            .iter()
            .any(|(l, _)| *l == strings::LABEL_TRUSTED_BEFORE)
    );

    // A host key known with another type warns of impersonation.
    dialog_msg(&mut app, DialogMsg::Cancel);
    show(
        &mut app,
        request(3, 1, "h:22", host_key(&["ecdsa-sha2-nistp256"])),
    );
    let content = dialogs::content(app.dialogs.current.as_ref().unwrap());
    assert_eq!(
        content.warning.as_deref(),
        Some(
            "This host is known with a different key type (ecdsa-sha2-nistp256). \
             Someone may be impersonating it."
        )
    );
    assert!(
        content
            .rows
            .contains(&(strings::LABEL_FINGERPRINT, "SHA256:new".into()))
    );
    assert_eq!(content.action, strings::TRUST_AND_CONNECT);
    dialog_msg(&mut app, DialogMsg::Cancel);
    show(&mut app, request(4, 1, "h:22", host_key(&[])));
    assert!(
        dialogs::content(app.dialogs.current.as_ref().unwrap())
            .warning
            .is_none()
    );
}

#[test]
fn cert_reason_text() {
    for (reason, text) in [
        (CertReason::Expired, "Expired"),
        (CertReason::NotYetValid, "Not valid yet"),
        (CertReason::WrongName, "Issued for a different name"),
        (
            CertReason::UnknownIssuer,
            "Unknown authority (self-signed or untrusted issuer)",
        ),
        (CertReason::Other("BadSignature".into()), "BadSignature"),
    ] {
        assert_eq!(strings::cert_reason(&reason), text, "{reason:?}");
    }
}

#[test]
fn password_cleared_after_submit() {
    let mut app = App::new();
    let effects = show(&mut app, request(1, 1, "h:22", password()));
    assert!(matches!(effects[..], [Effect::FocusDialog]), "{effects:?}");
    show(&mut app, request(2, 2, "h2:22", password()));
    dialog_msg(&mut app, DialogMsg::Input(0, Typed("hunter2".into())));
    dialog_msg(&mut app, DialogMsg::Save(true));
    assert!(
        !format!("{app:?}").contains("hunter2"),
        "Debug never shows it"
    );

    let effects = dialog_msg(&mut app, DialogMsg::Submit);
    assert!(!format!("{effects:?}").contains("hunter2"), "{effects:?}");
    let (id, PromptAnswer::Secret { secret, save }) = answer(&effects) else {
        panic!("{effects:?}");
    };
    assert_eq!(
        (id, secret.expose_secret(), *save),
        (PromptId(1), "hunter2", true)
    );
    // The next dialog starts empty, unticked; nothing typed is left in the state.
    let next = app.dialogs.current.as_ref().unwrap();
    assert_eq!(next.request.id, PromptId(2));
    assert_eq!(next.fields, [Typed::default()]);
    assert!(!next.save);
    assert!(app.dialogs.queued().next().is_none());

    // Keyboard-interactive: one answer per prompt, in order; echo shown in clear.
    dialog_msg(&mut app, DialogMsg::Cancel);
    let kind = PromptKind::KeyboardInteractive {
        name: String::new(),
        instructions: "Two steps".into(),
        prompts: vec![
            KbdField {
                text: "Code: ".into(),
                echo: true,
            },
            KbdField {
                text: "Password: ".into(),
                echo: false,
            },
        ],
    };
    show(&mut app, request(3, 1, "h:22", kind));
    let content = dialogs::content(app.dialogs.current.as_ref().unwrap());
    assert_eq!(
        content.fields,
        [("Code:".to_owned(), true), ("Password:".to_owned(), false)]
    );
    assert!(
        !content.save,
        "keyboard-interactive answers are never saved"
    );
    assert_eq!(content.title, strings::KBD_TITLE);
    dialog_msg(&mut app, DialogMsg::Input(0, Typed("123456".into())));
    dialog_msg(&mut app, DialogMsg::Input(1, Typed("s3cret".into())));
    let effects = dialog_msg(&mut app, DialogMsg::Submit);
    let debug = format!("{effects:?}");
    assert!(
        !debug.contains("123456") && !debug.contains("s3cret"),
        "{debug}"
    );
    let (_, PromptAnswer::Responses(responses)) = answer(&effects) else {
        panic!("{effects:?}");
    };
    let got: Vec<_> = responses.iter().map(|r| r.expose_secret()).collect();
    assert_eq!(got, ["123456", "s3cret"]);
    assert_eq!(shown(&app), None);

    // A passphrase offers saving too; its key file is shown.
    show(
        &mut app,
        request(
            4,
            1,
            "h:22",
            PromptKind::Passphrase {
                path: PathBuf::from("/home/u/.ssh/id_ed25519"),
            },
        ),
    );
    let content = dialogs::content(app.dialogs.current.as_ref().unwrap());
    assert!(content.save);
    assert!(
        content
            .rows
            .contains(&(strings::LABEL_KEY_FILE, "/home/u/.ssh/id_ed25519".into()))
    );
}

/// "Save in the system keyring" stores the typed password only once the session it was
/// typed for connects; a failed connect drops it.
#[tokio::test]
async fn save_in_keyring_only_after_success() {
    let ok = site("ok", "h", "u", Protocol::Ssh);
    let bad = site("bad", "h2", "u", Protocol::Ftp);
    let fake = Arc::new(
        FakeBackend::new()
            .with_site(ok.clone())
            .with_site(bad.clone()),
    );
    let mut app = loaded(&fake).await;
    let mut prompts = fake.prompt_events().unwrap();
    let backend: Arc<dyn BackendApi> = fake.clone();
    fake.fail_next_connect(&bad.id, Error::new(ErrorKind::Auth, "Denied"));

    for (s, connects) in [(&ok, true), (&bad, false)] {
        let release = fake.hold_connect(&s.id);
        let sites = connect_effect(app.update(Message::ConnectSite(s.id.clone())));
        let connecting =
            tokio::spawn(effects::connect_sites(backend.clone(), sites).collect::<Vec<_>>());
        // The session exists (it asks for its password while connecting).
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
        let id = PromptId(session.0 + 100);
        fake.push_prompt(BrokerEvent::Request(request(
            id.0,
            session.0,
            "h:22",
            password(),
        )));
        let event = prompts.recv().await.unwrap();
        app.update(Message::Prompt(event));
        dialog_msg(&mut app, DialogMsg::Input(0, Typed("hunter2".into())));
        dialog_msg(&mut app, DialogMsg::Save(true));
        let effects = dialog_msg(&mut app, DialogMsg::Submit);
        let (answered, reply) = answer(&effects);
        assert_eq!(answered, id);
        effects::answer_prompt(backend.clone(), answered, reply.clone()).await;
        assert!(
            fake.password(&s.id).is_none(),
            "not before the connect ends"
        );

        release.send(()).unwrap();
        let messages = connecting.await.unwrap();
        let effects: Vec<Effect> = messages.into_iter().flat_map(|m| app.update(m)).collect();
        let reloads = effects
            .iter()
            .filter(|e| matches!(e, Effect::LoadProfiles))
            .count();
        if connects {
            assert_eq!(
                fake.password(&s.id).map(|p| p.expose_secret().to_owned()),
                Some("hunter2".into())
            );
            assert_eq!(reloads, 1, "the site now shows its saved password");
        } else {
            assert!(
                fake.password(&s.id).is_none(),
                "a failed connect saves nothing"
            );
            assert_eq!(reloads, 0);
        }
    }
    assert!(fake.sites().await.unwrap()[0].has_password);
    assert!(!fake.sites().await.unwrap()[1].has_password);
}

/// The dialogs' subscription: each broker event as `Message::Prompt`, in order; the
/// channel is handed out once, so a second feed is empty.
#[tokio::test]
async fn prompt_feed_delivers_broker_events() {
    let fake = FakeBackend::new();
    let mut feed = Box::pin(effects::prompt_feed(&fake));
    fake.push_prompt(BrokerEvent::Request(request(1, 1, "h:22", password())));
    fake.push_prompt(BrokerEvent::Withdrawn(PromptId(1)));
    let first = feed.next().await;
    assert!(
        matches!(first, Some(Message::Prompt(BrokerEvent::Request(ref r))) if r.id == PromptId(1)),
        "{first:?}"
    );
    let second = feed.next().await;
    assert!(
        matches!(
            second,
            Some(Message::Prompt(BrokerEvent::Withdrawn(PromptId(1))))
        ),
        "{second:?}"
    );
    let mut again = Box::pin(effects::prompt_feed(&fake));
    assert!(again.next().await.is_none());
}
