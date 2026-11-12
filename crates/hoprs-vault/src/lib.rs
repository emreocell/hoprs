//! Local storage of saved sites and workspaces.
//!
//! Profiles live in two TOML files (`sites.toml`, `workspaces.toml`) under
//! [`default_dir`]. They never contain secrets.

mod atomic;
mod profiles;

use std::path::{Path, PathBuf};

use hoprs_core::{Error, ErrorKind, Result, Site, SiteId, Workspace, WorkspaceId};

pub use profiles::UnreadableEntry;

use profiles::Profiles;

/// Default profile directory: `<config dir>/hoprs` (e.g. `~/.config/hoprs`).
pub fn default_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("hoprs"))
}

/// Saved sites and workspaces. Every mutation is written to disk before it
/// becomes visible; a failed write leaves the vault unchanged.
#[derive(Debug)]
pub struct Vault {
    dir: PathBuf,
    profiles: Profiles,
}

impl Vault {
    /// Opens the profiles in `dir` without a secret store.
    ///
    /// Temporary constructor: replaced by `Vault::open(dir, secrets)` once
    /// secrets are wired in.
    pub fn open_profiles_only(dir: &Path) -> Result<Vault> {
        Ok(Vault {
            dir: dir.to_path_buf(),
            profiles: profiles::load(dir)?,
        })
    }

    pub fn sites(&self) -> &[Site] {
        &self.profiles.sites
    }

    pub fn site(&self, id: &SiteId) -> Option<&Site> {
        self.profiles.sites.iter().find(|s| &s.id == id)
    }

    /// Inserts `site`, or replaces the site with the same id, and saves.
    pub fn upsert_site(&mut self, site: Site) -> Result<()> {
        if self.profiles.is_unreadable_site_id(&site.id.0) {
            return Err(Error::new(
                ErrorKind::Protocol,
                "Site id is taken by an unreadable entry",
            )
            .with_detail(site.id.0));
        }
        let mut next = self.profiles.clone();
        match next.sites.iter_mut().find(|s| s.id == site.id) {
            Some(slot) => *slot = site,
            None => next.sites.push(site),
        }
        profiles::save_sites(&self.dir, &next)?;
        self.profiles = next;
        Ok(())
    }

    pub fn workspaces(&self) -> &[Workspace] {
        &self.profiles.workspaces
    }

    /// Inserts `workspace`, or replaces the one with the same id, and saves.
    /// Every site it references must exist (readable or not).
    pub fn upsert_workspace(&mut self, workspace: Workspace) -> Result<()> {
        if let Some(missing) = workspace.sites.iter().find(|ws| {
            self.site(&ws.site).is_none() && !self.profiles.is_unreadable_site_id(&ws.site.0)
        }) {
            return Err(Error::new(ErrorKind::Protocol, "Unknown site in workspace")
                .with_detail(missing.site.0.clone()));
        }
        if self.profiles.is_unreadable_workspace_id(&workspace.id.0) {
            return Err(Error::new(
                ErrorKind::Protocol,
                "Workspace id is taken by an unreadable entry",
            )
            .with_detail(workspace.id.0));
        }
        let mut next = self.profiles.clone();
        match next.workspaces.iter_mut().find(|w| w.id == workspace.id) {
            Some(slot) => *slot = workspace,
            None => next.workspaces.push(workspace),
        }
        profiles::save_workspaces(&self.dir, &next)?;
        self.profiles = next;
        Ok(())
    }

    pub fn remove_workspace(&mut self, id: &WorkspaceId) -> Result<()> {
        let Some(pos) = self.profiles.workspaces.iter().position(|w| &w.id == id) else {
            return Err(
                Error::new(ErrorKind::NotFound, "Unknown workspace").with_detail(id.0.clone())
            );
        };
        let mut next = self.profiles.clone();
        next.workspaces.remove(pos);
        next.forget_workspace_extras(&id.0);
        profiles::save_workspaces(&self.dir, &next)?;
        self.profiles = next;
        Ok(())
    }

    /// Entries kept on disk but not understood by this version of hoprs.
    pub fn unreadable_entries(&self) -> &[UnreadableEntry] {
        self.profiles.unreadable()
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use hoprs_core::{Protocol, SavedTab, TabKind, WorkspaceSite};

    use super::*;

    fn ssh_site(name: &str) -> Site {
        Site::new(name, "example.com", "deploy", Protocol::Ssh)
    }

    fn workspace(name: &str, sites: &[&Site]) -> Workspace {
        Workspace {
            id: WorkspaceId::new(),
            name: name.into(),
            sites: sites
                .iter()
                .map(|s| WorkspaceSite {
                    site: s.id.clone(),
                    tabs: vec![],
                })
                .collect(),
        }
    }

    fn read_table(path: &Path) -> toml::Table {
        fs::read_to_string(path).unwrap().parse().unwrap()
    }

    #[test]
    fn default_dir_ends_with_hoprs() {
        if let Some(dir) = default_dir() {
            assert!(dir.ends_with("hoprs"), "{}", dir.display());
        }
    }

    #[test]
    fn empty_dir_loads_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = Vault::open_profiles_only(tmp.path()).unwrap();
        assert!(vault.sites().is_empty());
        assert!(vault.workspaces().is_empty());
        assert!(vault.unreadable_entries().is_empty());
        // Opening must not create anything.
        assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 0);
    }

    #[test]
    fn missing_dir_loads_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = Vault::open_profiles_only(&tmp.path().join("nope")).unwrap();
        assert!(vault.sites().is_empty());
    }

    #[test]
    fn roundtrip_sites_and_workspaces() {
        let tmp = tempfile::tempdir().unwrap();
        let mut a = ssh_site("A");
        a.ssh.key_file = Some(PathBuf::from("keys").join("id_ed25519"));
        a.remote_dir = Some("/srv".into());
        a.has_password = true;
        let mut b = Site::new("B", "ftp.example.com", "web", Protocol::FtpsImplicit);
        b.ftp.pinned_fingerprint = Some("AB:CD".into());
        b.local_dir = Some(PathBuf::from("work"));
        let mut ws = workspace("Prod", &[&a, &b]);
        ws.sites[0].tabs = vec![
            SavedTab {
                kind: TabKind::Terminal,
                remote_dir: None,
                local_dir: None,
            },
            SavedTab {
                kind: TabKind::Files,
                remote_dir: Some("/var/www".into()),
                local_dir: Some(PathBuf::from("site")),
            },
        ];
        {
            let mut vault = Vault::open_profiles_only(tmp.path()).unwrap();
            vault.upsert_site(a.clone()).unwrap();
            vault.upsert_site(b.clone()).unwrap();
            vault.upsert_workspace(ws.clone()).unwrap();
            assert_eq!(vault.site(&b.id), Some(&b));
        }
        let vault = Vault::open_profiles_only(tmp.path()).unwrap();
        assert_eq!(vault.sites(), [a.clone(), b.clone()]);
        assert_eq!(vault.workspaces(), [ws]);
        assert_eq!(vault.site(&a.id), Some(&a));
        assert_eq!(vault.site(&SiteId::new()), None);

        let sites = fs::read_to_string(tmp.path().join("sites.toml")).unwrap();
        assert!(sites.starts_with("version = 1\n"), "{sites}");
        assert!(sites.contains("[[site]]"), "{sites}");
        assert!(!sites.lines().any(|l| l.starts_with("password")), "{sites}");
        let workspaces = fs::read_to_string(tmp.path().join("workspaces.toml")).unwrap();
        assert!(workspaces.starts_with("version = 1\n"), "{workspaces}");
        assert!(workspaces.contains("[[workspace]]"), "{workspaces}");
    }

    #[test]
    fn upsert_replaces_by_id() {
        let tmp = tempfile::tempdir().unwrap();
        let mut vault = Vault::open_profiles_only(tmp.path()).unwrap();
        let mut a = ssh_site("A");
        vault.upsert_site(a.clone()).unwrap();
        vault.upsert_site(ssh_site("B")).unwrap();
        a.name = "A2".into();
        vault.upsert_site(a.clone()).unwrap();
        assert_eq!(vault.sites().len(), 2);
        assert_eq!(vault.sites()[0], a);
        let reopened = Vault::open_profiles_only(tmp.path()).unwrap();
        assert_eq!(reopened.sites(), vault.sites());
    }

    #[test]
    fn remove_workspace_saves_and_unknown_is_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        let mut vault = Vault::open_profiles_only(tmp.path()).unwrap();
        let s = ssh_site("A");
        vault.upsert_site(s.clone()).unwrap();
        let w1 = workspace("one", &[&s]);
        let w2 = workspace("two", &[&s]);
        vault.upsert_workspace(w1.clone()).unwrap();
        vault.upsert_workspace(w2.clone()).unwrap();
        vault.remove_workspace(&w1.id).unwrap();
        assert_eq!(vault.workspaces(), std::slice::from_ref(&w2));
        let err = vault.remove_workspace(&w1.id).unwrap_err();
        assert_eq!(err.kind, ErrorKind::NotFound);
        let reopened = Vault::open_profiles_only(tmp.path()).unwrap();
        assert_eq!(reopened.workspaces(), [w2]);
    }

    #[test]
    fn corrupt_sites_toml_is_protocol_and_untouched() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("sites.toml");
        fs::write(&path, "[[site]\nname =").unwrap();
        let err = Vault::open_profiles_only(tmp.path()).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Protocol);
        assert_eq!(err.message, "Cannot read sites.toml");
        let detail = err.detail.unwrap();
        assert!(detail.contains("line"), "{detail}");
        assert_eq!(fs::read(&path).unwrap(), b"[[site]\nname =");
    }

    #[test]
    fn corrupt_workspaces_toml_is_protocol() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("workspaces.toml"), "version = = 1").unwrap();
        let err = Vault::open_profiles_only(tmp.path()).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Protocol);
        assert_eq!(err.message, "Cannot read workspaces.toml");
    }

    #[test]
    fn missing_version_is_protocol() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("sites.toml"), "[[site]]\nname = \"x\"\n").unwrap();
        let err = Vault::open_profiles_only(tmp.path()).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Protocol);
    }

    #[test]
    fn newer_version_is_refused() {
        for file in ["sites.toml", "workspaces.toml"] {
            let tmp = tempfile::tempdir().unwrap();
            let path = tmp.path().join(file);
            fs::write(&path, "version = 2\n").unwrap();
            let err = Vault::open_profiles_only(tmp.path()).unwrap_err();
            assert_eq!(err.kind, ErrorKind::Protocol, "{file}");
            assert_eq!(err.message, "Profile was written by a newer hoprs");
            assert_eq!(fs::read(&path).unwrap(), b"version = 2\n");
        }
    }

    #[test]
    fn unknown_fields_survive_a_save() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("sites.toml");
        fs::write(
            &path,
            r#"version = 1
future_top = true

[[site]]
id = "s1"
name = "Old"
host = "h"
port = 22
user = "u"
protocol = "ssh"
future_field = "x"

[site.ssh]
use_agent = false
future_nested = 7
"#,
        )
        .unwrap();
        let mut vault = Vault::open_profiles_only(tmp.path()).unwrap();
        assert!(!vault.sites()[0].ssh.use_agent);
        vault.upsert_site(ssh_site("New")).unwrap();
        // Editing the site itself keeps its unknown fields as well.
        let mut old = vault.sites()[0].clone();
        old.name = "Renamed".into();
        vault.upsert_site(old).unwrap();

        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("future_field = \"x\""), "{text}");
        let doc = read_table(&path);
        assert_eq!(doc["future_top"].as_bool(), Some(true));
        let first = doc["site"].as_array().unwrap()[0].as_table().unwrap();
        assert_eq!(first["name"].as_str(), Some("Renamed"));
        assert_eq!(first["future_field"].as_str(), Some("x"));
        assert_eq!(first["ssh"]["future_nested"].as_integer(), Some(7));
        assert_eq!(first["ssh"]["use_agent"].as_bool(), Some(false));

        let reopened = Vault::open_profiles_only(tmp.path()).unwrap();
        assert_eq!(reopened.sites(), vault.sites());
    }

    #[test]
    fn cleared_optional_field_is_not_resurrected() {
        let tmp = tempfile::tempdir().unwrap();
        let mut vault = Vault::open_profiles_only(tmp.path()).unwrap();
        let mut s = ssh_site("A");
        s.remote_dir = Some("/srv".into());
        s.ssh.key_file = Some(PathBuf::from("key"));
        vault.upsert_site(s.clone()).unwrap();
        let mut vault = Vault::open_profiles_only(tmp.path()).unwrap();
        s.remote_dir = None;
        s.ssh.key_file = None;
        vault.upsert_site(s.clone()).unwrap();
        let reopened = Vault::open_profiles_only(tmp.path()).unwrap();
        assert_eq!(reopened.sites(), [s]);
    }

    #[test]
    fn unknown_protocol_entry_is_preserved_and_reported() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("sites.toml");
        fs::write(
            &path,
            r#"version = 1

[[site]]
id = "s1"
name = "First"
host = "h"
port = 22
user = "u"
protocol = "ssh"

[[site]]
id = "s3-id"
name = "Bucket"
host = "s3.example.com"
port = 443
user = "u"
protocol = "s3"
region = "eu-west-1"

[[site]]
id = "s2"
name = "Second"
host = "h"
port = 21
user = "u"
protocol = "ftp"
"#,
        )
        .unwrap();
        let original_entry = read_table(&path)["site"].as_array().unwrap()[1].clone();

        let mut vault = Vault::open_profiles_only(tmp.path()).unwrap();
        let ids: Vec<_> = vault.sites().iter().map(|s| s.id.0.as_str()).collect();
        assert_eq!(ids, ["s1", "s2"]);
        let unreadable = vault.unreadable_entries();
        assert_eq!(unreadable.len(), 1);
        assert_eq!(unreadable[0].file, "sites.toml");
        assert_eq!(unreadable[0].index, 1);
        assert_eq!(unreadable[0].name.as_deref(), Some("Bucket"));
        assert!(
            unreadable[0].reason.contains("s3"),
            "{}",
            unreadable[0].reason
        );

        vault.upsert_site(ssh_site("Third")).unwrap();

        let entries = read_table(&path)["site"].as_array().unwrap().clone();
        assert_eq!(entries.len(), 4);
        assert_eq!(entries[1], original_entry, "kept verbatim, in place");

        let reopened = Vault::open_profiles_only(tmp.path()).unwrap();
        assert_eq!(reopened.sites().len(), 3);
        assert!(reopened.sites().iter().all(|s| s.id.0 != "s3-id"));
        assert_eq!(reopened.unreadable_entries(), vault.unreadable_entries());
    }

    #[test]
    fn unreadable_entry_ids_are_reserved() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            tmp.path().join("sites.toml"),
            "version = 1\n\n[[site]]\nid = \"dup\"\nname = \"x\"\nprotocol = \"s3\"\n\n\
             [[site]]\nid = \"dup\"\nname = \"y\"\nhost = \"h\"\nport = 22\nuser = \"u\"\nprotocol = \"ssh\"\n",
        )
        .unwrap();
        let err = Vault::open_profiles_only(tmp.path()).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Protocol);

        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            tmp.path().join("sites.toml"),
            "version = 1\n\n[[site]]\nid = \"future\"\nname = \"x\"\nprotocol = \"s3\"\n",
        )
        .unwrap();
        let mut vault = Vault::open_profiles_only(tmp.path()).unwrap();
        let mut clash = ssh_site("clash");
        clash.id = SiteId("future".into());
        assert_eq!(
            vault.upsert_site(clash).unwrap_err().kind,
            ErrorKind::Protocol
        );
        // A workspace may still reference the unreadable site.
        let mut ws = workspace("w", &[]);
        ws.sites.push(WorkspaceSite {
            site: SiteId("future".into()),
            tabs: vec![],
        });
        vault.upsert_workspace(ws).unwrap();
    }

    #[test]
    fn unreadable_workspace_is_preserved() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("workspaces.toml");
        fs::write(
            &path,
            "version = 1\n\n[[workspace]]\nid = \"w1\"\nname = \"Odd\"\n\n\
             [[workspace.sites]]\nsite = \"s\"\ntabs = [{ kind = \"hologram\" }]\n",
        )
        .unwrap();
        let original = read_table(&path)["workspace"].clone();
        let mut vault = Vault::open_profiles_only(tmp.path()).unwrap();
        assert!(vault.workspaces().is_empty());
        assert_eq!(vault.unreadable_entries()[0].file, "workspaces.toml");
        assert_eq!(vault.unreadable_entries()[0].name.as_deref(), Some("Odd"));
        vault.upsert_workspace(workspace("New", &[])).unwrap();
        let entries = read_table(&path)["workspace"].as_array().unwrap().clone();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0], original.as_array().unwrap()[0]);
    }

    #[test]
    fn duplicate_ids_are_protocol() {
        let tmp = tempfile::tempdir().unwrap();
        let entry = "[[site]]\nid = \"same\"\nname = \"n\"\nhost = \"h\"\nport = 22\nuser = \"u\"\nprotocol = \"ssh\"\n";
        fs::write(
            tmp.path().join("sites.toml"),
            format!("version = 1\n\n{entry}\n{entry}"),
        )
        .unwrap();
        let err = Vault::open_profiles_only(tmp.path()).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Protocol);

        let tmp = tempfile::tempdir().unwrap();
        let entry = "[[workspace]]\nid = \"w\"\nname = \"n\"\n";
        fs::write(
            tmp.path().join("workspaces.toml"),
            format!("version = 1\n\n{entry}\n{entry}"),
        )
        .unwrap();
        let err = Vault::open_profiles_only(tmp.path()).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Protocol);
    }

    #[test]
    fn workspace_with_unknown_site_is_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let mut vault = Vault::open_profiles_only(tmp.path()).unwrap();
        let known = ssh_site("known");
        vault.upsert_site(known.clone()).unwrap();
        let ghost = ssh_site("ghost");
        let err = vault
            .upsert_workspace(workspace("w", &[&known, &ghost]))
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::Protocol);
        assert_eq!(err.message, "Unknown site in workspace");
        assert!(vault.workspaces().is_empty());
        assert!(!tmp.path().join("workspaces.toml").exists());
    }

    /// Restores a directory's permissions on drop so `TempDir` can clean up.
    #[cfg(unix)]
    struct RestoreMode(PathBuf);

    #[cfg(unix)]
    impl Drop for RestoreMode {
        fn drop(&mut self) {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o700));
        }
    }

    #[cfg(unix)]
    #[test]
    fn failed_save_leaves_memory_and_disk_unchanged() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("hoprs");
        let mut vault = Vault::open_profiles_only(&dir).unwrap();
        let a = ssh_site("A");
        vault.upsert_site(a.clone()).unwrap();
        let path = dir.join("sites.toml");
        let before = fs::read(&path).unwrap();

        let _guard = RestoreMode(dir.clone());
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o500)).unwrap();
        if fs::write(dir.join("probe"), b"").is_ok() {
            // Privileged user: permissions are not enforced, nothing to test.
            return;
        }

        let mut renamed = a.clone();
        renamed.name = "Renamed".into();
        assert!(vault.upsert_site(ssh_site("B")).is_err());
        assert!(vault.upsert_site(renamed).is_err());
        assert_eq!(vault.sites(), std::slice::from_ref(&a));
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1, "no temporary left");

        drop(_guard);
        let reopened = Vault::open_profiles_only(&dir).unwrap();
        assert_eq!(reopened.sites(), [a]);
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_path_fails_save_without_panic() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let tmp = tempfile::tempdir().unwrap();
        let mut vault = Vault::open_profiles_only(tmp.path()).unwrap();
        let a = ssh_site("A");
        vault.upsert_site(a.clone()).unwrap();
        let before = fs::read(tmp.path().join("sites.toml")).unwrap();

        let mut bad = ssh_site("Bad");
        bad.local_dir = Some(PathBuf::from(OsStr::from_bytes(b"dir\xff")));
        let err = vault.upsert_site(bad.clone()).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Protocol);
        let detail = err.detail.unwrap();
        assert!(detail.contains(&bad.id.0), "{detail}");
        assert_eq!(vault.sites(), [a]);
        assert_eq!(fs::read(tmp.path().join("sites.toml")).unwrap(), before);
    }

    #[cfg(unix)]
    #[test]
    fn file_modes_are_private() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("config").join("hoprs");
        let mut vault = Vault::open_profiles_only(&dir).unwrap();
        let s = ssh_site("A");
        vault.upsert_site(s.clone()).unwrap();
        vault.upsert_workspace(workspace("w", &[&s])).unwrap();

        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&dir.join("sites.toml")), 0o600);
        assert_eq!(mode(&dir.join("workspaces.toml")), 0o600);
    }
}
