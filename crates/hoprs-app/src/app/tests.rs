use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use hoprs_core::{ErrorKind, Protocol, WorkspaceSite};
use secrecy::ExposeSecret;

use super::*;
use crate::backend::BackendApi;
use crate::backend::fake::FakeBackend;
use crate::effects;
use crate::ui::site_form::Password;
use crate::ui::strings;

fn site(name: &str, host: &str, user: &str, protocol: Protocol) -> Site {
    Site::new(name, host, user, protocol)
}

fn workspace(name: &str, sites: &[&SiteId]) -> Workspace {
    Workspace {
        id: WorkspaceId::new(),
        name: name.into(),
        sites: sites
            .iter()
            .map(|id| WorkspaceSite {
                site: (*id).clone(),
                tabs: Vec::new(),
            })
            .collect(),
    }
}

/// An app that loaded what `fake` holds.
async fn loaded(fake: &FakeBackend) -> App {
    let (mut app, effects) = App::boot();
    assert!(matches!(effects[..], [Effect::LoadProfiles]));
    assert!(
        app.update(Message::ProfilesLoaded(fake.profiles().await))
            .is_empty()
    );
    assert_eq!(app.load, Load::Loaded);
    app
}

/// `effects` without the file panes' listings and scrolls (Task 6), for the tests about
/// connections and profiles.
fn without_panes(effects: Vec<Effect>) -> Vec<Effect> {
    effects
        .into_iter()
        .filter(|e| !matches!(e, Effect::List { .. } | Effect::ScrollPane { .. }))
        .collect()
}

/// The single `ConnectSites` effect of `effects` (pane effects aside).
fn connect_effect(effects: Vec<Effect>) -> Vec<SiteId> {
    match <[Effect; 1]>::try_from(without_panes(effects)) {
        Ok([Effect::ConnectSites(sites)]) => sites,
        other => panic!("expected one ConnectSites effect, got {other:?}"),
    }
}

/// Runs a `ConnectSites` batch the way `effects::run` does and feeds every message to
/// the app, in arrival order.
async fn drive(app: &mut App, fake: &Arc<FakeBackend>, sites: Vec<SiteId>) -> Vec<Effect> {
    let backend: Arc<dyn BackendApi> = fake.clone();
    let messages: Vec<Message> = tokio::time::timeout(
        Duration::from_secs(5),
        effects::connect_sites(backend, sites).collect(),
    )
    .await
    .expect("the batch finishes");
    let effects = messages
        .into_iter()
        .flat_map(|message| app.update(message))
        .collect();
    without_panes(effects)
}

fn tab_state<'a>(app: &'a App, site: &SiteId) -> &'a TabState {
    &app.tabs[app.tab_of(site).expect("the site has a tab")].state
}

fn site_form(app: &mut App) -> &mut SiteForm {
    match &mut app.form {
        Some(Form::Site(form)) => form,
        other => panic!("no site form: {other:?}"),
    }
}

fn type_site(app: &mut App, m: SiteFormMsg) -> Vec<Effect> {
    app.update(Message::SiteForm(m))
}

#[test]
fn toggle_queue_flips_and_has_no_effects() {
    let mut app = App::new();
    assert!(app.queue_open);
    assert!(app.update(Message::ToggleQueue).is_empty());
    assert!(!app.queue_open);
    assert!(app.update(Message::ToggleQueue).is_empty());
    assert!(app.queue_open);
}

#[tokio::test]
async fn filter_matches_name_host_user() {
    let shop = site("Shop prod", "shop.example.com", "deploy", Protocol::Ssh);
    let nas = site("NAS", "192.168.1.10", "admin", Protocol::Ssh);
    let blog = site("Blog", "ftp.blog.net", "Writer", Protocol::Ftp);
    let fake = FakeBackend::new()
        .with_site(shop.clone())
        .with_site(nas.clone())
        .with_site(blog.clone());
    let mut app = loaded(&fake).await;

    let names =
        |app: &App| -> Vec<String> { app.visible_sites().iter().map(|s| s.name.clone()).collect() };
    assert_eq!(names(&app), ["Shop prod", "NAS", "Blog"], "file order");
    for (filter, expected) in [
        ("PROD", vec!["Shop prod"]),       // name, any case
        ("192.168", vec!["NAS"]),          // host
        ("writer", vec!["Blog"]),          // user, any case
        ("  EXAMPLE ", vec!["Shop prod"]), // host, trimmed
        ("ADMIN", vec!["NAS"]),            // user
        ("zzz", vec![]),
    ] {
        assert!(app.update(Message::FilterSites(filter.into())).is_empty());
        assert_eq!(names(&app), expected, "filter {filter:?}");
    }
    assert!(!crate::ui::sidebar::site_matches(&blog, "deploy"));
    assert!(crate::ui::sidebar::site_matches(&blog, ""));
}

#[tokio::test]
async fn new_site_validation() {
    let fake = FakeBackend::new();
    let mut app = loaded(&fake).await;
    assert!(app.update(Message::NewSite).is_empty());
    assert!(!site_form(&mut app).editing);

    // Empty host: nothing is saved, the error shows inline.
    assert!(type_site(&mut app, SiteFormMsg::Save).is_empty());
    let errors = site_form(&mut app).errors.clone().unwrap();
    assert_eq!(errors.host, Some(strings::HOST_REQUIRED));
    assert_eq!(errors.port, None, "the default port is valid");

    // Port 0 and non-numbers are refused; the errors follow the typing.
    type_site(&mut app, SiteFormMsg::Host("example.com".into()));
    type_site(&mut app, SiteFormMsg::Port("0".into()));
    let errors = site_form(&mut app).errors.clone().unwrap();
    assert_eq!(errors.host, None);
    assert_eq!(errors.port, Some(strings::PORT_INVALID));
    assert!(type_site(&mut app, SiteFormMsg::Save).is_empty());
    type_site(&mut app, SiteFormMsg::Port("70000".into()));
    assert!(type_site(&mut app, SiteFormMsg::Save).is_empty());
    assert!(!site_form(&mut app).busy);

    type_site(&mut app, SiteFormMsg::Port("2222".into()));
    let effects = type_site(&mut app, SiteFormMsg::Save);
    let [Effect::SaveSite { site, password }] = &effects[..] else {
        panic!("{effects:?}");
    };
    assert_eq!(site.host, "example.com");
    assert_eq!(
        site.name, "example.com",
        "an empty name defaults to the host"
    );
    assert_eq!(site.port, 2222);
    assert!(matches!(password, PasswordChange::Keep));
    let saving = ProfileOp::SaveSite(site.id.clone());
    assert!(site_form(&mut app).busy);
    // A second click while saving does nothing.
    assert!(type_site(&mut app, SiteFormMsg::Save).is_empty());

    // A refused save keeps the form, with the backend's error.
    let refused = Error::new(ErrorKind::Io, "Disk full");
    assert!(
        app.update(Message::FormDone {
            op: saving,
            result: Err(refused.clone())
        })
        .is_empty()
    );
    assert_eq!(site_form(&mut app).failure, Some(refused));
    assert!(!site_form(&mut app).busy);
}

#[tokio::test]
async fn protocol_change_updates_default_port() {
    let mut custom = site("custom", "h", "u", Protocol::Ssh);
    custom.port = 2222;
    let fake = FakeBackend::new().with_site(custom.clone());
    let mut app = loaded(&fake).await;

    app.update(Message::NewSite);
    assert_eq!(site_form(&mut app).port, "22");
    for (protocol, port) in [
        (Protocol::FtpsImplicit, "990"),
        (Protocol::Ftp, "21"),
        (Protocol::FtpsExplicit, "21"),
        (Protocol::Ssh, "22"),
    ] {
        type_site(&mut app, SiteFormMsg::Protocol(protocol));
        assert_eq!(site_form(&mut app).port, port, "{protocol:?}");
    }
    // Once the user typed a port, the protocol no longer replaces it.
    type_site(&mut app, SiteFormMsg::Port("2121".into()));
    type_site(&mut app, SiteFormMsg::Protocol(Protocol::FtpsImplicit));
    assert_eq!(site_form(&mut app).port, "2121");

    // A saved site with a custom port keeps it too.
    app.update(Message::EditSite(custom.id.clone()));
    type_site(&mut app, SiteFormMsg::Protocol(Protocol::Ftp));
    assert_eq!(site_form(&mut app).port, "2222");
}

#[tokio::test]
async fn save_site_with_password_emits_set_secret() {
    let fake = Arc::new(FakeBackend::new());
    let mut app = loaded(&fake).await;
    app.update(Message::NewSite);
    type_site(&mut app, SiteFormMsg::Host("example.com".into()));
    type_site(&mut app, SiteFormMsg::Password(Password("hunter2".into())));
    let effects = type_site(&mut app, SiteFormMsg::Save);

    assert!(
        !format!("{effects:?}").contains("hunter2"),
        "the effect never prints the password"
    );
    let Ok([Effect::SaveSite { site, password }]) = <[Effect; 1]>::try_from(effects) else {
        panic!("one SaveSite effect");
    };
    let PasswordChange::Set(secret) = &password else {
        panic!("{password:?}");
    };
    assert_eq!(secret.expose_secret(), "hunter2");
    assert!(
        site_form(&mut app).password.0.is_empty(),
        "the UI state no longer holds the password"
    );

    // The effect saves the site, then stores the password in the keyring.
    let id = site.id.clone();
    let backend: Arc<dyn BackendApi> = fake.clone();
    effects::save_site(backend.clone(), site, password)
        .await
        .unwrap();
    assert_eq!(
        fake.password(&id).map(|p| p.expose_secret().to_owned()),
        Some("hunter2".into())
    );
    let effects = app.update(Message::FormDone {
        op: ProfileOp::SaveSite(id.clone()),
        result: Ok(()),
    });
    assert!(app.form.is_none(), "a successful save closes the form");
    assert!(matches!(effects[..], [Effect::LoadProfiles]));
    app.update(Message::ProfilesLoaded(fake.profiles().await));

    // Editing it: the password is never shown, only that one is saved; forgetting it
    // deletes the secret.
    app.update(Message::EditSite(id.clone()));
    assert!(site_form(&mut app).has_password());
    assert!(site_form(&mut app).password.0.is_empty());
    type_site(&mut app, SiteFormMsg::ForgetPassword(true));
    let effects = type_site(&mut app, SiteFormMsg::Save);
    let Ok([Effect::SaveSite { site, password }]) = <[Effect; 1]>::try_from(effects) else {
        panic!("one SaveSite effect");
    };
    assert!(matches!(password, PasswordChange::Forget));
    effects::save_site(backend, site, password).await.unwrap();
    assert!(fake.password(&id).is_none());
    assert!(!fake.sites().await.unwrap()[0].has_password);
}

#[tokio::test]
async fn delete_site_needs_confirmation() {
    let doomed = site("doomed", "h", "u", Protocol::Ftp);
    let fake = Arc::new(FakeBackend::new().with_site(doomed.clone()));
    let mut app = loaded(&fake).await;

    // A new site has nothing to delete.
    app.update(Message::NewSite);
    assert!(type_site(&mut app, SiteFormMsg::Delete).is_empty());
    assert!(!site_form(&mut app).confirm_delete);

    // It has an open tab, connected.
    let sites = connect_effect(app.update(Message::ConnectSite(doomed.id.clone())));
    drive(&mut app, &fake, sites).await;
    let TabState::Connected(connected) = tab_state(&app, &doomed.id).clone() else {
        panic!("{:?}", app.tabs);
    };

    app.update(Message::EditSite(doomed.id.clone()));
    assert!(type_site(&mut app, SiteFormMsg::Delete).is_empty());
    assert!(site_form(&mut app).confirm_delete, "asks first");
    assert!(type_site(&mut app, SiteFormMsg::CancelDelete).is_empty());
    assert!(!site_form(&mut app).confirm_delete);
    // Confirming without the question asked does nothing.
    assert!(type_site(&mut app, SiteFormMsg::ConfirmDelete).is_empty());

    type_site(&mut app, SiteFormMsg::Delete);
    let effects = type_site(&mut app, SiteFormMsg::ConfirmDelete);
    let [Effect::DeleteSite(id)] = &effects[..] else {
        panic!("{effects:?}");
    };
    assert_eq!(id, &doomed.id);
    fake.delete_site(id.clone()).await.unwrap();
    let effects = app.update(Message::FormDone {
        op: ProfileOp::DeleteSite(id.clone()),
        result: Ok(()),
    });
    assert!(app.form.is_none());
    // Its tab goes, and so does its session.
    assert!(matches!(
        effects[..],
        [Effect::LoadProfiles, Effect::CloseSession(s)] if s == connected.session
    ));
    assert!(app.tabs.is_empty());
    assert_eq!(app.active_tab, None);
    app.update(Message::ProfilesLoaded(fake.profiles().await));
    assert!(app.sites.is_empty());
}

#[tokio::test]
async fn open_workspace_connects_all_in_parallel() {
    let a = site("a", "a.example", "u", Protocol::Ssh);
    let b = site("b", "b.example", "u", Protocol::Ftp);
    let c = site("c", "c.example", "u", Protocol::FtpsExplicit);
    let ws = workspace("Shop", &[&a.id, &b.id, &c.id]);
    let fake = Arc::new(
        FakeBackend::new()
            .with_site(c.clone())
            .with_site(a.clone())
            .with_site(b.clone())
            .with_workspace(ws.clone()),
    );
    let mut app = loaded(&fake).await;

    // One batch, in the workspace's order; a tab per site at once.
    let sites = connect_effect(app.update(Message::OpenWorkspace(ws.id.clone())));
    assert_eq!(sites, [a.id.clone(), b.id.clone(), c.id.clone()]);
    let tab_sites: Vec<_> = app.tabs.iter().map(|t| t.site.clone()).collect();
    assert_eq!(tab_sites, sites);
    assert!(app.tabs.iter().all(|t| t.state == TabState::Connecting));
    assert_eq!(app.active_tab, Some(0));

    // `a` is slow: `b` and `c` arrive while it still connects.
    let release_a = fake.hold_connect(&a.id);
    let backend: Arc<dyn BackendApi> = fake.clone();
    let mut stream = Box::pin(effects::connect_sites(backend, sites));
    let mut early = Vec::new();
    for _ in 0..2 {
        let message = tokio::time::timeout(Duration::from_secs(5), stream.next())
            .await
            .expect("b and c do not wait for a")
            .unwrap();
        let Message::SiteConnected { site, .. } = &message else {
            panic!("{message:?}");
        };
        early.push(site.clone());
        assert!(without_panes(app.update(message)).is_empty());
    }
    early.sort_by(|x, y| x.0.cmp(&y.0));
    let mut expected = vec![b.id.clone(), c.id.clone()];
    expected.sort_by(|x, y| x.0.cmp(&y.0));
    assert_eq!(early, expected);
    assert!(matches!(tab_state(&app, &b.id), TabState::Connected(_)));
    assert_eq!(tab_state(&app, &a.id), &TabState::Connecting);

    release_a.send(()).unwrap();
    let last = stream.next().await.unwrap();
    assert!(without_panes(app.update(last)).is_empty());
    assert!(stream.next().await.is_none(), "one message per site");
    assert!(matches!(tab_state(&app, &a.id), TabState::Connected(_)));

    // Opening it again reuses the tabs and reconnects nothing that is connected.
    assert!(app.update(Message::OpenWorkspace(ws.id.clone())).is_empty());
    assert_eq!(app.tabs.len(), 3);
}

#[tokio::test]
async fn open_workspace_tolerates_missing_site() {
    let a = site("a", "h", "u", Protocol::Ssh);
    let b = site("b", "h", "u", Protocol::Ftp);
    let gone = SiteId::new();
    let ws = workspace("w", &[&a.id, &gone, &b.id]);
    let fake = Arc::new(
        FakeBackend::new()
            .with_site(a.clone())
            .with_site(b.clone())
            .with_workspace(ws.clone()),
    );
    let mut app = loaded(&fake).await;

    // The sidebar lists it inline as missing.
    let rows = app.workspace_sites(&app.workspaces[0]);
    assert!(matches!(rows[0], WorkspaceRow::Site(s) if s.id == a.id));
    assert!(matches!(&rows[1], WorkspaceRow::Missing(id) if *id == gone));
    assert!(matches!(rows[2], WorkspaceRow::Site(s) if s.id == b.id));

    assert_eq!(strings::workspace_site_count(2, 3), "2 of 3 sites");
    assert_eq!(strings::workspace_site_count(1, 1), "1 site");

    // It is skipped; the others connect.
    let sites = connect_effect(app.update(Message::OpenWorkspace(ws.id.clone())));
    assert_eq!(sites, [a.id.clone(), b.id.clone()]);
    assert_eq!(app.tabs.len(), 2);
    assert!(app.tab_of(&gone).is_none());
    assert!(drive(&mut app, &fake, sites).await.is_empty());
    assert!(matches!(tab_state(&app, &a.id), TabState::Connected(_)));
    assert!(matches!(tab_state(&app, &b.id), TabState::Connected(_)));

    // A site deleted after the sidebar loaded fails alone, in its tab.
    let late = site("late", "h", "u", Protocol::Ftp);
    let fake2 = Arc::new(FakeBackend::new().with_site(late.clone()));
    let mut app2 = loaded(&fake2).await;
    let ws2 = workspace("w2", &[&late.id]);
    app2.workspaces.push(ws2.clone());
    let sites = connect_effect(app2.update(Message::OpenWorkspace(ws2.id)));
    fake2.delete_site(late.id.clone()).await.unwrap();
    drive(&mut app2, &fake2, sites).await;
    let TabState::Failed(failure) = tab_state(&app2, &late.id) else {
        panic!("{:?}", app2.tabs);
    };
    assert_eq!(failure.error.kind, ErrorKind::NotFound);
}

#[tokio::test]
async fn open_workspace_one_keyring_prompt_at_most() {
    let mut with_password = Vec::new();
    for name in ["a", "b", "c"] {
        let mut s = site(name, "h", "u", Protocol::Ssh);
        s.has_password = true;
        with_password.push(s);
    }
    let plain = site("plain", "h", "u", Protocol::Ftp);
    let ids: Vec<&SiteId> = with_password
        .iter()
        .map(|s| &s.id)
        .chain([&plain.id])
        .collect();
    let ws = workspace("w", &ids);
    let mut fake = FakeBackend::new().with_workspace(ws.clone());
    for s in with_password.iter().chain([&plain]) {
        fake = fake.with_site(s.clone());
    }
    let fake = Arc::new(fake);
    fake.lock_keyring();
    let mut app = loaded(&fake).await;

    let sites = connect_effect(app.update(Message::OpenWorkspace(ws.id.clone())));
    drive(&mut app, &fake, sites).await;

    assert_eq!(
        fake.keyring_reads(),
        1,
        "one keyring prompt for the workspace"
    );
    for s in &with_password {
        let TabState::Connected(connected) = tab_state(&app, &s.id) else {
            panic!("{:?}", app.tabs);
        };
        assert!(
            connected.keyring_unavailable,
            "falls back to the password prompt"
        );
    }
    // A site without a stored password is not affected.
    let TabState::Connected(connected) = tab_state(&app, &plain.id) else {
        panic!("{:?}", app.tabs);
    };
    assert!(!connected.keyring_unavailable);

    // The counter is real: separate connects would have asked the keyring each time.
    let mut control = FakeBackend::new();
    for s in &with_password {
        control = control.with_site(s.clone());
    }
    control.lock_keyring();
    for s in &with_password {
        control.connect(s.id.clone()).await.unwrap();
    }
    assert_eq!(control.keyring_reads(), 3);
}

#[tokio::test]
async fn one_site_failure_does_not_block_others() {
    let a = site("a", "h", "u", Protocol::Ssh);
    let b = site("b", "h", "u", Protocol::Ftp);
    let c = site("c", "h", "u", Protocol::Ftp);
    let ws = workspace("w", &[&a.id, &b.id, &c.id]);
    let fake = Arc::new(
        FakeBackend::new()
            .with_site(a.clone())
            .with_site(b.clone())
            .with_site(c.clone())
            .with_workspace(ws.clone()),
    );
    fake.fail_next_connect(&b.id, Error::new(ErrorKind::Auth, "Denied"));
    let mut app = loaded(&fake).await;

    let sites = connect_effect(app.update(Message::OpenWorkspace(ws.id.clone())));
    drive(&mut app, &fake, sites).await;
    assert!(matches!(tab_state(&app, &a.id), TabState::Connected(_)));
    assert!(matches!(tab_state(&app, &c.id), TabState::Connected(_)));
    let TabState::Failed(failure) = tab_state(&app, &b.id) else {
        panic!("{:?}", app.tabs);
    };
    assert_eq!(failure.error.message, "Denied");
    let kept = failure.session.expect("the failed session is kept");

    // Retrying from its tab reconnects only that site, on the same session.
    let sites = connect_effect(app.update(Message::ConnectSite(b.id.clone())));
    assert_eq!(sites, std::slice::from_ref(&b.id));
    assert_eq!(tab_state(&app, &b.id), &TabState::Connecting);
    assert_eq!(app.active_tab, app.tab_of(&b.id));
    drive(&mut app, &fake, sites).await;
    let TabState::Connected(connected) = tab_state(&app, &b.id) else {
        panic!("{:?}", app.tabs);
    };
    assert_eq!(connected.session, kept);
    assert_eq!(app.tabs.len(), 3);
}

#[tokio::test]
async fn unreadable_entries_listed() {
    let entry = UnreadableEntry {
        file: "sites.toml",
        index: 2,
        name: Some("future-site".into()),
        reason: "unknown variant `smb`".into(),
    };
    let nameless = UnreadableEntry {
        file: "workspaces.toml",
        index: 0,
        name: None,
        reason: "bad".into(),
    };
    let fake = FakeBackend::new()
        .with_unreadable(entry.clone())
        .with_unreadable(nameless.clone());
    let mut app = loaded(&fake).await;
    assert_eq!(app.unreadable, vec![entry, nameless]);
    assert_eq!(
        strings::unreadable_name(Some("future-site"), "sites.toml", 2),
        "future-site"
    );
    assert_eq!(
        strings::unreadable_name(None, "workspaces.toml", 0),
        "Entry 1 in workspaces.toml"
    );

    // "Open sites.toml folder" is an effect, never I/O in update.
    let effects = app.update(Message::OpenProfileFolder);
    assert!(matches!(effects[..], [Effect::OpenProfileFolder]));
    let failed = Error::new(ErrorKind::Io, "xdg-open failed");
    assert!(
        app.update(Message::ProfileFolderOpened(Err(failed.clone())))
            .is_empty()
    );
    assert_eq!(app.sidebar_error, Some(failed));
    app.update(Message::ProfileFolderOpened(Ok(())));
    assert_eq!(app.sidebar_error, None);

    // A failed load shows the error with a retry.
    let broken = Error::new(ErrorKind::Io, "Permission denied");
    app.update(Message::ProfilesLoaded(Err(broken.clone())));
    assert_eq!(app.load, Load::Failed(broken));
    assert!(matches!(
        app.update(Message::ReloadProfiles)[..],
        [Effect::LoadProfiles]
    ));
    assert_eq!(app.load, Load::Loading);
}

#[tokio::test]
async fn workspace_form_keeps_file_order_and_unknown_entries() {
    let a = site("a", "h", "u", Protocol::Ssh);
    let b = site("b", "h", "u", Protocol::Ssh);
    let c = site("c", "h", "u", Protocol::Ssh);
    let unknown = SiteId::new();
    let mut ws = workspace("w", &[&c.id, &unknown, &a.id]);
    ws.sites[0].tabs.push(hoprs_core::SavedTab {
        kind: hoprs_core::TabKind::Files,
        remote_dir: Some("/srv".into()),
        local_dir: None,
    });
    let fake = FakeBackend::new()
        .with_site(a.clone())
        .with_site(b.clone())
        .with_site(c.clone())
        .with_workspace(ws.clone());
    let mut app = loaded(&fake).await;

    // New: name required, sites in file order.
    app.update(Message::NewWorkspace);
    let form = |m| Message::WorkspaceForm(m);
    assert!(app.update(form(WorkspaceFormMsg::Save)).is_empty());
    let Some(Form::Workspace(f)) = &app.form else {
        panic!()
    };
    assert_eq!(f.name_error, Some(strings::NAME_REQUIRED));
    app.update(form(WorkspaceFormMsg::Name("New".into())));
    app.update(form(WorkspaceFormMsg::ToggleSite(c.id.clone(), true)));
    app.update(form(WorkspaceFormMsg::ToggleSite(a.id.clone(), true)));
    let effects = app.update(form(WorkspaceFormMsg::Save));
    let [Effect::SaveWorkspace(saved)] = &effects[..] else {
        panic!("{effects:?}");
    };
    let order: Vec<_> = saved.sites.iter().map(|s| s.site.clone()).collect();
    assert_eq!(order, [a.id.clone(), c.id.clone()], "file order");

    // Edit: existing entries stay in place with their tabs; unknown ones are kept.
    app.update(Message::EditWorkspace(ws.id.clone()));
    app.update(form(WorkspaceFormMsg::ToggleSite(a.id.clone(), false)));
    app.update(form(WorkspaceFormMsg::ToggleSite(b.id.clone(), true)));
    let effects = app.update(form(WorkspaceFormMsg::Save));
    let [Effect::SaveWorkspace(saved)] = &effects[..] else {
        panic!("{effects:?}");
    };
    let order: Vec<_> = saved.sites.iter().map(|s| s.site.clone()).collect();
    assert_eq!(order, [c.id.clone(), unknown, b.id.clone()]);
    assert_eq!(saved.sites[0].tabs, ws.sites[0].tabs);

    // Deleting asks first.
    app.update(Message::FormDone {
        op: ProfileOp::SaveWorkspace(ws.id.clone()),
        result: Ok(()),
    });
    assert!(app.form.is_none());
    app.update(Message::EditWorkspace(ws.id.clone()));
    assert!(app.update(form(WorkspaceFormMsg::Delete)).is_empty());
    let effects = app.update(form(WorkspaceFormMsg::ConfirmDelete));
    assert!(matches!(&effects[..], [Effect::DeleteWorkspace(id)] if *id == ws.id));
}

/// A result for a site whose tab is gone closes the session instead of leaking it.
#[tokio::test]
async fn result_without_tab_closes_the_session() {
    let mut app = App::new();
    let connected = Connected {
        session: SessionId(7),
        keyring_unavailable: false,
    };
    let effects = app.update(Message::SiteConnected {
        site: SiteId::new(),
        result: Ok(connected),
    });
    assert!(matches!(effects[..], [Effect::CloseSession(SessionId(7))]));
    assert!(app.tabs.is_empty());
}

/// Editing a site after a failed connect: saving closes the kept session, so Retry
/// connects with the new settings on a new session instead of retrying the old one.
#[tokio::test]
async fn edit_after_failure_retries_with_a_new_session() {
    let a = site("a", "wrong.example", "u", Protocol::Ftp);
    let fake = Arc::new(FakeBackend::new().with_site(a.clone()));
    fake.fail_next_connect(&a.id, Error::new(ErrorKind::Network, "No such host"));
    let mut app = loaded(&fake).await;
    let sites = connect_effect(app.update(Message::ConnectSite(a.id.clone())));
    drive(&mut app, &fake, sites).await;
    let TabState::Failed(failure) = tab_state(&app, &a.id) else {
        panic!("{:?}", app.tabs);
    };
    let kept = failure.session.unwrap();

    app.update(Message::EditSite(a.id.clone()));
    type_site(&mut app, SiteFormMsg::Host("right.example".into()));
    let effects = type_site(&mut app, SiteFormMsg::Save);
    let Ok([Effect::SaveSite { site, password }]) = <[Effect; 1]>::try_from(effects) else {
        panic!("one SaveSite effect");
    };
    effects::save_site(fake.clone(), site, password)
        .await
        .unwrap();
    let effects = without_panes(app.update(Message::FormDone {
        op: ProfileOp::SaveSite(a.id.clone()),
        result: Ok(()),
    }));
    assert!(matches!(
        effects[..],
        [Effect::LoadProfiles, Effect::CloseSession(s)] if s == kept
    ));
    let TabState::Failed(failure) = tab_state(&app, &a.id) else {
        panic!("{:?}", app.tabs);
    };
    assert_eq!(failure.session, None, "the tab forgets the closed session");
    fake.close(kept).await;
    app.update(Message::ProfilesLoaded(fake.profiles().await));

    let sites = connect_effect(app.update(Message::ConnectSite(a.id.clone())));
    drive(&mut app, &fake, sites).await;
    let TabState::Connected(connected) = tab_state(&app, &a.id) else {
        panic!("{:?}", app.tabs);
    };
    assert_ne!(
        connected.session, kept,
        "a new session, from the saved site"
    );
    assert_eq!(fake.closed(), vec![kept]);

    // A connected session is left alone by a later save.
    app.update(Message::EditSite(a.id.clone()));
    type_site(&mut app, SiteFormMsg::Save);
    let effects = without_panes(app.update(Message::FormDone {
        op: ProfileOp::SaveSite(a.id.clone()),
        result: Ok(()),
    }));
    assert!(matches!(effects[..], [Effect::LoadProfiles]));
}

/// The sidebar's "Edit" opens the form without connecting.
#[tokio::test]
async fn edit_site_from_sidebar_does_not_connect() {
    let a = site("a", "h", "u", Protocol::Ssh);
    let fake = FakeBackend::new().with_site(a.clone());
    let mut app = loaded(&fake).await;
    assert!(app.update(Message::EditSite(a.id.clone())).is_empty());
    assert!(app.tabs.is_empty());
    assert!(site_form(&mut app).editing);
    assert_eq!(site_form(&mut app).site_id(), &a.id);
}

/// A save result that arrives after its form was replaced never touches the new form.
#[tokio::test]
async fn late_form_result_leaves_other_form_untouched() {
    let a = site("a", "h", "u", Protocol::Ssh);
    let fake = FakeBackend::new().with_site(a.clone());
    let mut app = loaded(&fake).await;
    app.update(Message::EditSite(a.id.clone()));
    assert_eq!(type_site(&mut app, SiteFormMsg::Save).len(), 1);
    let op = ProfileOp::SaveSite(a.id.clone());

    app.update(Message::NewWorkspace);
    app.update(Message::WorkspaceForm(WorkspaceFormMsg::Name(
        "Draft".into(),
    )));
    let workspace_form = |app: &App| match &app.form {
        Some(Form::Workspace(form)) => (form.name.clone(), form.busy, form.failure.clone()),
        other => panic!("the workspace form was replaced: {other:?}"),
    };

    let effects = app.update(Message::FormDone {
        op: op.clone(),
        result: Err(Error::new(ErrorKind::Io, "Disk full")),
    });
    assert!(effects.is_empty());
    assert_eq!(workspace_form(&app), ("Draft".into(), false, None));
    let effects = app.update(Message::FormDone {
        op: op.clone(),
        result: Ok(()),
    });
    assert!(
        matches!(effects[..], [Effect::LoadProfiles]),
        "the data still reloads"
    );
    assert_eq!(workspace_form(&app), ("Draft".into(), false, None));

    // Nor does it close the same site's form opened again (not waiting for anything).
    app.update(Message::EditSite(a.id.clone()));
    app.update(Message::FormDone { op, result: Ok(()) });
    assert!(site_form(&mut app).editing);
}

// ---- Task 6: file panes ----

use std::time::SystemTime;

use hoprs_core::RemotePath;

use crate::app::files::{Listing, PaneStatus, SortKey};
use crate::backend::{SessionId, Side};

fn rpath(s: &str) -> RemotePath {
    RemotePath::new(s).unwrap()
}

/// The listings among `effects`, as `(side, request, session, path)`.
fn listings(effects: &[Effect]) -> Vec<(Side, RequestId, Option<SessionId>, String)> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::List {
                side,
                request_id,
                session,
                path,
                ..
            } => Some((*side, *request_id, *session, path.to_string())),
            _ => None,
        })
        .collect()
}

/// Runs every listing of `effects` against `fake`, as `effects::run` would, and returns
/// their `Listed` messages in effect order (not yet delivered).
async fn run_listings(fake: &Arc<FakeBackend>, effects: &[Effect]) -> Vec<Message> {
    let mut messages = Vec::new();
    for effect in effects {
        if let Effect::List {
            tab,
            side,
            request_id,
            session,
            path,
        } = effect
        {
            let backend: Arc<dyn BackendApi> = fake.clone();
            messages.push(
                effects::list(backend, *tab, *side, *request_id, *session, path.clone()).await,
            );
        }
    }
    messages
}

/// Delivers messages, running the listings and file operations they cause, until
/// nothing is left.
async fn settle(app: &mut App, fake: &Arc<FakeBackend>, mut pending: Vec<Message>) {
    while !pending.is_empty() {
        let mut next = Vec::new();
        for message in pending {
            let effects = app.update(message);
            next.extend(run_listings(fake, &effects).await);
            next.extend(run_file_ops(fake, &effects).await);
        }
        pending = next;
    }
}

/// Runs every file operation of `effects` against `fake`, as `effects::run` would, and
/// returns all their messages (progress, then the result) in order, not yet delivered.
async fn run_file_ops(fake: &Arc<FakeBackend>, effects: &[Effect]) -> Vec<Message> {
    let mut messages = Vec::new();
    for effect in effects {
        if let Effect::FileOp {
            tab,
            side,
            op,
            session,
            action,
        } = effect
        {
            let backend: Arc<dyn BackendApi> = fake.clone();
            let stream = effects::file_op(backend, *tab, *side, *op, *session, action.clone());
            messages.extend(
                tokio::time::timeout(Duration::from_secs(5), stream.collect::<Vec<_>>())
                    .await
                    .expect("the operation finishes"),
            );
        }
    }
    messages
}

fn pane_msg(tab: TabId, side: Side, msg: PaneMsg) -> Message {
    Message::Pane { tab, side, msg }
}

fn entry_names(app: &App, index: usize, side: Side) -> Vec<String> {
    app.tabs[index]
        .pane(side)
        .entries
        .iter()
        .map(|e| e.name.clone())
        .collect()
}

/// A connected tab for a site whose remote side starts at `/srv`, its local side at the
/// fake home, both listed.
async fn connected_tab(fake: &Arc<FakeBackend>, a: &Site) -> App {
    let mut app = loaded(fake).await;
    app.home = Some(rpath("/home/me"));
    let effects = app.update(Message::ConnectSite(a.id.clone()));
    settle(&mut app, fake, run_listings(fake, &effects).await).await;
    let backend: Arc<dyn BackendApi> = fake.clone();
    let messages: Vec<Message> = effects::connect_sites(backend, vec![a.id.clone()])
        .collect()
        .await;
    settle(&mut app, fake, messages).await;
    app
}

/// Review Focus 1: the user leaves a folder while its listing is in flight. The old
/// listing is not cancelled; its late result never replaces the new folder.
#[tokio::test]
async fn stale_listing_is_ignored() {
    let mut a = site("a", "h", "u", Protocol::Ftp);
    a.remote_dir = Some("/srv".into());
    let fake = Arc::new(FakeBackend::new().with_site(a.clone()));
    let remote = fake.remote_fs(&a.id);
    remote.add_file("/srv/old/stale.txt", b"x", SystemTime::UNIX_EPOCH);
    remote.add_file("/srv/new/fresh.txt", b"y", SystemTime::UNIX_EPOCH);
    fake.local_fs().add_dir("/home/me/work");

    let mut app = loaded(&fake).await;
    app.home = Some(rpath("/home/me"));

    // A new tab lists its local start folder (home) at once; the remote side waits for
    // the connection, then lists the site's folder on its session.
    let effects = app.update(Message::ConnectSite(a.id.clone()));
    let local = listings(&effects);
    assert_eq!(local.len(), 1);
    assert_eq!(local[0].0, Side::Local);
    assert_eq!((local[0].2, local[0].3.as_str()), (None, "/home/me"));
    let tab = app.tabs[0].id;
    let backend: Arc<dyn BackendApi> = fake.clone();
    let mut connected = Vec::new();
    for message in effects::connect_sites(backend, vec![a.id.clone()])
        .collect::<Vec<_>>()
        .await
    {
        connected.extend(app.update(message));
    }
    let remote_list = listings(&connected);
    assert_eq!(remote_list.len(), 1);
    assert_eq!(remote_list[0].0, Side::Remote);
    assert!(
        remote_list[0].2.is_some(),
        "the remote side lists on the session"
    );
    assert_eq!(remote_list[0].3, "/srv");
    settle(&mut app, &fake, run_listings(&fake, &connected).await).await;
    settle(&mut app, &fake, run_listings(&fake, &effects).await).await;
    assert_eq!(entry_names(&app, 0, Side::Remote), ["new", "old"]);
    assert_eq!(entry_names(&app, 0, Side::Local), ["work"]);

    // Into "old"; before it answers, back up with ".." (row 0, the only row while it
    // loads), then into "new" from the breadcrumb: three listings in flight.
    let to_old = app.update(pane_msg(tab, Side::Remote, PaneMsg::Open(2)));
    assert_eq!(app.tabs[0].remote.path, rpath("/srv/old"));
    assert_eq!(app.tabs[0].pane_status(Side::Remote), PaneStatus::Loading);
    let to_up = app.update(pane_msg(tab, Side::Remote, PaneMsg::Open(0)));
    assert_eq!(app.tabs[0].remote.path, rpath("/srv"));
    let to_new = app.update(pane_msg(
        tab,
        Side::Remote,
        PaneMsg::Navigate(rpath("/srv/new")),
    ));
    assert_eq!(app.tabs[0].remote.path, rpath("/srv/new"));
    let (old_id, up_id, new_id) = (
        listings(&to_old)[0].1,
        listings(&to_up)[0].1,
        listings(&to_new)[0].1,
    );
    assert!(
        old_id < up_id && up_id < new_id,
        "each request has a new id"
    );
    // Navigating scrolls the new folder to the top.
    assert!(to_new.iter().any(|e| matches!(
        e,
        Effect::ScrollPane { side: Side::Remote, offset_y, .. } if *offset_y == 0.0
    )));

    // All three listings run to their end (none is cancelled)...
    let old = run_listings(&fake, &to_old).await;
    let up = run_listings(&fake, &to_up).await;
    let new = run_listings(&fake, &to_new).await;
    assert!(matches!(&old[0], Message::Listed { result: Ok(e), .. } if e[0].name == "stale.txt"));

    // ...and the late ones, arriving before or after the latest, change nothing.
    assert!(app.update(old.into_iter().next().unwrap()).is_empty());
    assert_eq!(app.tabs[0].pane_status(Side::Remote), PaneStatus::Loading);
    assert!(app.tabs[0].remote.entries.is_empty());
    assert!(app.update(new.into_iter().next().unwrap()).is_empty());
    assert_eq!(entry_names(&app, 0, Side::Remote), ["fresh.txt"]);
    assert!(app.update(up.into_iter().next().unwrap()).is_empty());
    assert_eq!(app.tabs[0].remote.path, rpath("/srv/new"));
    assert_eq!(entry_names(&app, 0, Side::Remote), ["fresh.txt"]);
    assert_eq!(app.tabs[0].remote.request, None, "nothing in flight");

    // The same id for the other pane, or for a closed tab, is not applied either.
    let again = app.update(pane_msg(tab, Side::Remote, PaneMsg::Retry));
    let id = listings(&again)[0].1;
    let stray = Message::Listed {
        tab,
        side: Side::Local,
        request_id: id,
        result: Ok(Vec::new()),
    };
    assert!(app.update(stray).is_empty());
    assert_eq!(entry_names(&app, 0, Side::Local), ["work"]);
    let closed = app.update(Message::CloseTab(tab));
    assert!(matches!(closed[..], [Effect::CloseSession(_)]));
    let late = run_listings(&fake, &again).await;
    assert!(app.update(late.into_iter().next().unwrap()).is_empty());
    assert!(app.tabs.is_empty());
}

/// A pane goes loading -> error (with Retry) -> loading -> full, and shows empty folders
/// and a failed connection as their own states.
#[tokio::test]
async fn panel_states() {
    let mut a = site("a", "h", "u", Protocol::Ssh);
    a.remote_dir = Some("/srv".into());
    let fake = Arc::new(FakeBackend::new().with_site(a.clone()));
    fake.local_fs().add_dir("/home/me/empty");

    let mut app = loaded(&fake).await;
    app.home = Some(rpath("/home/me"));
    let effects = app.update(Message::ConnectSite(a.id.clone()));
    let tab = app.tabs[0].id;
    // Connecting: both panes load (the remote one waits for its session).
    assert_eq!(app.tabs[0].pane_status(Side::Local), PaneStatus::Loading);
    assert_eq!(app.tabs[0].pane_status(Side::Remote), PaneStatus::Loading);
    assert_eq!(app.tabs[0].remote.listing, Listing::Idle);
    settle(&mut app, &fake, run_listings(&fake, &effects).await).await;
    assert_eq!(app.tabs[0].pane_status(Side::Local), PaneStatus::Full);

    // Connected, but "/srv" does not exist yet: loading, then the error.
    let backend: Arc<dyn BackendApi> = fake.clone();
    let mut listing = Vec::new();
    for message in effects::connect_sites(backend, vec![a.id.clone()])
        .collect::<Vec<_>>()
        .await
    {
        listing.extend(app.update(message));
    }
    assert_eq!(app.tabs[0].pane_status(Side::Remote), PaneStatus::Loading);
    settle(&mut app, &fake, run_listings(&fake, &listing).await).await;
    let PaneStatus::Failed(error) = app.tabs[0].pane_status(Side::Remote) else {
        panic!("{:?}", app.tabs[0].remote.listing);
    };
    assert_eq!(error.kind, ErrorKind::NotFound);
    assert_eq!(app.tabs[0].remote.path, rpath("/srv"), "the path is kept");

    // The folder appears; Retry lists it again: loading, then full.
    fake.remote_fs(&a.id)
        .add_file("/srv/a.txt", b"abc", SystemTime::UNIX_EPOCH);
    let retry = app.update(pane_msg(tab, Side::Remote, PaneMsg::Retry));
    assert_eq!(listings(&retry).len(), 1);
    assert_eq!(listings(&retry)[0].3, "/srv");
    assert_eq!(app.tabs[0].pane_status(Side::Remote), PaneStatus::Loading);
    settle(&mut app, &fake, run_listings(&fake, &retry).await).await;
    assert_eq!(app.tabs[0].pane_status(Side::Remote), PaneStatus::Full);
    assert_eq!(entry_names(&app, 0, Side::Remote), ["a.txt"]);

    // An empty folder is its own state, with ".." still there.
    let empty = app.update(pane_msg(tab, Side::Local, PaneMsg::Open(1)));
    assert_eq!(app.tabs[0].local.path, rpath("/home/me/empty"));
    settle(&mut app, &fake, run_listings(&fake, &empty).await).await;
    assert_eq!(app.tabs[0].pane_status(Side::Local), PaneStatus::Empty);
    assert_eq!(app.tabs[0].local.row_count(), 1, "just ..");

    // A failed connection: the remote side is "Not connected"; the local side works.
    let b = site("b", "h", "u", Protocol::Ftp);
    let fake = Arc::new(FakeBackend::new().with_site(b.clone()));
    fake.fail_next_connect(&b.id, Error::new(ErrorKind::Auth, "Denied"));
    fake.local_fs().add_dir("/home/me/x");
    let mut app = loaded(&fake).await;
    app.home = Some(rpath("/home/me"));
    let effects = app.update(Message::ConnectSite(b.id.clone()));
    settle(&mut app, &fake, run_listings(&fake, &effects).await).await;
    drive(&mut app, &fake, vec![b.id.clone()]).await;
    assert_eq!(
        app.tabs[0].pane_status(Side::Remote),
        PaneStatus::NotConnected
    );
    assert_eq!(app.tabs[0].pane_status(Side::Local), PaneStatus::Full);
    // Reconnect goes through the sidebar's connect path: the remote side loads, then
    // lists once connected.
    let sites = connect_effect(app.update(Message::ConnectSite(b.id.clone())));
    assert_eq!(app.tabs[0].pane_status(Side::Remote), PaneStatus::Loading);
    let backend: Arc<dyn BackendApi> = fake.clone();
    let messages: Vec<Message> = effects::connect_sites(backend, sites).collect().await;
    settle(&mut app, &fake, messages).await;
    assert_eq!(app.tabs[0].pane_status(Side::Remote), PaneStatus::Empty);
}

/// Arrows move the cursor (Shift extends), Enter enters the folder, Backspace goes up,
/// Tab switches panes; clicks read the held modifiers; a header click sorts.
#[tokio::test]
async fn keyboard_and_clicks_drive_the_focused_pane() {
    let mut a = site("a", "h", "u", Protocol::Ftp);
    a.remote_dir = Some("/srv".into());
    let fake = Arc::new(FakeBackend::new().with_site(a.clone()));
    let remote = fake.remote_fs(&a.id);
    remote.add_file("/srv/b.txt", b"bb", SystemTime::UNIX_EPOCH);
    remote.add_file("/srv/c.txt", b"c", SystemTime::UNIX_EPOCH);
    remote.add_file("/srv/docs/readme", b"r", SystemTime::UNIX_EPOCH);
    fake.local_fs().add_dir("/home/me");
    let mut app = connected_tab(&fake, &a).await;
    let tab = app.tabs[0].id;
    assert_eq!(
        entry_names(&app, 0, Side::Remote),
        ["docs", "b.txt", "c.txt"]
    );

    // The local pane has the focus first; Tab moves it.
    assert_eq!(app.tabs[0].focus, Side::Local);
    let key = |key| Message::Key { key, shift: false };
    app.update(key(FileKey::SwitchPane));
    assert_eq!(app.tabs[0].focus, Side::Remote);

    // Rows: 0 "..", 1 docs, 2 b.txt, 3 c.txt.
    app.update(key(FileKey::Down));
    app.update(key(FileKey::Down));
    app.update(Message::Key {
        key: FileKey::Down,
        shift: true,
    });
    let sel = |app: &App| {
        app.tabs[0]
            .remote
            .selection
            .rows
            .iter()
            .copied()
            .collect::<Vec<_>>()
    };
    assert_eq!(sel(&app), [1, 2]);
    app.update(key(FileKey::End));
    assert_eq!(sel(&app), [3]);
    app.update(key(FileKey::Home));
    assert_eq!(sel(&app), [0]);

    // A Ctrl+click adds; the modifiers come from the keyboard subscription.
    app.update(Message::Modifiers(Mods {
        ctrl: true,
        shift: false,
    }));
    app.update(pane_msg(tab, Side::Remote, PaneMsg::Click(3)));
    assert_eq!(sel(&app), [0, 3]);
    app.update(Message::Modifiers(Mods::default()));

    // A header click sorts (once) and keeps the selection: by size, c.txt before b.txt.
    app.update(pane_msg(tab, Side::Remote, PaneMsg::Sort(SortKey::Size)));
    assert_eq!(
        entry_names(&app, 0, Side::Remote),
        ["docs", "c.txt", "b.txt"]
    );
    assert_eq!(sel(&app), [0, 2], "still .. and c.txt");
    // Again: descending.
    app.update(pane_msg(tab, Side::Remote, PaneMsg::Sort(SortKey::Size)));
    assert_eq!(
        entry_names(&app, 0, Side::Remote),
        ["docs", "b.txt", "c.txt"]
    );

    // Enter on a folder enters it; on a file, nothing happens.
    app.update(pane_msg(tab, Side::Remote, PaneMsg::Click(1)));
    let enter = app.update(key(FileKey::Enter));
    assert_eq!(listings(&enter)[0].3, "/srv/docs");
    settle(&mut app, &fake, run_listings(&fake, &enter).await).await;
    assert_eq!(entry_names(&app, 0, Side::Remote), ["readme"]);
    app.update(key(FileKey::End));
    assert!(
        app.update(key(FileKey::Enter)).is_empty(),
        "a file is not entered"
    );
    // Backspace goes up.
    let back = app.update(key(FileKey::Back));
    assert_eq!(listings(&back)[0].3, "/srv");

    // A cursor moved below the visible rows scrolls the list to show it.
    settle(&mut app, &fake, run_listings(&fake, &back).await).await;
    app.update(pane_msg(
        tab,
        Side::Remote,
        PaneMsg::Scrolled {
            offset_y: 0.0,
            viewport_h: 60.0,
        },
    ));
    app.update(key(FileKey::Home));
    assert!(
        app.update(key(FileKey::Down)).is_empty(),
        "row 1 is in view"
    );
    let scrolled = app.update(key(FileKey::Down));
    assert!(matches!(
        scrolled[..],
        [Effect::ScrollPane { side: Side::Remote, offset_y, .. }] if offset_y == 30.0
    ));

    // A form open in the center takes no file keys.
    app.update(Message::NewSite);
    assert!(app.update(key(FileKey::Home)).is_empty());
    assert_eq!(sel(&app), [2]);
}

/// The × closes the tab and its session; the neighbours keep their places.
#[tokio::test]
async fn close_tab_closes_its_session() {
    let a = site("a", "h", "u", Protocol::Ssh);
    let b = site("b", "h", "u", Protocol::Ftp);
    let ws = workspace("w", &[&a.id, &b.id]);
    let fake = Arc::new(
        FakeBackend::new()
            .with_site(a.clone())
            .with_site(b.clone())
            .with_workspace(ws.clone()),
    );
    let mut app = loaded(&fake).await;
    let sites = connect_effect(app.update(Message::OpenWorkspace(ws.id.clone())));
    drive(&mut app, &fake, sites).await;
    app.update(Message::SelectTab(1));
    let (first, second) = (app.tabs[0].id, app.tabs[1].id);
    let TabState::Connected(connected) = tab_state(&app, &a.id).clone() else {
        panic!("{:?}", app.tabs);
    };

    let effects = app.update(Message::CloseTab(first));
    assert!(matches!(
        without_panes(effects)[..],
        [Effect::CloseSession(s)] if s == connected.session
    ));
    assert_eq!(app.tabs.len(), 1);
    assert_eq!(app.tabs[0].id, second);
    assert_eq!(
        app.active_tab,
        Some(0),
        "the active tab moved with its index"
    );
    // A stale id does nothing.
    assert!(app.update(Message::CloseTab(first)).is_empty());
}

/// Showing another tab resyncs its lists' scroll offsets.
#[tokio::test]
async fn switching_tabs_restores_scroll_offsets() {
    let a = site("a", "h", "u", Protocol::Ssh);
    let b = site("b", "h", "u", Protocol::Ftp);
    let ws = workspace("w", &[&a.id, &b.id]);
    let fake = Arc::new(
        FakeBackend::new()
            .with_site(a.clone())
            .with_site(b.clone())
            .with_workspace(ws.clone()),
    );
    let mut app = loaded(&fake).await;
    let sites = connect_effect(app.update(Message::OpenWorkspace(ws.id.clone())));
    drive(&mut app, &fake, sites).await;
    let second = app.tabs[1].id;
    app.tabs[1].local.offset_y = 300.0;
    let effects = app.update(Message::SelectTab(1));
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::ScrollPane { tab, side: Side::Local, offset_y } if *tab == second && *offset_y == 300.0
    )));
    // Selecting the tab already shown changes nothing.
    assert!(app.update(Message::SelectTab(1)).is_empty());
}

// ---- Task 7: file actions ----

use crate::app::actions::{BusyKind, Edit, EditKind, FileAction, PaneAction, Permissions, octal};
use crate::backend::DeleteTarget;
use hoprs_core::EntryKind;
use hoprs_remote::RemoteFs;

/// The file operations among `effects`.
fn file_ops(effects: &[Effect]) -> Vec<(Side, Option<SessionId>, FileAction)> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::FileOp {
                side,
                session,
                action,
                ..
            } => Some((*side, *session, action.clone())),
            _ => None,
        })
        .collect()
}

fn key(key: FileKey) -> Message {
    Message::Key { key, shift: false }
}

fn ctrl(app: &mut App, on: bool) {
    app.update(Message::Modifiers(Mods {
        ctrl: on,
        shift: false,
    }));
}

/// The row of the entry called `name` in a pane.
fn row_of(app: &App, side: Side, name: &str) -> usize {
    let pane = app.tabs[0].pane(side);
    usize::from(pane.has_up())
        + pane
            .entries
            .iter()
            .position(|e| e.name == name)
            .unwrap_or_else(|| panic!("no {name} in {:?}", pane.entries))
}

fn selected_names(app: &App, side: Side) -> Vec<String> {
    app.tabs[0]
        .pane(side)
        .selected_entries()
        .into_iter()
        .map(|e| e.name.clone())
        .collect()
}

/// A connected tab with the remote pane focused (`/srv`).
async fn remote_focused(fake: &Arc<FakeBackend>, a: &Site) -> (App, TabId) {
    let mut app = connected_tab(fake, a).await;
    let tab = app.tabs[0].id;
    app.update(key(FileKey::SwitchPane));
    assert_eq!(app.tabs[0].focus, Side::Remote);
    (app, tab)
}

fn srv_site() -> Site {
    let mut a = site("a", "h", "u", Protocol::Ftp);
    a.remote_dir = Some("/srv".into());
    a
}

/// F7 opens the inline name row; Enter creates the folder on the pane's side; the folder
/// is listed again with the new folder selected. A taken name is refused inline.
#[tokio::test]
async fn mkdir_then_refresh() {
    let a = srv_site();
    let fake = Arc::new(FakeBackend::new().with_site(a.clone()));
    fake.remote_fs(&a.id)
        .add_file("/srv/a.txt", b"a", SystemTime::UNIX_EPOCH);
    fake.local_fs().add_dir("/home/me");
    let (mut app, tab) = remote_focused(&fake, &a).await;

    let effects = app.update(key(FileKey::NewFolder));
    assert!(
        matches!(
            effects[..],
            [Effect::FocusEdit {
                side: Side::Remote,
                select_end: 10,
                ..
            }]
        ),
        "the editor takes the focus with \"New folder\" selected: {effects:?}"
    );
    assert_eq!(
        app.tabs[0].remote.action,
        Some(PaneAction::Edit(Edit {
            kind: EditKind::NewFolder,
            value: "New folder".into(),
            error: None,
        }))
    );

    // A taken name stays in the editor with the reason.
    app.update(pane_msg(
        tab,
        Side::Remote,
        PaneMsg::EditChanged("a.txt".into()),
    ));
    assert!(app.update(key(FileKey::Enter)).is_empty());
    assert!(matches!(
        &app.tabs[0].remote.action,
        Some(PaneAction::Edit(Edit { error: Some(e), .. })) if *e == strings::NAME_TAKEN
    ));
    // Typing clears it; Enter creates the folder on the session.
    app.update(pane_msg(
        tab,
        Side::Remote,
        PaneMsg::EditChanged("build".into()),
    ));
    assert!(matches!(
        &app.tabs[0].remote.action,
        Some(PaneAction::Edit(Edit { error: None, .. }))
    ));
    let create = app.update(key(FileKey::Enter));
    let ops = file_ops(&create);
    assert_eq!(ops.len(), 1);
    assert!(ops[0].1.is_some(), "the remote side works on the session");
    assert_eq!(ops[0].2, FileAction::Mkdir(rpath("/srv/build")));
    assert_eq!(app.tabs[0].remote.action, None);
    assert_eq!(
        app.tabs[0].remote.busy.as_ref().map(|b| b.kind),
        Some(BusyKind::NewFolder)
    );
    // Nothing else starts while it runs.
    assert!(app.update(key(FileKey::NewFolder)).is_empty());
    assert_eq!(app.tabs[0].remote.action, None);

    // Done: the folder is listed again, with the new folder selected.
    let done = run_file_ops(&fake, &create).await;
    assert!(matches!(
        &done[..],
        [Message::FileOpDone { result: Ok(()), .. }]
    ));
    let refresh = app.update(done.into_iter().next().unwrap());
    assert_eq!(listings(&refresh).len(), 1);
    assert_eq!(listings(&refresh)[0].3, "/srv");
    assert_eq!(app.tabs[0].remote.busy, None);
    settle(&mut app, &fake, run_listings(&fake, &refresh).await).await;
    assert_eq!(entry_names(&app, 0, Side::Remote), ["build", "a.txt"]);
    assert_eq!(selected_names(&app, Side::Remote), ["build"]);
    let made = fake
        .remote_fs(&a.id)
        .stat(&rpath("/srv/build"))
        .await
        .unwrap();
    assert_eq!(made.kind, EntryKind::Dir);

    // The local side works too, without a session (toolbar button this time).
    let start = app.update(pane_msg(tab, Side::Local, PaneMsg::NewFolder));
    assert!(matches!(
        start[..],
        [Effect::FocusEdit {
            side: Side::Local,
            ..
        }]
    ));
    assert_eq!(app.tabs[0].focus, Side::Local);
    let create = app.update(pane_msg(tab, Side::Local, PaneMsg::EditCommit));
    assert_eq!(
        file_ops(&create),
        [(
            Side::Local,
            None,
            FileAction::Mkdir(rpath("/home/me/New folder"))
        )]
    );
    settle(&mut app, &fake, run_file_ops(&fake, &create).await).await;
    assert_eq!(entry_names(&app, 0, Side::Local), ["New folder"]);
}

/// F2 edits the name in its row; Escape leaves it as it was, Enter renames.
#[tokio::test]
async fn rename_inline_commits_on_enter_cancels_on_escape() {
    let a = srv_site();
    let fake = Arc::new(FakeBackend::new().with_site(a.clone()));
    let remote = fake.remote_fs(&a.id);
    remote.add_file("/srv/report.pdf", b"r", SystemTime::UNIX_EPOCH);
    remote.add_file("/srv/b.txt", b"b", SystemTime::UNIX_EPOCH);
    fake.local_fs().add_dir("/home/me");
    let (mut app, tab) = remote_focused(&fake, &a).await;

    // Nothing selected: nothing to rename.
    assert!(app.update(key(FileKey::Rename)).is_empty());
    assert_eq!(app.tabs[0].remote.action, None);

    let row = row_of(&app, Side::Remote, "report.pdf");
    app.update(pane_msg(tab, Side::Remote, PaneMsg::Click(row)));
    let start = app.update(key(FileKey::Rename));
    assert!(
        matches!(
            start[..],
            [Effect::FocusEdit {
                side: Side::Remote,
                select_end: 6,
                ..
            }]
        ),
        "the stem is preselected: {start:?}"
    );
    let editing = Some(PaneAction::Edit(Edit {
        kind: EditKind::Rename {
            from: "report.pdf".into(),
        },
        value: "report.pdf".into(),
        error: None,
    }));
    assert_eq!(app.tabs[0].remote.action, editing);

    // Escape: back as it was, nothing sent.
    app.update(pane_msg(
        tab,
        Side::Remote,
        PaneMsg::EditChanged("final.pdf".into()),
    ));
    assert!(app.update(key(FileKey::Escape)).is_empty());
    assert_eq!(app.tabs[0].remote.action, None);
    assert_eq!(entry_names(&app, 0, Side::Remote), ["b.txt", "report.pdf"]);
    assert!(remote.read_file("/srv/report.pdf").is_some());

    // Again: the editor starts from the real name; Enter renames.
    app.update(key(FileKey::Rename));
    assert_eq!(app.tabs[0].remote.action, editing);
    app.update(pane_msg(
        tab,
        Side::Remote,
        PaneMsg::EditChanged("final.pdf".into()),
    ));
    let commit = app.update(key(FileKey::Enter));
    assert_eq!(
        file_ops(&commit)[0].2,
        FileAction::Rename {
            from: rpath("/srv/report.pdf"),
            to: rpath("/srv/final.pdf"),
        }
    );
    settle(&mut app, &fake, run_file_ops(&fake, &commit).await).await;
    assert_eq!(entry_names(&app, 0, Side::Remote), ["b.txt", "final.pdf"]);
    assert_eq!(selected_names(&app, Side::Remote), ["final.pdf"]);
    assert!(remote.read_file("/srv/report.pdf").is_none());
    assert_eq!(
        remote.read_file("/srv/final.pdf").as_deref(),
        Some(&b"r"[..])
    );

    // The editor's Enter (on_submit) commits too; the same name just closes it.
    app.update(key(FileKey::Rename));
    assert!(
        app.update(pane_msg(tab, Side::Remote, PaneMsg::EditCommit))
            .is_empty()
    );
    assert_eq!(app.tabs[0].remote.action, None);
    // A name another entry has is refused inline.
    app.update(key(FileKey::Rename));
    app.update(pane_msg(
        tab,
        Side::Remote,
        PaneMsg::EditChanged("b.txt".into()),
    ));
    assert!(app.update(key(FileKey::Enter)).is_empty());
    assert!(matches!(
        &app.tabs[0].remote.action,
        Some(PaneAction::Edit(Edit { error: Some(e), .. })) if *e == strings::NAME_TAKEN
    ));
}

/// Del asks first, listing the names; only "Delete" deletes. Folders go with everything
/// in them (links are removed, not followed), with a running count.
#[tokio::test]
async fn delete_requires_confirmation() {
    let a = srv_site();
    let fake = Arc::new(FakeBackend::new().with_site(a.clone()));
    let remote = fake.remote_fs(&a.id);
    let t = SystemTime::UNIX_EPOCH;
    remote.add_file("/srv/tree/sub/x.txt", b"x", t);
    remote.add_file("/srv/tree/y.txt", b"y", t);
    remote.add_dir("/srv/empty");
    remote.add_file("/srv/f.txt", b"f", t);
    remote.add_file("/srv/keep.txt", b"k", t);
    remote.add_file("/elsewhere/k.txt", b"k", t);
    remote.add_symlink("/srv/link", "/elsewhere");
    fake.local_fs().add_dir("/home/me");
    let (mut app, tab) = remote_focused(&fake, &a).await;

    // Only ".." selected: nothing to delete.
    app.update(pane_msg(tab, Side::Remote, PaneMsg::Click(0)));
    assert!(app.update(key(FileKey::Delete)).is_empty());
    assert_eq!(app.tabs[0].remote.action, None);

    for (i, name) in ["tree", "empty", "f.txt", "link"].into_iter().enumerate() {
        ctrl(&mut app, i > 0);
        let row = row_of(&app, Side::Remote, name);
        app.update(pane_msg(tab, Side::Remote, PaneMsg::Click(row)));
    }
    ctrl(&mut app, false);
    // The plain click on "tree" replaced the "..": rows are in list order.
    assert_eq!(
        selected_names(&app, Side::Remote),
        ["empty", "tree", "f.txt", "link"]
    );

    // Del only asks, with the names.
    let ask = app.update(key(FileKey::Delete));
    assert!(file_ops(&ask).is_empty(), "nothing is deleted yet");
    let Some(PaneAction::ConfirmDelete(asked)) = &app.tabs[0].remote.action else {
        panic!("{:?}", app.tabs[0].remote.action);
    };
    let asked: Vec<&str> = asked.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(asked, ["empty", "tree", "f.txt", "link"]);
    assert_eq!(strings::delete_question(&asked), "Delete 4 items?");
    // Escape and Keep both leave everything in place.
    assert!(app.update(key(FileKey::Escape)).is_empty());
    assert_eq!(app.tabs[0].remote.action, None);
    app.update(pane_msg(tab, Side::Remote, PaneMsg::Delete));
    assert!(
        app.update(pane_msg(tab, Side::Remote, PaneMsg::CancelAction))
            .is_empty()
    );
    assert_eq!(app.tabs[0].remote.action, None);
    assert!(remote.read_file("/srv/f.txt").is_some());

    // Confirmed: one operation, not the transfer queue.
    app.update(key(FileKey::Delete));
    let go = app.update(pane_msg(tab, Side::Remote, PaneMsg::ConfirmDelete));
    let ops = file_ops(&go);
    assert_eq!(ops.len(), 1);
    let target = |path: &str, kind| DeleteTarget {
        path: rpath(path),
        kind,
    };
    assert_eq!(
        ops[0].2,
        FileAction::Delete(vec![
            target("/srv/empty", EntryKind::Dir),
            target("/srv/tree", EntryKind::Dir),
            target("/srv/f.txt", EntryKind::File),
            target("/srv/link", EntryKind::Symlink),
        ])
    );
    assert!(fake.enqueued().is_empty());
    assert_eq!(
        app.tabs[0].remote.busy.as_ref().map(|b| (b.kind, b.done)),
        Some((BusyKind::Delete, 0))
    );

    // The count runs as items go (empty/ 1; x.txt, sub/, y.txt, tree/ 4; f.txt; link),
    // then the result comes last.
    let messages = run_file_ops(&fake, &go).await;
    let (last, progress) = messages.split_last().unwrap();
    assert!(matches!(last, Message::FileOpDone { result: Ok(()), .. }));
    assert_eq!(progress.len(), 7);
    for message in progress.iter().cloned() {
        assert!(app.update(message).is_empty());
    }
    assert_eq!(app.tabs[0].remote.busy.as_ref().map(|b| b.done), Some(7));
    assert_eq!(strings::deleting(7), "Deleting… 7 items");
    let refresh = app.update(last.clone());
    assert_eq!(listings(&refresh).len(), 1);
    settle(&mut app, &fake, run_listings(&fake, &refresh).await).await;
    assert_eq!(entry_names(&app, 0, Side::Remote), ["keep.txt"]);
    assert_eq!(
        remote.read_file("/elsewhere/k.txt").as_deref(),
        Some(&b"k"[..])
    );
    assert_eq!(app.tabs[0].remote.busy, None);
    assert_eq!(app.tabs[0].remote.action_error, None);
}

/// The rwx grid and the octal value follow each other; Apply runs chmod with the mode.
#[tokio::test]
async fn chmod_dialog_octal_roundtrip() {
    // The dialog alone.
    let mut d = Permissions::new(vec!["run.sh".into()], 0o100_755);
    assert_eq!(
        (d.mode, d.octal.as_str()),
        (0o755, "755"),
        "file type bits dropped"
    );
    d.toggle(0o020);
    assert_eq!((d.mode, d.octal.as_str()), (0o775, "775"));
    d.set_octal("640".into());
    assert_eq!(d.mode, 0o640);
    assert!(d.has(0o400) && d.has(0o200) && d.has(0o040));
    assert!(!d.has(0o100) && !d.has(0o020) && !d.has(0o004));
    d.set_octal("6x".into());
    assert!(d.invalid);
    assert_eq!((d.mode, d.octal.as_str()), (0o640, "6x"), "kept as typed");
    d.toggle(0o004);
    assert!(!d.invalid);
    assert_eq!((d.mode, d.octal.as_str()), (0o644, "644"));
    d.set_octal("4755".into());
    assert_eq!(d.mode, 0o4755);
    d.toggle(0o002);
    assert_eq!((d.mode, d.octal.as_str()), (0o4757, "4757"));
    assert_eq!(octal(0o4757), "4757");

    // In the app: remote only, from the entry's mode, applied with chmod.
    let a = srv_site();
    let fake = Arc::new(FakeBackend::new().with_site(a.clone()));
    let remote = fake.remote_fs(&a.id);
    remote.add_file("/srv/run.sh", b"#!", SystemTime::UNIX_EPOCH);
    fake.local_fs()
        .add_file("/home/me/l.txt", b"l", SystemTime::UNIX_EPOCH);
    let (mut app, tab) = remote_focused(&fake, &a).await;
    app.update(pane_msg(tab, Side::Local, PaneMsg::Click(1)));
    assert!(
        app.update(pane_msg(tab, Side::Local, PaneMsg::Permissions))
            .is_empty()
    );
    assert_eq!(
        app.tabs[0].local.action, None,
        "no permissions on the local side"
    );

    let row = row_of(&app, Side::Remote, "run.sh");
    app.update(pane_msg(tab, Side::Remote, PaneMsg::Click(row)));
    app.update(pane_msg(tab, Side::Remote, PaneMsg::Permissions));
    let dialog = |app: &App| match &app.tabs[0].remote.action {
        Some(PaneAction::Permissions(d)) => d.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(dialog(&app).mode, 0o644, "from the listed mode");
    assert_eq!(dialog(&app).names, ["run.sh"]);
    app.update(pane_msg(
        tab,
        Side::Remote,
        PaneMsg::PermissionToggled(0o100),
    ));
    assert_eq!(dialog(&app).octal, "744");
    // An invalid value can't be applied.
    app.update(pane_msg(
        tab,
        Side::Remote,
        PaneMsg::PermissionOctal("99".into()),
    ));
    assert!(
        app.update(pane_msg(tab, Side::Remote, PaneMsg::ApplyPermissions))
            .is_empty()
    );
    app.update(pane_msg(
        tab,
        Side::Remote,
        PaneMsg::PermissionOctal("750".into()),
    ));
    assert!(dialog(&app).has(0o010) && !dialog(&app).has(0o020));
    let apply = app.update(pane_msg(tab, Side::Remote, PaneMsg::ApplyPermissions));
    assert_eq!(
        file_ops(&apply)[0].2,
        FileAction::Chmod {
            paths: vec![rpath("/srv/run.sh")],
            mode: 0o750,
        }
    );
    assert_eq!(app.tabs[0].remote.action, None);
    settle(&mut app, &fake, run_file_ops(&fake, &apply).await).await;
    assert_eq!(app.tabs[0].remote.entries[0].mode, Some(0o750));
    assert_eq!(selected_names(&app, Side::Remote), Vec::<String>::new());
}

/// A failed action shows in the pane, above the table, with Dismiss: no form, no modal,
/// the pane keeps working.
#[tokio::test]
async fn action_error_is_inline_not_modal() {
    let a = srv_site();
    let fake = Arc::new(FakeBackend::new().with_site(a.clone()));
    let remote = fake.remote_fs(&a.id);
    remote.add_file("/srv/a.txt", b"a", SystemTime::UNIX_EPOCH);
    fake.local_fs().add_dir("/home/me/x");
    remote.deny_writes_under("/srv");
    let (mut app, tab) = remote_focused(&fake, &a).await;

    app.update(key(FileKey::NewFolder));
    let create = app.update(key(FileKey::Enter));
    let done = run_file_ops(&fake, &create).await;
    let refresh = app.update(done.into_iter().next().unwrap());
    let error = app.tabs[0]
        .remote
        .action_error
        .clone()
        .expect("inline error");
    assert_eq!(error.kind, ErrorKind::Permission);
    assert!(app.form.is_none(), "no form, no modal");
    assert!(matches!(tab_state(&app, &a.id), TabState::Connected(_)));
    assert_eq!(app.tabs[0].remote.busy, None);
    // The folder is listed again: the table stays, under the error.
    settle(&mut app, &fake, run_listings(&fake, &refresh).await).await;
    assert_eq!(app.tabs[0].pane_status(Side::Remote), PaneStatus::Full);
    assert_eq!(entry_names(&app, 0, Side::Remote), ["a.txt"]);
    assert!(
        app.tabs[0].remote.action_error.is_some(),
        "kept until dismissed"
    );
    assert_eq!(
        app.tabs[0].local.action_error, None,
        "the other pane is untouched"
    );

    // The pane keeps working; Dismiss (or Escape) clears the bar.
    app.update(pane_msg(tab, Side::Remote, PaneMsg::Click(1)));
    app.update(key(FileKey::Rename));
    app.update(pane_msg(
        tab,
        Side::Remote,
        PaneMsg::EditChanged("b.txt".into()),
    ));
    let rename = app.update(key(FileKey::Enter));
    assert_eq!(
        app.tabs[0].remote.action_error, None,
        "a new action clears the old error"
    );
    settle(&mut app, &fake, run_file_ops(&fake, &rename).await).await;
    assert_eq!(
        app.tabs[0].remote.action_error.as_ref().map(|e| e.kind),
        Some(ErrorKind::Permission)
    );
    app.update(pane_msg(tab, Side::Remote, PaneMsg::DismissError));
    assert_eq!(app.tabs[0].remote.action_error, None);
    // Escape dismisses as well.
    app.tabs[0].remote.action_error = Some(error);
    app.update(key(FileKey::Escape));
    assert_eq!(app.tabs[0].remote.action_error, None);
}

/// Results that come back late: for a closed tab, for an operation the pane no longer
/// waits for, or after the pane moved to another folder (not listed again there).
#[tokio::test]
async fn late_action_results_are_safe() {
    let a = srv_site();
    let fake = Arc::new(FakeBackend::new().with_site(a.clone()));
    let remote = fake.remote_fs(&a.id);
    remote.add_file("/srv/docs/r.txt", b"r", SystemTime::UNIX_EPOCH);
    fake.local_fs().add_dir("/home/me");
    let (mut app, tab) = remote_focused(&fake, &a).await;

    app.update(key(FileKey::NewFolder));
    let create = app.update(key(FileKey::Enter));
    let Some(Effect::FileOp { op, .. }) =
        create.iter().find(|e| matches!(e, Effect::FileOp { .. }))
    else {
        panic!("{create:?}");
    };
    let op = *op;
    // Some other id: ignored, the pane still waits.
    let stray = Message::FileOpDone {
        tab,
        side: Side::Remote,
        op: RequestId(op.0 + 1000),
        result: Ok(()),
    };
    assert!(app.update(stray).is_empty());
    assert!(app.tabs[0].remote.busy.is_some());
    // Same id for the other pane: ignored too.
    let other = Message::FileOpDone {
        tab,
        side: Side::Local,
        op,
        result: Ok(()),
    };
    assert!(app.update(other).is_empty());

    // The user goes into "docs" while it runs; the result lands there safely.
    let docs = row_of(&app, Side::Remote, "docs");
    let into = app.update(pane_msg(tab, Side::Remote, PaneMsg::Open(docs)));
    settle(&mut app, &fake, run_listings(&fake, &into).await).await;
    let done = run_file_ops(&fake, &create).await;
    let effects = app.update(done.into_iter().next().unwrap());
    assert!(
        listings(&effects).is_empty(),
        "docs is not listed again for /srv"
    );
    assert_eq!(app.tabs[0].remote.busy, None, "the pane can act again");
    assert_eq!(app.tabs[0].remote.select_after, None);
    assert_eq!(entry_names(&app, 0, Side::Remote), ["r.txt"]);

    // A closed tab drops its result.
    app.update(key(FileKey::NewFolder));
    let create = app.update(key(FileKey::Enter));
    app.update(Message::CloseTab(tab));
    let done = run_file_ops(&fake, &create).await;
    assert!(app.update(done.into_iter().next().unwrap()).is_empty());
    let progress = Message::FileOpProgress {
        tab,
        side: Side::Remote,
        op,
        done: 3,
    };
    assert!(app.update(progress).is_empty());
}

/// Task 6 carry-over: listing a folder again starts its list from the top, so the
/// remembered offset matches the new scrollable.
#[tokio::test]
async fn relisting_resets_the_scroll_offset() {
    let a = srv_site();
    let fake = Arc::new(FakeBackend::new().with_site(a.clone()));
    fake.remote_fs(&a.id)
        .add_file("/srv/a.txt", b"a", SystemTime::UNIX_EPOCH);
    fake.local_fs().add_dir("/home/me");
    let (mut app, tab) = remote_focused(&fake, &a).await;
    let scrolled = |app: &mut App| {
        app.update(pane_msg(
            tab,
            Side::Remote,
            PaneMsg::Scrolled {
                offset_y: 300.0,
                viewport_h: 600.0,
            },
        ));
        assert_eq!(app.tabs[0].remote.offset_y, 300.0);
    };
    let to_top = |effects: &[Effect]| {
        effects.iter().any(|e| {
            matches!(e, Effect::ScrollPane { side: Side::Remote, offset_y, .. } if *offset_y == 0.0)
        })
    };

    scrolled(&mut app);
    let retry = app.update(pane_msg(tab, Side::Remote, PaneMsg::Retry));
    assert_eq!(app.tabs[0].remote.offset_y, 0.0);
    assert!(to_top(&retry));
    settle(&mut app, &fake, run_listings(&fake, &retry).await).await;

    // After an action too.
    scrolled(&mut app);
    app.update(key(FileKey::NewFolder));
    let create = app.update(key(FileKey::Enter));
    let done = run_file_ops(&fake, &create).await;
    let refresh = app.update(done.into_iter().next().unwrap());
    assert_eq!(app.tabs[0].remote.offset_y, 0.0);
    assert!(to_top(&refresh));
}

/// Task 6 carry-over: while a name is being edited (or the permissions dialog is open),
/// the pane shortcuts don't fire: the keys belong to the text field. Outside it, every
/// key first checks that no text field (the sidebar filter) has the focus.
#[tokio::test]
async fn shortcuts_are_suppressed_while_renaming() {
    let a = srv_site();
    let fake = Arc::new(FakeBackend::new().with_site(a.clone()));
    let remote = fake.remote_fs(&a.id);
    remote.add_file("/srv/a.txt", b"a", SystemTime::UNIX_EPOCH);
    remote.add_file("/srv/docs/x", b"x", SystemTime::UNIX_EPOCH);
    fake.local_fs().add_dir("/home/me");
    let (mut app, tab) = remote_focused(&fake, &a).await;

    // Outside an editor: the key waits for the focus check.
    assert!(matches!(
        app.update(Message::KeyPressed {
            key: FileKey::Down,
            shift: false
        })[..],
        [Effect::KeyUnlessTyping {
            key: FileKey::Down,
            shift: false
        }]
    ));

    let row = row_of(&app, Side::Remote, "a.txt");
    app.update(pane_msg(tab, Side::Remote, PaneMsg::Click(row)));
    app.update(key(FileKey::Rename));
    let before = app.tabs[0].clone();
    for k in [
        FileKey::Up,
        FileKey::Down,
        FileKey::Home,
        FileKey::End,
        FileKey::Back,
        FileKey::SwitchPane,
        FileKey::NewFolder,
        FileKey::Rename,
        FileKey::Delete,
    ] {
        assert!(
            app.update(Message::KeyPressed {
                key: k,
                shift: false
            })
            .is_empty(),
            "{k:?} pressed"
        );
        assert!(app.update(key(k)).is_empty(), "{k:?}");
        assert_eq!(app.tabs[0], before, "{k:?} changed the pane");
    }
    // Escape goes straight through and cancels.
    assert!(
        app.update(Message::KeyPressed {
            key: FileKey::Escape,
            shift: false
        })
        .is_empty()
    );
    assert_eq!(app.tabs[0].remote.action, None);

    // Same with the permissions dialog open.
    app.update(pane_msg(tab, Side::Remote, PaneMsg::Permissions));
    let before = app.tabs[0].clone();
    assert!(
        app.update(Message::KeyPressed {
            key: FileKey::Delete,
            shift: false
        })
        .is_empty()
    );
    assert!(app.update(key(FileKey::Delete)).is_empty());
    assert!(app.update(key(FileKey::Enter)).is_empty());
    assert_eq!(app.tabs[0], before);

    // A form open in the center takes every key.
    app.update(key(FileKey::Escape));
    app.update(Message::NewSite);
    assert!(
        app.update(Message::KeyPressed {
            key: FileKey::Down,
            shift: false
        })
        .is_empty()
    );
}
