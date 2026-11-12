//! Writes a parsed FileZilla import into a [`Vault`].

use std::io::Read;
use std::path::Path;

use hoprs_core::{
    Error, ErrorKind, Protocol, Result, Site, SiteId, Workspace, WorkspaceId, WorkspaceSite,
};
use hoprs_vault::{SecretKind, Vault};

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
}

/// Applies `parsed` to the vault: sites, passwords (to the secret store) and,
/// optionally, folders as workspaces. Re-importing the same file skips every
/// site already present (same name, host, port, user and protocol) and every
/// workspace whose name already exists.
pub fn apply(
    vault: &mut Vault,
    parsed: ParsedSiteManager,
    options: &ImportOptions,
) -> Result<ImportReport> {
    let mut report = ImportReport::default();
    let mut folders: Vec<(String, Vec<SiteId>)> = Vec::new();
    let mut path = Vec::new();
    walk(
        vault,
        parsed.entries,
        options,
        &mut path,
        &mut report,
        &mut folders,
    )?;

    for (name, ids) in folders {
        if ids.is_empty() || vault.workspaces().iter().any(|w| w.name == name) {
            continue;
        }
        vault.upsert_workspace(Workspace {
            id: WorkspaceId::new(),
            name: name.clone(),
            sites: ids
                .into_iter()
                .map(|site| WorkspaceSite {
                    site,
                    tabs: Vec::new(),
                })
                .collect(),
        })?;
        report.workspaces_created.push(name);
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
) -> Result<()> {
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
                if let Some(id) = import_site(vault, site, prefix, report)?
                    && let Some(slot) = slot
                {
                    folders[slot].1.push(id);
                }
            }
            ParsedEntry::Folder { name, entries } => {
                path.push(name);
                let result = walk(vault, entries, options, path, report, folders);
                path.pop();
                result?;
            }
        }
    }
    Ok(())
}

fn import_site(
    vault: &mut Vault,
    parsed: ParsedSite,
    prefix: &[String],
    report: &mut ImportReport,
) -> Result<Option<SiteId>> {
    let name = if prefix.is_empty() {
        parsed.name.clone()
    } else {
        format!("{}/{}", prefix.join("/"), parsed.name)
    };
    let protocol = match parsed.protocol {
        Ok(protocol) => protocol,
        Err(reason) => {
            report.skipped.push(SkippedSite { name, reason });
            return Ok(None);
        }
    };
    let port = parsed.port.unwrap_or_else(|| protocol.default_port());
    let duplicate = vault.sites().iter().any(|s| {
        s.name == name
            && s.host == parsed.host
            && s.port == port
            && s.user == parsed.user
            && s.protocol == protocol
    });
    if duplicate {
        report.skipped.push(SkippedSite {
            name,
            reason: "already in hoprs".to_owned(),
        });
        return Ok(None);
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
    vault.upsert_site(site)?;

    let mut notes = parsed.notes;
    if let ParsedPassword::Plain(password) = &parsed.password
        && let Err(err) = vault.set_secret(&id, SecretKind::Password, password)
    {
        notes.push(format!("password not saved: {}", err.message));
    }
    report.imported.push(ImportedSite {
        name,
        id: id.clone(),
        notes,
    });
    Ok(Some(id))
}

/// How much of the file is inspected before deleting it.
const SNIFF_BYTES: u64 = 4096;

/// Deletes the original `sitemanager.xml`, after checking it still is a
/// FileZilla file (starts with `<?xml`, mentions `<FileZilla3` in its first
/// 4 KiB). Symlinks and non-regular files are refused, never followed.
pub fn delete_source(path: &Path) -> Result<()> {
    let refuse = |why: &str| Error::new(ErrorKind::Protocol, format!("not deleting: {why}"));
    let meta = std::fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() {
        return Err(refuse("the path is a symlink"));
    }
    if !meta.is_file() {
        return Err(refuse("not a regular file"));
    }
    let mut head = Vec::new();
    std::fs::File::open(path)?
        .take(SNIFF_BYTES)
        .read_to_end(&mut head)?;
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
