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

/// The single `ConnectSites` effect of `effects`.
fn connect_effect(effects: Vec<Effect>) -> Vec<SiteId> {
    match <[Effect; 1]>::try_from(effects) {
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
    messages
        .into_iter()
        .flat_map(|message| app.update(message))
        .collect()
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
    assert!(site_form(&mut app).busy);
    // A second click while saving does nothing.
    assert!(type_site(&mut app, SiteFormMsg::Save).is_empty());

    // A refused save keeps the form, with the backend's error.
    let refused = Error::new(ErrorKind::Io, "Disk full");
    assert!(
        app.update(Message::FormDone(Err(refused.clone())))
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
    let effects = app.update(Message::FormDone(Ok(())));
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
    let effects = app.update(Message::FormDone(Ok(())));
    assert!(app.form.is_none());
    assert!(matches!(effects[..], [Effect::LoadProfiles]));
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
        assert!(app.update(message).is_empty());
    }
    early.sort_by(|x, y| x.0.cmp(&y.0));
    let mut expected = vec![b.id.clone(), c.id.clone()];
    expected.sort_by(|x, y| x.0.cmp(&y.0));
    assert_eq!(early, expected);
    assert!(matches!(tab_state(&app, &b.id), TabState::Connected(_)));
    assert_eq!(tab_state(&app, &a.id), &TabState::Connecting);

    release_a.send(()).unwrap();
    let last = stream.next().await.unwrap();
    assert!(app.update(last).is_empty());
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
    app.update(Message::FormDone(Ok(())));
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
