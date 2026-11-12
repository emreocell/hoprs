use std::path::{Path, PathBuf};
use std::sync::Arc;

use hoprs_core::{Error, ErrorKind, FtpMode, Protocol, Result, SiteId};
use hoprs_import::{ImportOptions, apply, delete_source, parse_sitemanager};
use hoprs_vault::{MemoryStore, SecretKind, SecretStore, Vault};
use secrecy::{ExposeSecret, SecretString};
use tempfile::TempDir;

fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn parsed(name: &str) -> hoprs_import::ParsedSiteManager {
    parse_sitemanager(&std::fs::read_to_string(fixture_path(name)).unwrap()).unwrap()
}

fn open() -> (TempDir, Arc<MemoryStore>, Vault) {
    let dir = TempDir::new().unwrap();
    let store = Arc::new(MemoryStore::new());
    let vault = Vault::open(dir.path(), store.clone()).unwrap();
    (dir, store, vault)
}

const FLAT: ImportOptions = ImportOptions {
    folders_as_workspaces: false,
};
const WS: ImportOptions = ImportOptions {
    folders_as_workspaces: true,
};

#[test]
fn imports_supported_sites_with_secrets() {
    let (dir, store, mut vault) = open();
    let report = apply(&mut vault, parsed("sitemanager-3.66.xml"), &FLAT).unwrap();
    // 10 sites, one with an unsupported protocol.
    assert_eq!(report.imported.len(), 9);
    assert_eq!(report.skipped.len(), 1);
    assert_eq!(vault.sites().len(), 9);

    let legacy = vault
        .sites()
        .iter()
        .find(|s| s.name == "Legacy FTP")
        .unwrap();
    assert_eq!(legacy.protocol, Protocol::FtpsExplicit);
    assert!(legacy.has_password);
    let pw = store
        .get(&legacy.id, SecretKind::Password)
        .unwrap()
        .unwrap();
    assert_eq!(pw.expose_secret(), "hunter2-fictional");

    let shell = vault.sites().iter().find(|s| s.name == "Shell").unwrap();
    assert_eq!(shell.port, 2222);
    assert_eq!(
        shell.ssh.key_file.as_deref(),
        Some(Path::new("/home/alice/.ssh/id_ed25519"))
    );
    assert!(!shell.has_password);
    let implicit = vault
        .sites()
        .iter()
        .find(|s| s.name == "Implicit TLS")
        .unwrap();
    assert_eq!(implicit.ftp.mode, FtpMode::Active);
    let explicit = vault
        .sites()
        .iter()
        .find(|s| s.name == "Explicit TLS")
        .unwrap();
    assert_eq!(explicit.port, 21);

    // The password is nowhere in the report or on disk.
    let report_text = format!("{report:?}");
    assert!(!report_text.contains("hunter2"));
    for file in std::fs::read_dir(dir.path()).unwrap() {
        let text = std::fs::read_to_string(file.unwrap().path()).unwrap_or_default();
        assert!(!text.contains("hunter2"), "{text}");
    }
    // Report order follows the XML.
    assert_eq!(report.imported[0].name, "Legacy FTP");
    assert_eq!(report.skipped[0].name, "Object storage");
}

#[test]
fn folders_as_workspaces_creates_workspaces() {
    let (_dir, _store, mut vault) = open();
    let report = apply(&mut vault, parsed("sitemanager-3.40-folders.xml"), &WS).unwrap();
    assert_eq!(
        report.workspaces_created,
        vec![
            "Work",
            "Work/Clientes de Produção",
            "Work/Clientes de Produção/Deep"
        ]
    );
    assert_eq!(vault.workspaces().len(), 3);
    let names = |ws: &str| -> Vec<String> {
        let w = vault.workspaces().iter().find(|w| w.name == ws).unwrap();
        w.sites
            .iter()
            .map(|s| vault.site(&s.site).unwrap().name.clone())
            .collect()
    };
    assert_eq!(names("Work"), vec!["Alpha"]);
    assert_eq!(names("Work/Clientes de Produção"), vec!["Beta"]);
    assert_eq!(names("Work/Clientes de Produção/Deep"), vec!["Gamma"]);
    // Names are not prefixed in this mode.
    assert!(vault.sites().iter().any(|s| s.name == "Top level"));
}

#[test]
fn folders_flattened_into_names() {
    let (_dir, _store, mut vault) = open();
    let report = apply(&mut vault, parsed("sitemanager-3.40-folders.xml"), &FLAT).unwrap();
    assert!(report.workspaces_created.is_empty());
    assert!(vault.workspaces().is_empty());
    let names: Vec<_> = report.imported.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "Work/Alpha",
            "Work/Clientes de Produção/Beta",
            "Work/Clientes de Produção/Deep/Gamma",
            "Top level"
        ]
    );
}

#[test]
fn skipped_sites_have_reasons() {
    let (_dir, _store, mut vault) = open();
    let report = apply(&mut vault, parsed("sitemanager-3.66.xml"), &FLAT).unwrap();
    assert_eq!(
        report.skipped,
        vec![hoprs_import::SkippedSite {
            name: "Object storage".into(),
            reason: "unsupported protocol 7".into()
        }]
    );
    assert!(!vault.sites().iter().any(|s| s.name == "Object storage"));
}

#[test]
fn encrypted_passwords_import_without_secret_and_note() {
    let (_dir, store, mut vault) = open();
    let report = apply(&mut vault, parsed("sitemanager-crypt.xml"), &FLAT).unwrap();
    let locked = report.imported.iter().find(|s| s.name == "Locked").unwrap();
    assert_eq!(
        locked.notes,
        vec!["password is protected by a FileZilla master password; enter it in hoprs"]
    );
    let site = vault.site(&locked.id).unwrap();
    assert!(!site.has_password);
    assert!(
        store
            .get(&locked.id, SecretKind::Password)
            .unwrap()
            .is_none()
    );
}

#[test]
fn reimport_is_idempotent() {
    let (_dir, _store, mut vault) = open();
    let first = apply(&mut vault, parsed("sitemanager-3.40-folders.xml"), &WS).unwrap();
    let again = apply(&mut vault, parsed("sitemanager-3.40-folders.xml"), &WS).unwrap();
    assert!(again.imported.is_empty());
    assert!(again.workspaces_created.is_empty());
    assert_eq!(again.skipped.len(), first.imported.len());
    assert!(again.skipped.iter().all(|s| s.reason == "already in hoprs"));
    assert_eq!(vault.sites().len(), first.imported.len());
    assert_eq!(vault.workspaces().len(), 3);

    let (_dir, _store, mut vault) = open();
    let first = apply(&mut vault, parsed("sitemanager-3.66.xml"), &FLAT).unwrap();
    let again = apply(&mut vault, parsed("sitemanager-3.66.xml"), &FLAT).unwrap();
    assert!(again.imported.is_empty());
    assert_eq!(
        again.skipped.len(),
        first.imported.len() + first.skipped.len()
    );
    assert_eq!(vault.sites().len(), first.imported.len());
}

struct FailingStore;

impl SecretStore for FailingStore {
    fn get(&self, _: &SiteId, _: SecretKind) -> Result<Option<SecretString>> {
        Ok(None)
    }
    fn set(&self, _: &SiteId, _: SecretKind, _: &SecretString) -> Result<()> {
        Err(Error::new(ErrorKind::Io, "System keyring unavailable"))
    }
    fn delete(&self, _: &SiteId, _: SecretKind) -> Result<()> {
        Ok(())
    }
}

#[test]
fn keyring_failure_still_imports_site() {
    let dir = TempDir::new().unwrap();
    let mut vault = Vault::open(dir.path(), Arc::new(FailingStore)).unwrap();
    let report = apply(&mut vault, parsed("sitemanager-3.66.xml"), &FLAT).unwrap();
    assert_eq!(report.imported.len(), 9);
    let legacy = report
        .imported
        .iter()
        .find(|s| s.name == "Legacy FTP")
        .unwrap();
    assert!(
        legacy
            .notes
            .contains(&"password not saved: System keyring unavailable".to_string()),
        "{:?}",
        legacy.notes
    );
    assert!(!format!("{report:?}").contains("hunter2"));
    assert!(!vault.site(&legacy.id).unwrap().has_password);
}

#[test]
fn delete_source_only_deletes_filezilla_xml() {
    let dir = TempDir::new().unwrap();
    let xml = dir.path().join("sitemanager.xml");
    std::fs::copy(fixture_path("sitemanager-3.66.xml"), &xml).unwrap();
    delete_source(&xml).unwrap();
    assert!(!xml.exists());

    let other = dir.path().join("notes.txt");
    std::fs::write(&other, "<?xml version=\"1.0\"?><other/>").unwrap();
    assert_eq!(delete_source(&other).unwrap_err().kind, ErrorKind::Protocol);
    let plain = dir.path().join("plain.txt");
    std::fs::write(&plain, "hello <FileZilla3").unwrap();
    assert_eq!(delete_source(&plain).unwrap_err().kind, ErrorKind::Protocol);
    assert!(other.exists() && plain.exists());

    assert_eq!(
        delete_source(&dir.path().join("missing.xml"))
            .unwrap_err()
            .kind,
        ErrorKind::NotFound
    );
    assert_eq!(
        delete_source(dir.path()).unwrap_err().kind,
        ErrorKind::Protocol
    );
}

#[cfg(unix)]
#[test]
fn delete_source_refuses_symlink() {
    let dir = TempDir::new().unwrap();
    let real = dir.path().join("real.xml");
    std::fs::copy(fixture_path("sitemanager-3.66.xml"), &real).unwrap();
    let link = dir.path().join("link.xml");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    assert_eq!(delete_source(&link).unwrap_err().kind, ErrorKind::Protocol);
    assert!(real.exists() && link.exists());
}
