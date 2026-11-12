//! Task 11: "Import from FileZilla" (three steps in the center, no modal), the encrypted
//! backup (export and restore) and the settings page.

use hoprs_import::{ImportReport, ImportedSite, SkippedSite};
use hoprs_transfer::ConflictPolicy;
use hoprs_vault::{BackupImportReport, ExportReport, ImportMode};

use std::path::PathBuf;

use secrecy::SecretString;

use super::*;
use crate::app::import::{ImportMsg, ImportStep, SourceDelete};
use crate::app::settings::{BackupMsg, BackupPanel, BackupState, SettingsMsg};
use crate::backend::fake::BackupCall;
use crate::settings::{Settings, SettingsLoad};
use crate::ui::import as import_view;

fn import(app: &mut App, msg: ImportMsg) -> Vec<Effect> {
    app.update(Message::Import(msg))
}

fn settings(app: &mut App, msg: SettingsMsg) -> Vec<Effect> {
    app.update(Message::Settings(msg))
}

fn backup(app: &mut App, msg: BackupMsg) -> Vec<Effect> {
    settings(app, SettingsMsg::Backup(msg))
}

fn report() -> ImportReport {
    ImportReport {
        imported: vec![
            ImportedSite {
                name: "Prod".into(),
                id: SiteId::new(),
                notes: Vec::new(),
            },
            ImportedSite {
                name: "Legacy FTP".into(),
                id: SiteId::new(),
                notes: vec!["password not saved: keyring unavailable".into()],
            },
        ],
        skipped: vec![SkippedSite {
            name: "Old S3".into(),
            reason: "unsupported protocol".into(),
        }],
        workspaces_created: vec!["Clients".into()],
        skipped_workspaces: vec!["Work: a workspace with this name already exists".into()],
        failures: Vec::new(),
    }
}

/// An app with the import flow at its report, from `path`.
fn at_report(path: &str, report: ImportReport) -> App {
    let mut app = App::new();
    app.update(Message::OpenImport);
    import(&mut app, ImportMsg::Path(path.into()));
    import(&mut app, ImportMsg::Next);
    import(
        &mut app,
        ImportMsg::Start {
            folders_as_workspaces: false,
        },
    );
    app.update(Message::Imported(Ok(report)));
    assert_eq!(app.import.step, ImportStep::Report);
    app
}

#[test]
fn import_flow_steps() {
    let mut app = App::new();
    app.filezilla_default = Some(PathBuf::from("/home/me/.config/filezilla/sitemanager.xml"));
    assert!(app.update(Message::OpenImport).is_empty());
    assert!(
        matches!(app.form, Some(Form::Import)),
        "in the center, no modal"
    );
    assert!(!app.dialogs.is_open());

    // Step 1: the file, FileZilla's default path to start with.
    assert_eq!(app.import.step, ImportStep::File);
    assert_eq!(
        app.import.path,
        "/home/me/.config/filezilla/sitemanager.xml"
    );
    // An empty path is refused inline.
    import(&mut app, ImportMsg::Path("  ".into()));
    assert!(import(&mut app, ImportMsg::Next).is_empty());
    assert_eq!(app.import.step, ImportStep::File);
    assert_eq!(app.import.path_error, Some(strings::IMPORT_PATH_REQUIRED));
    // "Choose…" opens the system file dialog; its answer fills the path.
    let effects = import(&mut app, ImportMsg::Choose);
    assert!(matches!(
        effects[..],
        [Effect::PickFile(PickTarget::Sitemanager)]
    ));
    app.update(Message::FilePicked {
        target: PickTarget::Sitemanager,
        path: Some(PathBuf::from("/tmp/x/sitemanager.xml")),
    });
    assert_eq!(app.import.path, "/tmp/x/sitemanager.xml");
    assert_eq!(app.import.path_error, None);
    // A cancelled dialog changes nothing.
    app.update(Message::FilePicked {
        target: PickTarget::Sitemanager,
        path: None,
    });
    assert_eq!(app.import.path, "/tmp/x/sitemanager.xml");

    // Step 2: the folders question; Back returns to the file.
    assert!(import(&mut app, ImportMsg::Next).is_empty());
    assert_eq!(app.import.step, ImportStep::Folders);
    import(&mut app, ImportMsg::Back);
    assert_eq!(app.import.step, ImportStep::File);
    import(&mut app, ImportMsg::Next);

    // The answer starts the import, off the UI thread.
    let effects = import(
        &mut app,
        ImportMsg::Start {
            folders_as_workspaces: true,
        },
    );
    match &effects[..] {
        [
            Effect::ImportFilezilla {
                path,
                folders_as_workspaces: true,
            },
        ] => assert_eq!(path, &PathBuf::from("/tmp/x/sitemanager.xml")),
        other => panic!("expected one ImportFilezilla, got {other:?}"),
    }
    assert_eq!(app.import.step, ImportStep::Running);
    assert!(app.busy(), "the progress indicator runs");
    // A second start while it runs does nothing.
    assert!(
        import(
            &mut app,
            ImportMsg::Start {
                folders_as_workspaces: false
            }
        )
        .is_empty()
    );

    // Step 3: the report, and the sidebar reloads.
    let done = report();
    let effects = app.update(Message::Imported(Ok(done.clone())));
    assert!(matches!(effects[..], [Effect::LoadProfiles]));
    assert_eq!(app.import.step, ImportStep::Report);
    assert_eq!(app.import.report, Some(done));
    assert!(!app.busy());

    // A file that can't be read or parsed: back to step 1 with the error inline.
    let mut app = App::new();
    app.update(Message::OpenImport);
    import(&mut app, ImportMsg::Path("/nope.xml".into()));
    import(&mut app, ImportMsg::Next);
    import(
        &mut app,
        ImportMsg::Start {
            folders_as_workspaces: false,
        },
    );
    let error = Error::new(ErrorKind::NotFound, "No such file");
    assert!(app.update(Message::Imported(Err(error.clone()))).is_empty());
    assert_eq!(app.import.step, ImportStep::File);
    assert_eq!(app.import.error, Some(error));
    assert_eq!(app.import.path, "/nope.xml");

    // Done closes the flow; opening it again starts over.
    let mut app = at_report("/a.xml", report());
    import(&mut app, ImportMsg::Close);
    assert!(app.form.is_none());
    app.update(Message::OpenImport);
    assert_eq!(app.import.step, ImportStep::File);
    assert_eq!(app.import.report, None);
}

#[test]
fn import_report_sections() {
    let full = report();
    let sections = import_view::report_sections(&full);
    let heads: Vec<(&str, usize)> = sections.iter().map(|s| (s.title, s.count)).collect();
    assert_eq!(
        heads,
        [
            (strings::IMPORT_IMPORTED, 2),
            (strings::IMPORT_SKIPPED, 1),
            (strings::IMPORT_WORKSPACES_CREATED, 1),
            (strings::IMPORT_WORKSPACES_NOT_CREATED, 1),
            (strings::IMPORT_FAILURES, 0),
        ]
    );
    // Imported sites with their notes; skipped ones with the reason.
    assert_eq!(sections[0].lines[0].name, "Prod");
    assert!(sections[0].lines[0].notes.is_empty());
    assert_eq!(sections[0].lines[1].name, "Legacy FTP");
    assert_eq!(
        sections[0].lines[1].notes,
        ["password not saved: keyring unavailable"]
    );
    assert_eq!(sections[1].lines[0].name, "Old S3");
    assert_eq!(sections[1].lines[0].notes, ["unsupported protocol"]);
    assert_eq!(
        sections[3].lines[0].name,
        "Work: a workspace with this name already exists"
    );

    // Failures are listed with their count.
    let mut failed = report();
    failed.failures = vec!["A: disk full".into(), "B: disk full".into()];
    let sections = import_view::report_sections(&failed);
    assert_eq!(sections[4].count, 2);
    assert_eq!(sections[4].lines[1].name, "B: disk full");

    // The view renders every step without panicking.
    let mut app = at_report("/a.xml", failed);
    let _ = import_view::view(&app);
    app.import.step = ImportStep::Running;
    let _ = import_view::view(&app);
}

#[test]
fn delete_source_needs_confirmation() {
    let mut app = at_report("/home/me/sitemanager.xml", report());
    assert_eq!(app.import.delete, SourceDelete::Offered);

    // The first click only asks; Keep backs out.
    assert!(import(&mut app, ImportMsg::Delete).is_empty());
    assert_eq!(app.import.delete, SourceDelete::Confirming);
    assert!(import(&mut app, ImportMsg::CancelDelete).is_empty());
    assert_eq!(app.import.delete, SourceDelete::Offered);
    // Confirming without being asked does nothing.
    assert!(import(&mut app, ImportMsg::ConfirmDelete).is_empty());

    import(&mut app, ImportMsg::Delete);
    // The file the report came from, even if the path field were edited since.
    let effects = import(&mut app, ImportMsg::ConfirmDelete);
    match &effects[..] {
        [Effect::DeleteSitemanager(path)] => {
            assert_eq!(path, &PathBuf::from("/home/me/sitemanager.xml"));
        }
        other => panic!("expected one DeleteSitemanager, got {other:?}"),
    }
    assert_eq!(app.import.delete, SourceDelete::Deleting);
    assert!(import(&mut app, ImportMsg::ConfirmDelete).is_empty());

    app.update(Message::SitemanagerDeleted(Ok(())));
    assert_eq!(app.import.delete, SourceDelete::Deleted);

    // A refusal (not a FileZilla file, a symlink…) shows inline.
    let mut app = at_report("/x.xml", report());
    import(&mut app, ImportMsg::Delete);
    import(&mut app, ImportMsg::ConfirmDelete);
    let error = Error::new(ErrorKind::Protocol, "not deleting: the path is a symlink");
    app.update(Message::SitemanagerDeleted(Err(error.clone())));
    assert_eq!(app.import.delete, SourceDelete::Failed(error));

    // A report with failures doesn't offer to delete the source.
    let mut failed = report();
    failed.failures = vec!["A: disk full".into()];
    let mut app = at_report("/x.xml", failed);
    assert_eq!(app.import.delete, SourceDelete::Unavailable);
    import(&mut app, ImportMsg::Delete);
    assert!(import(&mut app, ImportMsg::ConfirmDelete).is_empty());
}

#[test]
fn export_refuses_empty_password_in_ui() {
    let mut app = App::new();
    app.update(Message::OpenSettings);
    assert!(matches!(app.form, Some(Form::Settings)));
    backup(&mut app, BackupMsg::OpenExport);
    assert_eq!(app.backup.panel, BackupPanel::Export);
    backup(
        &mut app,
        BackupMsg::ExportPath("/home/me/hoprs.backup".into()),
    );

    // Empty password: refused inline, nothing sent.
    assert!(backup(&mut app, BackupMsg::Export).is_empty());
    assert_eq!(
        app.backup.field_error,
        Some(strings::BACKUP_PASSWORD_REQUIRED)
    );
    assert_eq!(app.backup.export, BackupState::Idle);

    // A confirmation that differs: refused too.
    backup(&mut app, BackupMsg::ExportPassword("correct horse".into()));
    backup(&mut app, BackupMsg::ExportConfirm("correct hose".into()));
    assert!(backup(&mut app, BackupMsg::Export).is_empty());
    assert_eq!(
        app.backup.field_error,
        Some(strings::BACKUP_PASSWORDS_DIFFER)
    );

    // No file: refused.
    backup(&mut app, BackupMsg::ExportConfirm("correct horse".into()));
    backup(&mut app, BackupMsg::ExportPath(String::new()));
    assert!(backup(&mut app, BackupMsg::Export).is_empty());
    assert_eq!(app.backup.field_error, Some(strings::BACKUP_PATH_REQUIRED));
    backup(
        &mut app,
        BackupMsg::ExportPath("/home/me/hoprs.backup".into()),
    );

    // The password leaves the UI state with the effect; its Debug never shows it.
    let effects = backup(&mut app, BackupMsg::Export);
    match &effects[..] {
        [Effect::ExportBackup { path, password }] => {
            assert_eq!(path, &PathBuf::from("/home/me/hoprs.backup"));
            assert_eq!(password.expose_secret(), "correct horse");
        }
        other => panic!("expected one ExportBackup, got {other:?}"),
    }
    assert!(!format!("{effects:?}").contains("correct horse"));
    assert!(app.backup.export_password.0.is_empty());
    assert!(app.backup.export_confirm.0.is_empty());
    assert!(!format!("{app:?}").contains("correct horse"));
    assert_eq!(app.backup.field_error, None);
    assert_eq!(app.backup.export, BackupState::Running);
    assert!(app.busy());
    // A second export while one runs: nothing.
    backup(&mut app, BackupMsg::ExportPassword("again".into()));
    backup(&mut app, BackupMsg::ExportConfirm("again".into()));
    assert!(backup(&mut app, BackupMsg::Export).is_empty());

    // The report.
    let report = ExportReport {
        sites: 3,
        workspaces: 1,
        secrets: 2,
        skipped_unreadable: Vec::new(),
        missing_secrets: vec!["Old box".into()],
    };
    assert!(
        app.update(Message::BackupExported(Ok(report.clone())))
            .is_empty()
    );
    assert_eq!(app.backup.export, BackupState::Done(report));
    assert!(!app.busy());

    // "Choose…" asks where to save.
    let effects = backup(&mut app, BackupMsg::Choose(PickTarget::ExportBackup));
    assert!(matches!(
        effects[..],
        [Effect::PickFile(PickTarget::ExportBackup)]
    ));
    app.update(Message::FilePicked {
        target: PickTarget::ExportBackup,
        path: Some(PathBuf::from("/media/usb/hoprs.backup")),
    });
    assert_eq!(app.backup.export_path, "/media/usb/hoprs.backup");
}

#[test]
fn restore_replace_needs_extra_confirmation() {
    let mut app = App::new();
    app.update(Message::OpenSettings);
    backup(&mut app, BackupMsg::OpenRestore);
    assert_eq!(app.backup.panel, BackupPanel::Restore);
    backup(&mut app, BackupMsg::RestorePath("/b.backup".into()));
    assert!(backup(&mut app, BackupMsg::Restore).is_empty());
    assert_eq!(
        app.backup.field_error,
        Some(strings::BACKUP_PASSWORD_REQUIRED)
    );
    backup(&mut app, BackupMsg::RestorePassword("pw".into()));

    // Merge (the default) restores at once.
    assert_eq!(app.backup.restore_mode, ImportMode::Merge);
    let effects = backup(&mut app, BackupMsg::Restore);
    assert!(matches!(
        effects[..],
        [Effect::RestoreBackup {
            mode: ImportMode::Merge,
            ..
        }]
    ));
    assert!(app.backup.restore_password.0.is_empty());
    let done = BackupImportReport {
        added_sites: 2,
        secret_errors: vec!["Prod: keyring locked".into()],
        ..BackupImportReport::default()
    };
    // The restored profiles reach the sidebar.
    let effects = app.update(Message::BackupRestored(Ok(done.clone())));
    assert!(matches!(effects[..], [Effect::LoadProfiles]));
    assert_eq!(app.backup.restore, BackupState::Done(done));

    // Replace: the first click only asks.
    backup(&mut app, BackupMsg::RestoreMode(ImportMode::Replace));
    backup(&mut app, BackupMsg::RestorePassword("pw2".into()));
    assert!(backup(&mut app, BackupMsg::Restore).is_empty());
    assert!(app.backup.confirm_replace);
    assert_eq!(app.backup.restore_password.0, "pw2", "kept until confirmed");
    // Backing out keeps the vault as it is.
    assert!(backup(&mut app, BackupMsg::CancelReplace).is_empty());
    assert!(!app.backup.confirm_replace);
    // Confirming without being asked does nothing.
    assert!(backup(&mut app, BackupMsg::ConfirmReplace).is_empty());

    backup(&mut app, BackupMsg::Restore);
    let effects = backup(&mut app, BackupMsg::ConfirmReplace);
    match &effects[..] {
        [
            Effect::RestoreBackup {
                path,
                password,
                mode: ImportMode::Replace,
            },
        ] => {
            assert_eq!(path, &PathBuf::from("/b.backup"));
            assert_eq!(password.expose_secret(), "pw2");
        }
        other => panic!("expected one RestoreBackup, got {other:?}"),
    }
    assert!(!format!("{effects:?}").contains("pw2"));
    assert!(app.backup.restore_password.0.is_empty());
    assert!(!app.backup.confirm_replace);
    assert_eq!(app.backup.restore, BackupState::Running);

    // A wrong password comes back inline; nothing reloads.
    let error = Error::new(ErrorKind::Protocol, "Wrong password or damaged backup");
    assert!(
        app.update(Message::BackupRestored(Err(error.clone())))
            .is_empty()
    );
    assert_eq!(app.backup.restore, BackupState::Failed(error));

    // Switching back to Merge drops a pending Replace question.
    backup(&mut app, BackupMsg::RestorePassword("pw3".into()));
    backup(&mut app, BackupMsg::Restore);
    assert!(app.backup.confirm_replace);
    backup(&mut app, BackupMsg::RestoreMode(ImportMode::Merge));
    assert!(!app.backup.confirm_replace);
}

/// The settings page: changes apply to the engine and the sessions, and are saved;
/// an unreadable file is never overwritten.
#[test]
fn settings_changes_apply_and_save() {
    let mut app = App::new();
    let (_, boot) = App::boot();
    assert!(matches!(
        boot[..],
        [Effect::LoadProfiles, Effect::LoadSettings]
    ));
    app.update(Message::SettingsLoaded(SettingsLoad {
        settings: Settings {
            workers_per_site: 6,
            default_policy: ConflictPolicy::Skip,
        },
        error: None,
    }));
    assert_eq!(app.settings.workers_per_site, 6);
    app.update(Message::OpenSettings);

    // Workers: clamped, applied to the next connections, saved.
    let effects = settings(&mut app, SettingsMsg::Workers(40));
    assert_eq!(app.settings.workers_per_site, 16);
    match &effects[..] {
        [Effect::SetWorkers(16), Effect::SaveSettings(saved)] => {
            assert_eq!(saved.workers_per_site, 16);
            assert_eq!(saved.default_policy, ConflictPolicy::Skip);
        }
        other => panic!("{other:?}"),
    }
    let effects = settings(&mut app, SettingsMsg::Workers(0));
    assert_eq!(app.settings.workers_per_site, 1);
    assert!(matches!(
        effects[..],
        [Effect::SetWorkers(1), Effect::SaveSettings(_)]
    ));
    // The same value again: nothing to do.
    assert!(settings(&mut app, SettingsMsg::Workers(1)).is_empty());

    // The policy goes to the engine, and the queue's "this session" note goes away
    // (the engine now runs under the chosen policy).
    app.queue
        .set_session_policy(Some(ConflictPolicy::Overwrite));
    let effects = settings(&mut app, SettingsMsg::Policy(ConflictPolicy::Resume));
    match &effects[..] {
        [
            Effect::SetPolicy(ConflictPolicy::Resume),
            Effect::SaveSettings(saved),
        ] => {
            assert_eq!(saved.default_policy, ConflictPolicy::Resume);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(app.queue.session_policy(), None);

    // A failed save shows inline; the next success clears it.
    let error = Error::new(ErrorKind::Io, "Permission denied");
    app.update(Message::SettingsSaved(Err(error.clone())));
    assert_eq!(app.settings_save_error, Some(error));
    app.update(Message::SettingsSaved(Ok(())));
    assert_eq!(app.settings_save_error, None);

    // An unreadable file: the defaults, the error inline, and changes are applied but
    // never saved over the file.
    let mut app = App::new();
    let error = Error::new(ErrorKind::Protocol, "Can't read settings.toml");
    app.update(Message::SettingsLoaded(SettingsLoad {
        settings: Settings::default(),
        error: Some(error.clone()),
    }));
    assert_eq!(app.settings, Settings::default());
    assert_eq!(app.settings_load_error, Some(error));
    let effects = settings(&mut app, SettingsMsg::Workers(8));
    assert!(
        matches!(effects[..], [Effect::SetWorkers(8)]),
        "{effects:?}"
    );
    let effects = settings(&mut app, SettingsMsg::Policy(ConflictPolicy::Skip));
    assert!(matches!(
        effects[..],
        [Effect::SetPolicy(ConflictPolicy::Skip)]
    ));

    // Close returns to the tabs.
    settings(&mut app, SettingsMsg::Close);
    assert!(app.form.is_none());
}

/// The real effects against the fake backend: the import, the delete, the backup and
/// the settings reach it, and come back as messages.
#[tokio::test]
async fn effects_reach_the_backend() {
    let fake = Arc::new(FakeBackend::new());
    let backend: Arc<dyn BackendApi> = fake.clone();

    let message = effects::import_filezilla(backend.clone(), PathBuf::from("/s.xml"), true).await;
    assert!(matches!(message, Message::Imported(Ok(_))));
    let message = effects::delete_sitemanager(backend.clone(), PathBuf::from("/s.xml")).await;
    assert!(matches!(message, Message::SitemanagerDeleted(Ok(()))));
    let message = effects::export_backup(
        backend.clone(),
        PathBuf::from("/b"),
        SecretString::from("pw"),
    )
    .await;
    assert!(matches!(message, Message::BackupExported(Ok(_))));
    let message = effects::restore_backup(
        backend.clone(),
        PathBuf::from("/b"),
        SecretString::from("pw"),
        ImportMode::Replace,
    )
    .await;
    assert!(matches!(message, Message::BackupRestored(Ok(_))));
    let saved = Settings {
        workers_per_site: 9,
        default_policy: ConflictPolicy::Overwrite,
    };
    let message = effects::save_settings(backend.clone(), saved).await;
    assert!(matches!(message, Message::SettingsSaved(Ok(()))));
    backend.set_policy(ConflictPolicy::Skip).await;
    backend.set_workers(3);

    assert_eq!(
        fake.backup_calls(),
        [
            BackupCall::Import {
                path: PathBuf::from("/s.xml"),
                folders_as_workspaces: true,
            },
            BackupCall::DeleteSitemanager(PathBuf::from("/s.xml")),
            BackupCall::Export {
                path: PathBuf::from("/b"),
                password: "pw".into(),
            },
            BackupCall::Restore {
                path: PathBuf::from("/b"),
                password: "pw".into(),
                mode: ImportMode::Replace,
            },
            BackupCall::SaveSettings(saved),
            BackupCall::SetPolicy(ConflictPolicy::Skip),
            BackupCall::SetWorkers(3),
        ]
    );
}
