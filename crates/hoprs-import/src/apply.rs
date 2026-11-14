//! Writes a parsed FileZilla import into a [`Vault`].

use std::io::Read;
use std::path::Path;

use hoprs_core::{
    Error, ErrorKind, Protocol, Result, Site, SiteId, Workspace, WorkspaceId, WorkspaceSite,
};
use hoprs_vault::{SecretKind, Vault, is_keyring_unavailable};

use crate::{ParsedEntry, ParsedPassword, ParsedSite, ParsedSiteManager};

#[derive(Clone, Copy, Debug, Default)]
pub struct ImportOptions {
    /// One workspace per FileZilla folder (named by its path). When false,
    /// the folder path is prefixed to the site names instead.
    pub folders_as_workspaces: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ImportedSite {
    pub name: String,
    pub id: SiteId,
    pub notes: Vec<String>,
    /// The file had a password for this site but the secret store refused it: the
    /// source file is still the only copy of that password.
    pub secret_not_saved: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkippedSite {
    pub name: String,
    pub reason: String,
}

/// What an import did, in XML order. Never contains passwords.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ImportReport {
    pub imported: Vec<ImportedSite>,
    pub skipped: Vec<SkippedSite>,
    pub workspaces_created: Vec<String>,
    /// Folders whose workspace was not created, as "<name>: <reason>".
    pub skipped_workspaces: Vec<String>,
    /// Sites or workspaces that could not be written, as "<name>: <message>".
    pub failures: Vec<String>,
}

/// Applies `parsed` to the vault: sites, passwords (to the secret store) and,
/// optionally, folders as workspaces. Re-importing the same file skips every
/// site already present (same name, host, port, user and protocol) and every
/// workspace whose name already exists.
///
/// A write that fails does not stop the import: it is recorded in
/// [`ImportReport::failures`] and the remaining sites are still tried, so the
/// report always says what was persisted. A folder whose workspace is missing
/// gets one made of its imported sites and of the existing sites it matched,
/// so re-running after a failure recovers. `Err` is reserved for failures
/// before anything is applied (none today).
pub fn apply(
    vault: &mut Vault,
    parsed: ParsedSiteManager,
    options: &ImportOptions,
) -> Result<ImportReport> {
    let mut report = ImportReport::default();
    let mut folders: Vec<(String, Vec<SiteId>)> = Vec::new();
    let mut path = Vec::new();
    let mut keyring_down = false;
    walk(
        vault,
        parsed.entries,
        options,
        &mut path,
        &mut report,
        &mut folders,
        &mut keyring_down,
    );

    for (name, ids) in folders {
        if ids.is_empty() {
            continue;
        }
        if vault.workspaces().iter().any(|w| w.name == name) {
            report
                .skipped_workspaces
                .push(format!("{name}: a workspace with this name already exists"));
            continue;
        }
        let workspace = Workspace {
            id: WorkspaceId::new(),
            name: name.clone(),
            sites: ids
                .into_iter()
                .map(|site| WorkspaceSite {
                    site,
                    tabs: Vec::new(),
                })
                .collect(),
        };
        match vault.upsert_workspace(workspace) {
            Ok(()) => report.workspaces_created.push(name),
            Err(err) => report.failures.push(format!("{name}: {}", err.message)),
        }
    }
    Ok(report)
}

fn walk(
    vault: &mut Vault,
    entries: Vec<ParsedEntry>,
    options: &ImportOptions,
    path: &mut Vec<String>,
    report: &mut ImportReport,
    folders: &mut Vec<(String, Vec<SiteId>)>,
    keyring_down: &mut bool,
) {
    // Workspaces are created in XML (pre-)order: reserve this folder's slot.
    let slot = (!path.is_empty() && options.folders_as_workspaces).then(|| {
        folders.push((path.join("/"), Vec::new()));
        folders.len() - 1
    });
    for entry in entries {
        match entry {
            ParsedEntry::Site(site) => {
                let prefix = if options.folders_as_workspaces {
                    &[][..]
                } else {
                    &path[..]
                };
                if let Some(id) = import_site(vault, site, prefix, report, keyring_down)
                    && let Some(slot) = slot
                {
                    folders[slot].1.push(id);
                }
            }
            ParsedEntry::Folder { name, entries } => {
                path.push(name);
                walk(vault, entries, options, path, report, folders, keyring_down);
                path.pop();
            }
        }
    }
}

fn import_site(
    vault: &mut Vault,
    parsed: ParsedSite,
    prefix: &[String],
    report: &mut ImportReport,
    keyring_down: &mut bool,
) -> Option<SiteId> {
    let name = if prefix.is_empty() {
        parsed.name.clone()
    } else {
        format!("{}/{}", prefix.join("/"), parsed.name)
    };
    let protocol = match parsed.protocol {
        Ok(protocol) => protocol,
        Err(reason) => {
            report.skipped.push(SkippedSite { name, reason });
            return None;
        }
    };
    if parsed.host.is_empty() {
        report.skipped.push(SkippedSite {
            name,
            reason: "missing host".to_owned(),
        });
        return None;
    }
    if parsed.port == Some(0) {
        report.skipped.push(SkippedSite {
            name,
            reason: "invalid port".to_owned(),
        });
        return None;
    }
    let port = parsed.port.unwrap_or_else(|| protocol.default_port());
    let existing = vault.sites().iter().find(|s| {
        s.name == name
            && s.host == parsed.host
            && s.port == port
            && s.user == parsed.user
            && s.protocol == protocol
    });
    if let Some(existing) = existing {
        let id = existing.id.clone();
        report.skipped.push(SkippedSite {
            name,
            reason: "already in hoprs".to_owned(),
        });
        // Its folder's workspace may be missing (an earlier run failed).
        return Some(id);
    }

    let mut site = Site::new(name.clone(), parsed.host, parsed.user, protocol);
    site.port = port;
    site.remote_dir = parsed.remote_dir;
    site.local_dir = parsed.local_dir;
    if let Some(mode) = parsed.mode {
        site.ftp.mode = mode;
    }
    if protocol == Protocol::Ssh {
        site.ssh.key_file = parsed.key_file;
    }
    let id = site.id.clone();
    if let Err(err) = vault.upsert_site(site) {
        report.failures.push(format!("{name}: {}", err.message));
        return None;
    }

    let mut notes = parsed.notes;
    let mut secret_not_saved = false;
    if let ParsedPassword::Plain(password) = &parsed.password {
        // Once the keyring is unavailable, stop asking: each attempt could
        // raise another OS unlock prompt.
        if *keyring_down {
            notes.push("password not saved: keyring unavailable".to_owned());
            secret_not_saved = true;
        } else if let Err(err) = vault.set_secret(&id, SecretKind::Password, password) {
            *keyring_down = is_keyring_unavailable(&err);
            notes.push(format!("password not saved: {}", err.message));
            secret_not_saved = true;
        }
    }
    report.imported.push(ImportedSite {
        name,
        id: id.clone(),
        notes,
        secret_not_saved,
    });
    Some(id)
}

/// How much of the file is inspected before deleting it.
const SNIFF_BYTES: u64 = 4096;

/// Deletes the original `sitemanager.xml`, after checking it still is a
/// FileZilla file (starts with `<?xml`, mentions `<FileZilla3` in its first
/// 4 KiB). Symlinks and non-regular files are refused, never followed (on
/// unix the file is opened with `O_NOFOLLOW` and checked through the handle).
/// A window remains between that check and `remove_file`: the path could be
/// swapped in between. The caller only runs this on explicit user request.
pub fn delete_source(path: &Path) -> Result<()> {
    let refuse = |why: &str| Error::new(ErrorKind::Protocol, format!("not deleting: {why}"));
    // Friendly refusal for the common case; O_NOFOLLOW below closes the race.
    let meta = std::fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() {
        return Err(refuse("the path is a symlink"));
    }
    // Before opening: Windows refuses to open a directory (PermissionDenied) instead
    // of opening it as Unix does.
    if !meta.is_file() {
        return Err(refuse("not a regular file"));
    }
    let mut file = open_nofollow(path)?;
    if !file.metadata()?.is_file() {
        return Err(refuse("not a regular file"));
    }
    let mut head = Vec::new();
    (&mut file).take(SNIFF_BYTES).read_to_end(&mut head)?;
    drop(file);
    let text = head.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&head);
    let is_filezilla = text.starts_with(b"<?xml")
        && text
            .windows(b"<FileZilla3".len())
            .any(|w| w == b"<FileZilla3");
    if !is_filezilla {
        return Err(refuse("not a FileZilla site manager file"));
    }
    std::fs::remove_file(path)?;
    Ok(())
}

#[cfg(unix)]
fn open_nofollow(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
}

#[cfg(not(unix))]
fn open_nofollow(path: &Path) -> std::io::Result<std::fs::File> {
    std::fs::File::open(path)
}
