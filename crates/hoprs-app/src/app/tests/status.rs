//! Task 10: the status bar, connection states and the reconnect banner, saved tabs per
//! workspace, and the orderly shutdown on window close (Review Focus 5).

use std::path::PathBuf;

use hoprs_core::{ConnectionState, SavedTab, TabKind};
use hoprs_ssh::AuthMethod;

use super::*;
use crate::backend::fake::Call;
use crate::ui::status;

fn path(s: &str) -> RemotePath {
    RemotePath::new(s).unwrap()
}

fn files_tab(local: Option<&str>, remote: Option<&str>) -> SavedTab {
    SavedTab {
        kind: TabKind::Files,
        remote_dir: remote.map(str::to_owned),
        local_dir: local.map(PathBuf::from),
    }
}

/// The session of the site's tab, once connected.
fn session_of(app: &App, site: &SiteId) -> SessionId {
    app.tabs[app.tab_of(site).unwrap()]
        .session()
        .expect("the tab is connected")
}

fn state(app: &mut App, session: SessionId, state: ConnectionState) -> Vec<Effect> {
    app.update(Message::SessionState {
        session,
        state,
        auth: None,
    })
}

fn select(app: &mut App, site: &SiteId) {
    let index = app.tab_of(site).unwrap();
    app.update(Message::SelectTab(index));
}

type Feed = std::pin::Pin<Box<dyn futures_util::Stream<Item = Message> + Send>>;

/// The feed's next message (`None` once it ended); never waits more than 5 s.
async fn next(feed: &mut Feed) -> Option<Message> {
    tokio::time::timeout(Duration::from_secs(5), feed.next())
        .await
        .expect("the feed answers")
}

/// The pane's folder and the path of the listing effect for it, if any.
fn listed_path(effects: &[Effect], tab: TabId, side: Side) -> Option<RemotePath> {
    effects.iter().find_map(|e| match e {
        Effect::List {
            tab: t,
            side: s,
            path,
            ..
        } if *t == tab && *s == side => Some(path.clone()),
        _ => None,
    })
}

#[tokio::test]
async fn status_texts_for_each_state() {
    let mut a = site("Shop prod", "203.0.113.4", "deploy", Protocol::Ssh);
    a.port = 22;
    let b = site("Blog", "ftp.blog.net", "writer", Protocol::Ftp);
    let c = site(
        "Shop assets",
        "files.shop.net",
        "up",
        Protocol::FtpsExplicit,
    );
    let ws = workspace("Shop", &[&a.id, &b.id, &c.id]);
    let fake = Arc::new(
        FakeBackend::new()
            .with_site(a.clone())
            .with_site(b.clone())
            .with_site(c.clone())
            .with_workspace(ws.clone()),
    );
    let mut app = loaded(&fake).await;

    // Nothing open: only the state.
    let line = status::line(&app);
    assert_eq!(line.state, strings::NOT_CONNECTED);
    assert_eq!(
        (line.endpoint, line.auth, line.workspace),
        (None, None, None)
    );

    let sites = connect_effect(app.update(Message::OpenWorkspace(ws.id.clone())));
    let line = status::line(&app);
    assert_eq!(line.state, "Connecting…");
    assert_eq!(line.endpoint.as_deref(), Some("deploy@203.0.113.4:22"));
    assert_eq!(line.auth, None, "nothing authenticated yet");
    assert_eq!(line.workspace.as_deref(), Some("0 of 3 sites in Shop"));

    drive(&mut app, &fake, sites).await;
    let session = session_of(&app, &a.id);
    let line = status::line(&app);
    assert_eq!(line.state, "Connected");
    assert_eq!(line.workspace.as_deref(), Some("3 of 3 sites in Shop"));

    // The session's feed: the method that logged in, then each state.
    app.update(Message::SessionState {
        session,
        state: ConnectionState::Connected,
        auth: Some(AuthMethod::Agent {
            key_type: "ed25519".into(),
        }),
    });
    assert_eq!(
        status::line(&app).auth.as_deref(),
        Some("ed25519 via ssh-agent")
    );
    state(
        &mut app,
        session,
        ConnectionState::Reconnecting { attempt: 2 },
    );
    let line = status::line(&app);
    assert_eq!(line.state, "Reconnecting (2/5)");
    assert_eq!(line.workspace.as_deref(), Some("2 of 3 sites in Shop"));
    assert_eq!(
        line.auth.as_deref(),
        Some("ed25519 via ssh-agent"),
        "kept while reconnecting"
    );
    state(&mut app, session, ConnectionState::WaitingForUser);
    assert_eq!(status::line(&app).state, "Waiting for you");
    let lost = Error::new(ErrorKind::Network, "Connection lost");
    state(&mut app, session, ConnectionState::Disconnected(lost));
    assert_eq!(status::line(&app).state, "Disconnected — Connection lost");
    // A state for a session no tab has (closed meanwhile) changes nothing.
    assert!(state(&mut app, SessionId(999), ConnectionState::Connected).is_empty());
    assert_eq!(status::line(&app).state, "Disconnected — Connection lost");

    // The other methods.
    for (method, label) in [
        (AuthMethod::Password, "password"),
        (
            AuthMethod::KeyFile {
                key_type: "rsa".into(),
            },
            "rsa key",
        ),
        (AuthMethod::KeyboardInteractive, "keyboard-interactive"),
    ] {
        app.update(Message::SessionState {
            session,
            state: ConnectionState::Connected,
            auth: Some(method),
        });
        assert_eq!(status::line(&app).auth.as_deref(), Some(label));
    }

    // FTP: the protocol is the method.
    select(&mut app, &b.id);
    let line = status::line(&app);
    assert_eq!(line.auth.as_deref(), Some("FTP (unencrypted)"));
    assert_eq!(line.endpoint.as_deref(), Some("writer@ftp.blog.net:21"));
    select(&mut app, &c.id);
    assert_eq!(status::line(&app).auth.as_deref(), Some("FTPS explicit"));

    // A first connect that failed reads as disconnected, with its reason.
    let d = site("d", "d.example", "u", Protocol::Ssh);
    let fake = Arc::new(FakeBackend::new().with_site(d.clone()));
    let mut app = loaded(&fake).await;
    fake.fail_next_connect(&d.id, Error::new(ErrorKind::Auth, "Authentication failed"));
    let sites = connect_effect(app.update(Message::ConnectSite(d.id.clone())));
    drive(&mut app, &fake, sites).await;
    let line = status::line(&app);
    assert_eq!(line.state, "Disconnected — Authentication failed");
    assert_eq!(line.workspace, None, "no workspace open");
}

#[tokio::test]
async fn reconnect_banner_on_disconnected() {
    let a = site("a", "a.example", "u", Protocol::Ssh);
    let fake = Arc::new(FakeBackend::new().with_site(a.clone()));
    let mut app = loaded(&fake).await;
    let sites = connect_effect(app.update(Message::ConnectSite(a.id.clone())));
    drive(&mut app, &fake, sites).await;
    let session = session_of(&app, &a.id);
    let tab = app.tabs[0].id;
    assert_eq!(app.watched_sessions(), vec![session]);

    // The session's feed reports what the backend's watch says, as it changes.
    let backend: Arc<dyn BackendApi> = fake.clone();
    fake.set_auth(session, AuthMethod::Password);
    let mut feed: Feed = Box::pin(effects::session_feed(backend.clone(), session));
    let first = next(&mut feed).await.unwrap();
    assert!(matches!(
        &first,
        Message::SessionState {
            session: s,
            state: ConnectionState::Connected,
            auth: Some(AuthMethod::Password),
        } if *s == session
    ));
    app.update(first);
    assert_eq!(
        app.tabs[0].lost_connection(),
        None,
        "no banner while connected"
    );
    assert_eq!(status::line(&app).auth.as_deref(), Some("password"));

    // The connection drops for good: the banner shows why, with Reconnect.
    let lost = Error::new(ErrorKind::Network, "Connection lost");
    fake.drop_connection(session, lost.clone());
    let dropped = next(&mut feed).await.unwrap();
    app.update(dropped);
    assert_eq!(app.tabs[0].lost_connection(), Some(&lost));

    // Reconnect goes through the backend's reconnect (one at a time).
    let effects = app.update(Message::ReconnectSession(tab));
    let [Effect::Reconnect { tab: t, session: s }] = &effects[..] else {
        panic!("{effects:?}");
    };
    assert_eq!((*t, *s), (tab, session));
    assert!(app.tabs[0].reconnecting);
    assert!(app.update(Message::ReconnectSession(tab)).is_empty());
    let done = effects::reconnect(backend.clone(), tab, session).await;
    assert!(fake.calls().contains(&Call::Reconnect(session)));
    let effects = app.update(done);
    assert!(!app.tabs[0].reconnecting);
    assert_eq!(app.tabs[0].lost_connection(), None);
    // The remote folder is listed again.
    assert_eq!(
        listed_path(&effects, tab, Side::Remote),
        Some(app.tabs[0].remote.path.clone())
    );
    let up = next(&mut feed).await.unwrap();
    assert!(matches!(
        up,
        Message::SessionState {
            state: ConnectionState::Connected,
            ..
        }
    ));

    // A failed reconnect keeps the banner, with the new reason.
    fake.drop_connection(session, lost.clone());
    let refused = Error::new(ErrorKind::Auth, "Authentication failed");
    fake.fail_next_connect(&a.id, refused.clone());
    app.update(next(&mut feed).await.unwrap());
    app.update(Message::ReconnectSession(tab));
    let failed = effects::reconnect(backend.clone(), tab, session).await;
    app.update(failed);
    assert_eq!(app.tabs[0].lost_connection(), Some(&refused));
    assert!(!app.tabs[0].reconnecting);

    // Closing the tab ends the watch: the app stops asking for it, and the backend's
    // stream ends with the session.
    app.update(Message::CloseTab(tab));
    assert!(app.watched_sessions().is_empty());
    fake.close(session).await;
    loop {
        match next(&mut feed).await {
            Some(Message::SessionState { .. }) => continue,
            None => break,
            Some(other) => panic!("{other:?}"),
        }
    }
    // A reconnect result for a tab that is gone is dropped.
    assert!(
        app.update(Message::Reconnected {
            tab,
            session,
            result: Ok(())
        })
        .is_empty()
    );
}

#[tokio::test]
async fn closing_workspace_saves_tabs() {
    let a = site("a", "a.example", "u", Protocol::Ssh);
    let b = site("b", "b.example", "u", Protocol::Ftp);
    let other = site("other", "o.example", "u", Protocol::Ssh);
    let mut ws = workspace("Shop", &[&a.id, &b.id]);
    // `b` had a saved tab: it has no tab now, so it keeps it.
    let kept = files_tab(Some("/srv/b-local"), Some("/b"));
    ws.sites[1].tabs = vec![kept.clone()];
    let fake = Arc::new(
        FakeBackend::new()
            .with_site(a.clone())
            .with_site(b.clone())
            .with_site(other.clone())
            .with_workspace(ws.clone()),
    );
    let mut app = loaded(&fake).await;
    let sites = connect_effect(app.update(Message::OpenWorkspace(ws.id.clone())));
    drive(&mut app, &fake, sites).await;
    assert_eq!(app.open_workspace, Some(ws.id.clone()));
    let sites = connect_effect(app.update(Message::ConnectSite(other.id.clone())));
    drive(&mut app, &fake, sites).await;
    // `b`'s tab is closed by hand; `a`'s panes move.
    let b_tab = app.tabs[app.tab_of(&b.id).unwrap()].id;
    app.update(Message::CloseTab(b_tab));
    let a_tab = app.tabs[app.tab_of(&a.id).unwrap()].id;
    app.update(Message::Pane {
        tab: a_tab,
        side: Side::Local,
        msg: PaneMsg::Navigate(path("/home/me/shop")),
    });
    app.update(Message::Pane {
        tab: a_tab,
        side: Side::Remote,
        msg: PaneMsg::Navigate(path("/var/www/shop")),
    });
    let a_session = session_of(&app, &a.id);
    let other_session = session_of(&app, &other.id);

    let effects = without_panes(app.update(Message::CloseWorkspace(ws.id.clone())));
    let saved = files_tab(Some("/home/me/shop"), Some("/var/www/shop"));
    let [
        Effect::SaveTabs { workspace, tabs },
        Effect::CloseSession(closed),
    ] = &effects[..]
    else {
        panic!("{effects:?}");
    };
    assert_eq!(workspace, &ws.id);
    assert_eq!(tabs, &vec![(a.id.clone(), vec![saved.clone()])]);
    assert_eq!(*closed, a_session);
    // The workspace's tabs are gone; the other site's tab stays.
    assert_eq!(app.open_workspace, None);
    assert_eq!(app.tabs.len(), 1);
    assert_eq!(app.tabs[0].session(), Some(other_session));
    assert_eq!(status::line(&app).workspace, None);

    // The save goes to the vault; the profiles are read again after it.
    let done = effects::save_tabs(fake.clone(), ws.id.clone(), tabs.clone()).await;
    let stored = &fake.workspaces().await.unwrap()[0];
    assert_eq!(stored.sites[0].tabs, vec![saved]);
    assert_eq!(stored.sites[1].tabs, vec![kept]);
    assert!(matches!(app.update(done)[..], [Effect::LoadProfiles]));

    // A failed save is logged only: nothing to reload, nothing shown.
    assert!(
        app.update(Message::TabsSaved(Err(Error::new(
            ErrorKind::Io,
            "Disk full"
        ))))
        .is_empty()
    );
    // Closing a workspace that is not open does nothing.
    assert!(
        app.update(Message::CloseWorkspace(ws.id.clone()))
            .is_empty()
    );
}

#[tokio::test]
async fn opening_workspace_restores_dirs() {
    let mut a = site("a", "a.example", "u", Protocol::Ssh);
    a.local_dir = Some(PathBuf::from("/site-default/local"));
    a.remote_dir = Some("/site-default/remote".into());
    let mut b = site("b", "b.example", "u", Protocol::Ftp);
    b.local_dir = Some(PathBuf::from("/b-local"));
    let c = site("c", "c.example", "u", Protocol::Ssh);
    let mut ws = workspace("Shop", &[&a.id, &b.id, &c.id]);
    ws.sites[0].tabs = vec![files_tab(Some("/home/me/shop"), Some("/var/www/shop"))];
    // Only a remote folder saved: the local side uses the site's.
    ws.sites[1].tabs = vec![files_tab(None, Some("/pub"))];
    // A terminal tab names no folder for the files tab.
    ws.sites[2].tabs = vec![SavedTab {
        kind: TabKind::Terminal,
        remote_dir: Some("/ignored".into()),
        local_dir: None,
    }];
    let other = workspace("Other", &[&a.id]);
    let fake = Arc::new(
        FakeBackend::new()
            .with_site(a.clone())
            .with_site(b.clone())
            .with_site(c.clone())
            .with_workspace(ws.clone())
            .with_workspace(other.clone()),
    );
    let mut app = loaded(&fake).await;
    app.home = Some(path("/home/me"));

    let effects = app.update(Message::OpenWorkspace(ws.id.clone()));
    let tab = |app: &App, site: &SiteId| app.tabs[app.tab_of(site).unwrap()].clone();
    let (ta, tb, tc) = (tab(&app, &a.id), tab(&app, &b.id), tab(&app, &c.id));
    assert_eq!(ta.local.path, path("/home/me/shop"));
    assert_eq!(ta.remote.path, path("/var/www/shop"));
    assert_eq!(tb.local.path, path("/b-local"));
    assert_eq!(tb.remote.path, path("/pub"));
    assert_eq!(tc.local.path, path("/home/me"));
    assert_eq!(tc.remote.path, RemotePath::root());
    // The local listings start at the restored folders.
    assert_eq!(
        listed_path(&effects, ta.id, Side::Local),
        Some(path("/home/me/shop"))
    );
    // Once connected, the remote side lists its restored folder.
    let sites = connect_effect(effects);
    let backend: Arc<dyn BackendApi> = fake.clone();
    let messages: Vec<Message> = effects::connect_sites(backend, sites).collect().await;
    let effects: Vec<Effect> = messages.into_iter().flat_map(|m| app.update(m)).collect();
    assert_eq!(
        listed_path(&effects, ta.id, Side::Remote),
        Some(path("/var/www/shop"))
    );

    // Opening another workspace saves the open one's tabs first; the tab already open
    // keeps its folders.
    let effects = app.update(Message::OpenWorkspace(other.id.clone()));
    let saves: Vec<_> = effects
        .iter()
        .filter_map(|e| match e {
            Effect::SaveTabs { workspace, .. } => Some(workspace.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(saves, vec![ws.id.clone()]);
    assert_eq!(app.open_workspace, Some(other.id.clone()));
    assert_eq!(tab(&app, &a.id).local.path, path("/home/me/shop"));
}

/// Review Focus 5: closing the window saves the open workspace's tabs, shuts the engine
/// down (its active items reopen paused), closes the sessions, and only then exits.
#[tokio::test]
async fn close_request_saves_then_shuts_down_in_order() {
    let a = site("a", "a.example", "u", Protocol::Ssh);
    let b = site("b", "b.example", "u", Protocol::Ftp);
    let ws = workspace("Shop", &[&a.id, &b.id]);
    let fake = Arc::new(
        FakeBackend::new()
            .with_site(a.clone())
            .with_site(b.clone())
            .with_workspace(ws.clone()),
    );
    let mut app = loaded(&fake).await;
    let sites = connect_effect(app.update(Message::OpenWorkspace(ws.id.clone())));
    drive(&mut app, &fake, sites).await;
    let a_tab = app.tabs[app.tab_of(&a.id).unwrap()].id;
    app.update(Message::Pane {
        tab: a_tab,
        side: Side::Remote,
        msg: PaneMsg::Navigate(path("/var/www")),
    });
    let mut sessions = [session_of(&app, &a.id), session_of(&app, &b.id)];
    sessions.sort();

    let window = iced::window::Id::unique();
    let effects = app.update(Message::CloseRequested(window));
    let [Effect::Shutdown { save, window: w }] = &effects[..] else {
        panic!("{effects:?}");
    };
    assert_eq!(*w, window);
    let (saved_ws, tabs) = save.clone().expect("the open workspace is saved");
    assert_eq!(saved_ws, ws.id);
    assert_eq!(tabs.len(), 2);
    assert_eq!(tabs[0].1[0].remote_dir.as_deref(), Some("/var/www"));
    assert!(app.closing);
    assert_eq!(status::line(&app).state, strings::CLOSING);
    // A second request while closing starts nothing new.
    assert!(app.update(Message::CloseRequested(window)).is_empty());

    let backend: Arc<dyn BackendApi> = fake.clone();
    let outcome = effects::shutdown(backend.clone(), save.clone(), effects::SHUTDOWN_LIMIT).await;
    assert_eq!(outcome, effects::ShutdownOutcome::Done);
    // Exit comes last, once the backend is done.
    let exit = app.update(Message::ShutdownDone(window));
    assert!(matches!(exit[..], [Effect::ExitWindow(w)] if w == window));
    let mut expected = vec![Call::SaveTabs(ws.id.clone()), Call::Shutdown];
    expected.extend(sessions.iter().map(|s| Call::Close(*s)));
    assert_eq!(fake.calls(), expected);
    assert_eq!(
        fake.workspaces().await.unwrap()[0].sites[0].tabs[0]
            .remote_dir
            .as_deref(),
        Some("/var/www")
    );

    // A failed save never stops the shutdown; without a workspace nothing is saved.
    let fake = Arc::new(FakeBackend::new().with_workspace(ws.clone()));
    fake.fail_next_save_tabs(Error::new(ErrorKind::Io, "Disk full"));
    let backend: Arc<dyn BackendApi> = fake.clone();
    let outcome = effects::shutdown(backend, save.clone(), effects::SHUTDOWN_LIMIT).await;
    assert_eq!(outcome, effects::ShutdownOutcome::Done);
    assert_eq!(
        fake.calls(),
        vec![Call::SaveTabs(ws.id.clone()), Call::Shutdown]
    );
    let mut app = loaded(&FakeBackend::new()).await;
    let effects = app.update(Message::CloseRequested(window));
    assert!(matches!(
        &effects[..],
        [Effect::Shutdown { save: None, .. }]
    ));
}

/// The backend gets at most 5 s: a shutdown that hangs never keeps the window open.
#[tokio::test(start_paused = true)]
async fn close_times_out_after_5s() {
    assert_eq!(effects::SHUTDOWN_LIMIT, Duration::from_secs(5));
    let ws = WorkspaceId::new();
    let fake = Arc::new(FakeBackend::new());
    fake.hang_shutdown();
    let backend: Arc<dyn BackendApi> = fake.clone();
    let started = tokio::time::Instant::now();
    let outcome = effects::shutdown(backend, Some((ws, Vec::new())), effects::SHUTDOWN_LIMIT).await;
    let waited = started.elapsed();
    assert_eq!(outcome, effects::ShutdownOutcome::TimedOut);
    assert!(
        waited >= Duration::from_secs(5) && waited < Duration::from_millis(5100),
        "{waited:?}"
    );
    assert_eq!(
        fake.calls().last(),
        Some(&Call::Shutdown),
        "it got that far"
    );

    // The app exits all the same.
    let mut app = App::new();
    let window = iced::window::Id::unique();
    app.update(Message::CloseRequested(window));
    assert!(matches!(
        app.update(Message::ShutdownDone(window))[..],
        [Effect::ExitWindow(w)] if w == window
    ));
}
