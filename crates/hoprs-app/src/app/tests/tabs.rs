//! Plan 6 Task 1: tabs per session. A site's tabs (terminal and files) share its one
//! session: closing one keeps it for the others, its state reaches all of them, and
//! "Open as" decides which tabs a site opens with (Review Focus 5).

use std::path::PathBuf;

use hoprs_core::{ConnectionState, OpenAs, SavedTab, TabKind};
use hoprs_ssh::AuthMethod;

use super::*;
use crate::backend::broker::{PromptKind, PromptRequest};
use crate::backend::fake::Call;
use crate::backend::{BrokerEvent, PromptId};

fn ssh_site(name: &str, open_as: OpenAs) -> Site {
    let mut site = site(name, &format!("{name}.example"), "u", Protocol::Ssh);
    site.ssh.open_as = open_as;
    site
}

fn kinds(app: &App) -> Vec<TabKind> {
    app.tabs.iter().map(|t| t.kind).collect()
}

/// The indices of the site's tabs, in strip order.
fn tabs_of(app: &App, site: &SiteId) -> Vec<usize> {
    (0..app.tabs.len())
        .filter(|&i| &app.tabs[i].site == site)
        .collect()
}

fn closes(effects: &[Effect]) -> Vec<SessionId> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::CloseSession(s) => Some(*s),
            _ => None,
        })
        .collect()
}

/// The tabs that `effects` list a pane of, with the side.
fn listings(effects: &[Effect]) -> Vec<(TabId, Side)> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::List { tab, side, .. } => Some((*tab, *side)),
            _ => None,
        })
        .collect()
}

/// An app with `site` (a Both SSH site unless the caller says otherwise) open and
/// connected through the fake.
async fn connected(site: &Site) -> (App, Arc<FakeBackend>, SessionId) {
    let fake = Arc::new(FakeBackend::new().with_site(site.clone()));
    let mut app = loaded(&fake).await;
    let sites = connect_effect(app.update(Message::ConnectSite(site.id.clone())));
    assert!(drive(&mut app, &fake, sites).await.is_empty());
    let session = app.tabs[0].session().expect("connected");
    (app, fake, session)
}

#[tokio::test]
async fn open_as_both_opens_terminal_then_files_on_one_session() {
    let s = ssh_site("both", OpenAs::Both);
    let fake = Arc::new(FakeBackend::new().with_site(s.clone()));
    let mut app = loaded(&fake).await;

    let effects = app.update(Message::ConnectSite(s.id.clone()));
    assert_eq!(kinds(&app), [TabKind::Terminal, TabKind::Files]);
    assert_eq!(
        app.active_tab,
        Some(0),
        "the terminal comes first and has the focus"
    );
    // Only the files tab lists a folder (its local side, until connected).
    let files = app.tabs[1].id;
    assert_eq!(listings(&effects), [(files, Side::Local)]);
    // One connect for the site, whatever its tabs.
    let sites = connect_effect(effects);
    assert_eq!(sites, std::slice::from_ref(&s.id));

    let backend: Arc<dyn BackendApi> = fake.clone();
    let messages: Vec<Message> = effects::connect_sites(backend, sites).collect().await;
    assert_eq!(messages.len(), 1, "one connect result");
    let effects: Vec<Effect> = messages.into_iter().flat_map(|m| app.update(m)).collect();
    let (t0, t1) = (app.tabs[0].session(), app.tabs[1].session());
    assert!(t0.is_some());
    assert_eq!(t0, t1, "both tabs share the session");
    assert_eq!(
        app.watched_sessions(),
        vec![t0.unwrap()],
        "one feed per session"
    );
    // Connected, only the files tab lists its remote folder.
    assert_eq!(listings(&effects), [(files, Side::Remote)]);

    // Open as Files / Terminal open just that tab.
    let f = ssh_site("files", OpenAs::Files);
    let t = ssh_site("term", OpenAs::Terminal);
    let fake = FakeBackend::new().with_site(f.clone()).with_site(t.clone());
    let mut app = loaded(&fake).await;
    app.update(Message::ConnectSite(f.id.clone()));
    app.update(Message::ConnectSite(t.id.clone()));
    assert_eq!(kinds(&app), [TabKind::Files, TabKind::Terminal]);
    // Clicking a site with tabs again opens no more tabs: it focuses its first one.
    assert!(without_panes(app.update(Message::ConnectSite(f.id.clone()))).is_empty());
    assert_eq!(app.tabs.len(), 2);
    assert_eq!(app.active_tab, Some(0));
}

/// Review Focus 5: a terminal and a files tab of one site share its session; closing
/// either keeps the session for the other.
#[tokio::test]
async fn closing_one_tab_keeps_the_session_for_the_other() {
    let s = ssh_site("both", OpenAs::Both);
    let (mut app, fake, session) = connected(&s).await;
    let (terminal, files) = (app.tabs[0].id, app.tabs[1].id);

    let effects = app.update(Message::CloseTab(terminal));
    assert!(closes(&effects).is_empty(), "{effects:?}");
    assert_eq!(app.tabs.len(), 1);
    assert_eq!(app.tabs[0].id, files);
    assert_eq!(app.tabs[0].session(), Some(session));
    assert_eq!(app.watched_sessions(), vec![session]);
    assert!(!fake.calls().contains(&Call::Close(session)));

    // The other way round: the files tab closes, the terminal keeps the session.
    let (mut app, _fake, session) = connected(&s).await;
    let files = app.tabs[1].id;
    assert!(closes(&app.update(Message::CloseTab(files))).is_empty());
    assert_eq!(kinds(&app), [TabKind::Terminal]);
    assert_eq!(app.tabs[0].session(), Some(session));

    // While connecting: closing one tab abandons nothing; the dialog stays and the
    // result lands in the tab left.
    let fake = Arc::new(FakeBackend::new().with_site(s.clone()));
    let mut app = loaded(&fake).await;
    let sites = connect_effect(app.update(Message::ConnectSite(s.id.clone())));
    let request = PromptRequest {
        id: PromptId(1),
        session: SessionId(1),
        site: s.id.clone(),
        key: "both.example:22".into(),
        kind: PromptKind::Password {
            user: "u".into(),
            host: "both.example".into(),
        },
    };
    app.update(Message::Prompt(BrokerEvent::Request(request)));
    let terminal = app.tabs[0].id;
    // (A dialog blocks the ×: the user answers it first, then closes.)
    app.update(Message::Dialog(DialogMsg::Cancel));
    assert!(closes(&app.update(Message::CloseTab(terminal))).is_empty());
    let again = PromptRequest {
        id: PromptId(2),
        session: SessionId(1),
        site: s.id.clone(),
        key: "both.example:22".into(),
        kind: PromptKind::Password {
            user: "u".into(),
            host: "both.example".into(),
        },
    };
    let effects = app.update(Message::Prompt(BrokerEvent::Request(again)));
    assert!(closes(&effects).is_empty(), "the site is not abandoned");
    assert!(app.dialogs.is_open());
    app.update(Message::Dialog(DialogMsg::Cancel));
    drive(&mut app, &fake, sites).await;
    assert_eq!(kinds(&app), [TabKind::Files]);
    assert!(matches!(app.tabs[0].state, TabState::Connected(_)));
}

#[tokio::test]
async fn closing_the_last_tab_closes_the_session() {
    let s = ssh_site("both", OpenAs::Both);
    let (mut app, _fake, session) = connected(&s).await;
    let (terminal, files) = (app.tabs[0].id, app.tabs[1].id);
    assert!(closes(&app.update(Message::CloseTab(files))).is_empty());
    let effects = app.update(Message::CloseTab(terminal));
    assert_eq!(closes(&effects), [session]);
    assert!(app.tabs.is_empty());
    assert!(app.watched_sessions().is_empty());
    assert_eq!(app.active_tab, None);

    // The last tab of a site still connecting takes its dialogs with it.
    let t = ssh_site("term", OpenAs::Terminal);
    let fake = FakeBackend::new().with_site(t.clone());
    let mut app = loaded(&fake).await;
    app.update(Message::ConnectSite(t.id.clone()));
    let request = PromptRequest {
        id: PromptId(7),
        session: SessionId(9),
        site: t.id.clone(),
        key: "term.example:22".into(),
        kind: PromptKind::Password {
            user: "u".into(),
            host: "term.example".into(),
        },
    };
    app.update(Message::Prompt(BrokerEvent::Request(request)));
    app.update(Message::Dialog(DialogMsg::Cancel));
    let effects = app.update(Message::CloseTab(app.tabs[0].id));
    assert!(app.tabs.is_empty());
    let late = PromptRequest {
        id: PromptId(8),
        session: SessionId(9),
        site: t.id.clone(),
        key: "term.example:22".into(),
        kind: PromptKind::Password {
            user: "u".into(),
            host: "term.example".into(),
        },
    };
    let mut effects = effects;
    effects.extend(app.update(Message::Prompt(BrokerEvent::Request(late))));
    assert_eq!(closes(&effects), [SessionId(9)]);
    assert!(!app.dialogs.is_open());
}

#[tokio::test]
async fn session_state_reaches_every_tab_of_the_session() {
    let s = ssh_site("both", OpenAs::Both);
    let (mut app, _fake, session) = connected(&s).await;
    let files = app.tabs[1].id;

    app.update(Message::SessionState {
        session,
        state: ConnectionState::Reconnecting { attempt: 2 },
        auth: Some(AuthMethod::Password),
    });
    for tab in &app.tabs {
        assert!(
            matches!(
                tab.conn_status(),
                ConnStatus::Reconnecting { attempt: 2, .. }
            ),
            "{:?}",
            tab.kind
        );
        assert_eq!(tab.auth, Some(AuthMethod::Password));
    }

    let lost = Error::new(ErrorKind::Network, "Connection reset");
    app.update(Message::SessionState {
        session,
        state: ConnectionState::Disconnected(lost.clone()),
        auth: None,
    });
    for tab in &app.tabs {
        assert_eq!(tab.lost_connection(), Some(&lost));
        assert_eq!(tab.failed_endpoint, Some(("both.example".into(), 22)));
    }

    // "Reconnect" from the terminal tab: one reconnect, shown in both tabs.
    let terminal = app.tabs[0].id;
    let effects = app.update(Message::ReconnectSession(terminal));
    assert!(matches!(
        effects[..],
        [Effect::Reconnect { session: s, .. }] if s == session
    ));
    assert!(app.tabs.iter().all(|t| t.reconnecting));
    // The files tab asks too: already in flight.
    assert!(app.update(Message::ReconnectSession(files)).is_empty());

    // The terminal tab closes meanwhile: the result still reaches the files tab.
    app.update(Message::CloseTab(terminal));
    let effects = app.update(Message::Reconnected {
        session,
        result: Ok(()),
    });
    let tab = &app.tabs[0];
    assert_eq!(tab.id, files);
    assert!(!tab.reconnecting);
    assert_eq!(tab.conn_status(), ConnStatus::Connected);
    assert_eq!(listings(&effects), [(files, Side::Remote)]);

    // A failed connect lands in both tabs, and "Close and reconnect" (or Reconnect) from
    // one of them starts every tab of the site over, with one connect.
    let fake = Arc::new(FakeBackend::new().with_site(s.clone()));
    fake.fail_next_connect(&s.id, Error::new(ErrorKind::Network, "Refused"));
    let mut app = loaded(&fake).await;
    let sites = connect_effect(app.update(Message::ConnectSite(s.id.clone())));
    drive(&mut app, &fake, sites).await;
    assert!(
        app.tabs
            .iter()
            .all(|t| matches!(t.state, TabState::Failed(_)))
    );
    assert!(app.tabs.iter().all(|t| t.failed_endpoint.is_some()));
    let effects = app.update(Message::RestartSession(app.tabs[1].id));
    let [Effect::RestartSite { site, .. }] = &effects[..] else {
        panic!("{effects:?}");
    };
    assert_eq!(site, &s.id);
    assert!(
        app.tabs
            .iter()
            .all(|t| t.state == TabState::Connecting && t.failed_endpoint.is_none())
    );
    // The sidebar's retry on a failed site also retries it once for all its tabs.
    let fake = Arc::new(FakeBackend::new().with_site(s.clone()));
    fake.fail_next_connect(&s.id, Error::new(ErrorKind::Network, "Refused"));
    let mut app = loaded(&fake).await;
    let sites = connect_effect(app.update(Message::ConnectSite(s.id.clone())));
    drive(&mut app, &fake, sites).await;
    let sites = connect_effect(app.update(Message::ConnectSite(s.id.clone())));
    assert_eq!(sites, std::slice::from_ref(&s.id));
    assert!(app.tabs.iter().all(|t| t.state == TabState::Connecting));
    drive(&mut app, &fake, sites).await;
    let sessions: Vec<_> = app.tabs.iter().map(|t| t.session()).collect();
    assert!(sessions[0].is_some());
    assert_eq!(sessions[0], sessions[1]);
}

#[tokio::test]
async fn workspace_restores_terminal_tabs() {
    // Saved as Files only, the site's own "Open as" doesn't matter: the workspace
    // restores what it saved.
    let s = ssh_site("prod", OpenAs::Files);
    let mut ws = workspace("Shop", &[&s.id]);
    ws.sites[0].tabs = vec![
        SavedTab {
            kind: TabKind::Terminal,
            remote_dir: None,
            local_dir: None,
        },
        SavedTab {
            kind: TabKind::Files,
            remote_dir: Some("/var/www".into()),
            local_dir: Some(PathBuf::from("/home/me/shop")),
        },
        SavedTab {
            kind: TabKind::Terminal,
            remote_dir: None,
            local_dir: None,
        },
    ];
    let fake = Arc::new(
        FakeBackend::new()
            .with_site(s.clone())
            .with_workspace(ws.clone()),
    );
    let mut app = loaded(&fake).await;
    let sites = connect_effect(app.update(Message::OpenWorkspace(ws.id.clone())));
    assert_eq!(
        sites,
        std::slice::from_ref(&s.id),
        "one connect for the three tabs"
    );
    assert_eq!(
        kinds(&app),
        [TabKind::Terminal, TabKind::Files, TabKind::Terminal]
    );
    assert_eq!(app.tabs[1].remote.path.as_str(), "/var/www");
    assert_eq!(app.tabs[1].local.path.as_str(), "/home/me/shop");
    drive(&mut app, &fake, sites).await;
    let session = app.tabs[0].session().expect("connected");
    assert!(app.tabs.iter().all(|t| t.session() == Some(session)));

    // Closing it saves every tab, terminals included, in strip order.
    let effects = without_panes(app.update(Message::CloseWorkspace(ws.id.clone())));
    let [Effect::SaveTabs { tabs, .. }, Effect::CloseSession(closed)] = &effects[..] else {
        panic!("{effects:?}");
    };
    assert_eq!(*closed, session, "one close for the session");
    assert_eq!(tabs, &vec![(s.id.clone(), ws.sites[0].tabs.clone())]);
    assert!(app.tabs.is_empty());

    // A workspace site that saved no tabs opens per its "Open as".
    let both = ssh_site("both", OpenAs::Both);
    let ws = workspace("New", &[&both.id]);
    let fake = FakeBackend::new()
        .with_site(both.clone())
        .with_workspace(ws.clone());
    let mut app = loaded(&fake).await;
    app.update(Message::OpenWorkspace(ws.id.clone()));
    assert_eq!(kinds(&app), [TabKind::Terminal, TabKind::Files]);
}

#[tokio::test]
async fn ftp_site_ignores_open_as() {
    let mut ftp = site("ftp", "ftp.example", "u", Protocol::Ftp);
    ftp.ssh.open_as = OpenAs::Terminal;
    let mut ws = workspace("w", &[&ftp.id]);
    ws.sites[0].tabs = vec![SavedTab {
        kind: TabKind::Terminal,
        remote_dir: None,
        local_dir: None,
    }];
    let fake = Arc::new(
        FakeBackend::new()
            .with_site(ftp.clone())
            .with_workspace(ws.clone()),
    );
    let mut app = loaded(&fake).await;
    let sites = connect_effect(app.update(Message::ConnectSite(ftp.id.clone())));
    assert_eq!(kinds(&app), [TabKind::Files]);
    drive(&mut app, &fake, sites).await;
    // No terminal for an FTP site, even asked for.
    let tab = app.tabs[0].id;
    assert!(
        app.update(Message::NewTab {
            tab,
            kind: TabKind::Terminal
        })
        .is_empty()
    );
    assert_eq!(app.tabs.len(), 1);
    app.update(Message::CloseTab(tab));

    // A saved terminal tab of an FTP site restores as its files tab.
    app.update(Message::OpenWorkspace(ws.id.clone()));
    assert_eq!(kinds(&app), [TabKind::Files]);
}

#[tokio::test]
async fn delete_site_closes_all_its_tabs() {
    let s = ssh_site("both", OpenAs::Both);
    let other = ssh_site("other", OpenAs::Files);
    let fake = Arc::new(
        FakeBackend::new()
            .with_site(s.clone())
            .with_site(other.clone()),
    );
    let mut app = loaded(&fake).await;
    let sites = connect_effect(app.update(Message::ConnectSite(s.id.clone())));
    drive(&mut app, &fake, sites).await;
    let sites = connect_effect(app.update(Message::ConnectSite(other.id.clone())));
    drive(&mut app, &fake, sites).await;
    let session = app.tabs[0].session().unwrap();
    assert_eq!(app.tabs.len(), 3);

    let effects = app.update(Message::FormDone {
        op: ProfileOp::DeleteSite(s.id.clone()),
        result: Ok(()),
    });
    assert_eq!(closes(&effects), [session], "one close for the session");
    assert!(tabs_of(&app, &s.id).is_empty());
    assert_eq!(app.tabs.len(), 1);
    assert_eq!(app.tabs[0].site, other.id);
    assert_eq!(app.active_tab, Some(0));
}

#[tokio::test]
async fn profile_reload_closes_all_tabs_of_a_vanished_site() {
    let s = ssh_site("both", OpenAs::Both);
    let other = ssh_site("other", OpenAs::Files);
    let fake = Arc::new(
        FakeBackend::new()
            .with_site(s.clone())
            .with_site(other.clone()),
    );
    let mut app = loaded(&fake).await;
    let sites = connect_effect(app.update(Message::ConnectSite(other.id.clone())));
    drive(&mut app, &fake, sites).await;
    let sites = connect_effect(app.update(Message::ConnectSite(s.id.clone())));
    drive(&mut app, &fake, sites).await;
    let session = app.tabs[1].session().unwrap();

    // The host changes: every tab of the site loses the session, closed once.
    let mut moved = s.clone();
    moved.host = "new.example".into();
    fake.save_site(moved).await.unwrap();
    let effects = app.update(profiles_loaded(&app, fake.profiles().await));
    assert_eq!(closes(&effects), [session]);
    for i in tabs_of(&app, &s.id) {
        assert!(matches!(app.tabs[i].state, TabState::Failed(_)));
        assert_eq!(app.tabs[i].conn, None);
    }
    assert_eq!(tabs_of(&app, &s.id).len(), 2);

    // Reconnected, then gone from the profiles (a "Replace" restore): all its tabs go.
    let sites = connect_effect(app.update(Message::ConnectSite(s.id.clone())));
    drive(&mut app, &fake, sites).await;
    let session = app.tabs[1].session().unwrap();
    fake.delete_site(s.id.clone()).await.unwrap();
    let reload = app.load_profiles();
    assert!(matches!(reload, Effect::LoadProfiles(_)));
    let effects = app.update(profiles_loaded(&app, fake.profiles().await));
    assert_eq!(closes(&effects), [session]);
    assert!(tabs_of(&app, &s.id).is_empty());
    assert_eq!(app.tabs.len(), 1);
    assert_eq!(app.tabs[0].site, other.id);
}

#[tokio::test]
async fn new_terminal_and_files_tabs_join_the_session() {
    let s = ssh_site("files", OpenAs::Files);
    let (mut app, _fake, session) = connected(&s).await;
    let first = app.tabs[0].id;

    let effects = app.update(Message::NewTab {
        tab: first,
        kind: TabKind::Terminal,
    });
    assert!(
        without_panes(effects).is_empty(),
        "no connect: the session is reused"
    );
    assert_eq!(kinds(&app), [TabKind::Files, TabKind::Terminal]);
    assert_eq!(app.active_tab, Some(1));
    assert_eq!(app.tabs[1].session(), Some(session));
    assert_eq!(app.tabs[1].conn_status(), ConnStatus::Connected);

    // Another files tab on the same session lists both sides at once.
    let effects = app.update(Message::NewTab {
        tab: first,
        kind: TabKind::Files,
    });
    assert_eq!(
        kinds(&app),
        [TabKind::Files, TabKind::Files, TabKind::Terminal]
    );
    let new = app.tabs[1].id;
    assert_eq!(app.active_tab, Some(1));
    assert_eq!(app.tabs[1].session(), Some(session));
    let mut listed = listings(&effects);
    listed.sort_by_key(|(_, side)| *side == Side::Remote);
    assert_eq!(listed, [(new, Side::Local), (new, Side::Remote)]);
    assert_eq!(app.watched_sessions(), vec![session]);

    // Not on a tab whose site isn't connected.
    let down = ssh_site("down", OpenAs::Files);
    let fake = Arc::new(FakeBackend::new().with_site(down.clone()));
    fake.fail_next_connect(&down.id, Error::new(ErrorKind::Network, "Refused"));
    let mut app = loaded(&fake).await;
    let sites = connect_effect(app.update(Message::ConnectSite(down.id.clone())));
    drive(&mut app, &fake, sites).await;
    let tab = app.tabs[0].id;
    assert!(
        app.update(Message::NewTab {
            tab,
            kind: TabKind::Terminal
        })
        .is_empty()
    );
    assert_eq!(app.tabs.len(), 1);
}
