//! `sites.toml` and `workspaces.toml`: loading, validation and saving.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::Path;

use hoprs_core::{Error, ErrorKind, Result, Site, Workspace};
use serde::Serialize;
use serde::de::DeserializeOwned;
use toml::{Table, Value};

use crate::atomic::write_atomic;

pub(crate) const SITES_FILE: &str = "sites.toml";
pub(crate) const WORKSPACES_FILE: &str = "workspaces.toml";
const VERSION: i64 = 1;

/// A `[[site]]` or `[[workspace]]` entry that this version of hoprs cannot
/// understand (for example a protocol added by a newer version). It is kept
/// unchanged on disk and left out of the vault's lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnreadableEntry {
    /// `"sites.toml"` or `"workspaces.toml"`.
    pub file: &'static str,
    /// Position of the entry in the file's array when it was loaded.
    pub index: usize,
    /// The entry's `name`, if it has a readable one.
    pub name: Option<String>,
    /// Why the entry could not be read.
    pub reason: String,
}

/// An unreadable entry's original table, plus where to put it back.
#[derive(Clone, Debug)]
struct RawEntry {
    /// Id of the readable entry that preceded it, `None` if it came first.
    after: Option<String>,
    /// Raw `id`, if readable; still reserved against duplicates.
    id: Option<String>,
    table: Table,
}

/// What a file holds besides its readable entries.
#[derive(Clone, Debug, Default)]
struct FileRest {
    /// Unknown top-level keys.
    top: Table,
    unreadable: Vec<RawEntry>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Profiles {
    pub sites: Vec<Site>,
    pub workspaces: Vec<Workspace>,
    /// Unknown keys of each readable site, by site id.
    site_extras: HashMap<String, Table>,
    /// Unknown keys of each readable workspace, by workspace id.
    workspace_extras: HashMap<String, Table>,
    sites_rest: FileRest,
    workspaces_rest: FileRest,
    unreadable: Vec<UnreadableEntry>,
}

impl Profiles {
    pub fn unreadable(&self) -> &[UnreadableEntry] {
        &self.unreadable
    }

    /// Whether `id` is taken by an unreadable `[[site]]` entry.
    pub fn is_unreadable_site_id(&self, id: &str) -> bool {
        self.sites_rest.has_unreadable_id(id)
    }

    /// Whether `id` is taken by an unreadable `[[workspace]]` entry.
    pub fn is_unreadable_workspace_id(&self, id: &str) -> bool {
        self.workspaces_rest.has_unreadable_id(id)
    }

    pub fn forget_workspace_extras(&mut self, id: &str) {
        self.workspace_extras.remove(id);
    }
}

impl FileRest {
    fn has_unreadable_id(&self, id: &str) -> bool {
        self.unreadable.iter().any(|e| e.id.as_deref() == Some(id))
    }
}

/// How one profile file maps to its entry type.
struct Kind<T> {
    file: &'static str,
    /// Key of the array of tables: `site` or `workspace`.
    key: &'static str,
    id_of: fn(&T) -> &str,
}

const SITES: Kind<Site> = Kind {
    file: SITES_FILE,
    key: "site",
    id_of: |s| &s.id.0,
};

const WORKSPACES: Kind<Workspace> = Kind {
    file: WORKSPACES_FILE,
    key: "workspace",
    id_of: |w| &w.id.0,
};

/// One file's content, as loaded.
struct Loaded<T> {
    items: Vec<T>,
    extras: HashMap<String, Table>,
    rest: FileRest,
    unreadable: Vec<UnreadableEntry>,
}

impl<T> Default for Loaded<T> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            extras: HashMap::new(),
            rest: FileRest::default(),
            unreadable: Vec::new(),
        }
    }
}

pub(crate) fn load(dir: &Path) -> Result<Profiles> {
    let sites = load_file(dir, &SITES)?;
    let workspaces = load_file(dir, &WORKSPACES)?;
    let mut unreadable = sites.unreadable;
    unreadable.extend(workspaces.unreadable);
    Ok(Profiles {
        sites: sites.items,
        workspaces: workspaces.items,
        site_extras: sites.extras,
        workspace_extras: workspaces.extras,
        sites_rest: sites.rest,
        workspaces_rest: workspaces.rest,
        unreadable,
    })
}

pub(crate) fn save_sites(dir: &Path, profiles: &Profiles) -> Result<()> {
    save_file(
        dir,
        &SITES,
        &profiles.sites,
        &profiles.site_extras,
        &profiles.sites_rest,
    )
}

pub(crate) fn save_workspaces(dir: &Path, profiles: &Profiles) -> Result<()> {
    save_file(
        dir,
        &WORKSPACES,
        &profiles.workspaces,
        &profiles.workspace_extras,
        &profiles.workspaces_rest,
    )
}

fn cannot_read(file: &str, detail: impl Into<String>) -> Error {
    Error::new(ErrorKind::Protocol, format!("Cannot read {file}")).with_detail(detail)
}

fn load_file<T: DeserializeOwned + Serialize>(dir: &Path, kind: &Kind<T>) -> Result<Loaded<T>> {
    let file = kind.file;
    let text = match fs::read_to_string(dir.join(file)) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Loaded::default()),
        Err(err) if err.kind() == io::ErrorKind::InvalidData => {
            return Err(cannot_read(file, err.to_string()));
        }
        Err(err) => {
            let mut e = Error::from(err);
            e.detail = Some(std::mem::replace(
                &mut e.message,
                format!("Cannot read {file}"),
            ));
            return Err(e);
        }
    };
    let mut doc: Table = text
        .parse()
        .map_err(|e: toml::de::Error| cannot_read(file, e.to_string()))?;

    match doc.remove("version") {
        Some(Value::Integer(v)) if v == VERSION => {}
        Some(Value::Integer(v)) if v > VERSION => {
            return Err(
                Error::new(ErrorKind::Protocol, "Profile was written by a newer hoprs")
                    .with_detail(format!("{file} has version {v}")),
            );
        }
        Some(Value::Integer(v)) => {
            return Err(cannot_read(file, format!("unsupported version {v}")));
        }
        Some(_) => return Err(cannot_read(file, "`version` is not an integer")),
        None => return Err(cannot_read(file, "missing `version`")),
    }

    let raw_entries = match doc.remove(kind.key) {
        None => Vec::new(),
        Some(Value::Array(values)) => values
            .into_iter()
            .map(|v| match v {
                Value::Table(t) => Ok(t),
                _ => Err(()),
            })
            .collect::<std::result::Result<Vec<_>, ()>>()
            .map_err(|()| {
                cannot_read(file, format!("`{}` must be an array of tables", kind.key))
            })?,
        Some(_) => {
            return Err(cannot_read(
                file,
                format!("`{}` must be an array of tables", kind.key),
            ));
        }
    };

    let mut loaded = Loaded {
        rest: FileRest {
            top: doc,
            unreadable: Vec::new(),
        },
        ..Loaded::default()
    };
    let mut seen = HashSet::new();
    let mut previous: Option<String> = None;
    for (index, raw) in raw_entries.into_iter().enumerate() {
        match raw.clone().try_into::<T>() {
            Ok(item) => {
                let id = (kind.id_of)(&item).to_owned();
                let typed = Table::try_from(&item).map_err(|e| cannot_read(file, e.to_string()))?;
                let extras = unknown_keys(&raw, &typed);
                if !extras.is_empty() {
                    loaded.extras.insert(id.clone(), extras);
                }
                check_unique(&mut seen, file, &id)?;
                previous = Some(id);
                loaded.items.push(item);
            }
            Err(err) => {
                let id = raw.get("id").and_then(Value::as_str).map(str::to_owned);
                if let Some(id) = &id {
                    check_unique(&mut seen, file, id)?;
                }
                loaded.unreadable.push(UnreadableEntry {
                    file,
                    index,
                    name: raw.get("name").and_then(Value::as_str).map(str::to_owned),
                    reason: err.message().to_owned(),
                });
                loaded.rest.unreadable.push(RawEntry {
                    after: previous.clone(),
                    id,
                    table: raw,
                });
            }
        }
    }
    Ok(loaded)
}

fn check_unique(seen: &mut HashSet<String>, file: &str, id: &str) -> Result<()> {
    if seen.insert(id.to_owned()) {
        Ok(())
    } else {
        Err(Error::new(ErrorKind::Protocol, format!("Duplicate id in {file}")).with_detail(id))
    }
}

fn save_file<T: Serialize>(
    dir: &Path,
    kind: &Kind<T>,
    items: &[T],
    extras: &HashMap<String, Table>,
    rest: &FileRest,
) -> Result<()> {
    let file = kind.file;
    let cannot_save = |detail: String| {
        Error::new(ErrorKind::Protocol, format!("Cannot save {file}")).with_detail(detail)
    };
    let raw_after = |anchor: Option<&str>| -> Vec<Value> {
        rest.unreadable
            .iter()
            .filter(|e| e.after.as_deref() == anchor)
            .map(|e| Value::Table(e.table.clone()))
            .collect()
    };

    let mut entries = raw_after(None);
    for item in items {
        let id = (kind.id_of)(item);
        let mut table =
            Table::try_from(item).map_err(|e| cannot_save(format!("{} {id}: {e}", kind.key)))?;
        if let Some(extra) = extras.get(id) {
            merge_unknown(&mut table, extra);
        }
        entries.push(Value::Table(table));
        entries.extend(raw_after(Some(id)));
    }
    // Unreadable entries whose anchor no longer exists go last.
    let anchors: HashSet<&str> = items.iter().map(|i| (kind.id_of)(i)).collect();
    entries.extend(
        rest.unreadable
            .iter()
            .filter(|e| e.after.as_deref().is_some_and(|a| !anchors.contains(a)))
            .map(|e| Value::Table(e.table.clone())),
    );

    let mut doc = Table::new();
    doc.insert("version".into(), Value::Integer(VERSION));
    for (k, v) in &rest.top {
        doc.insert(k.clone(), v.clone());
    }
    if !entries.is_empty() {
        doc.insert(kind.key.into(), Value::Array(entries));
    }
    let text = toml::to_string(&doc).map_err(|e| cannot_save(e.to_string()))?;
    write_atomic(&dir.join(file), text.as_bytes())
}

/// Keys of `raw` that the typed entry does not produce, recursing into
/// sub-tables present on both sides.
fn unknown_keys(raw: &Table, typed: &Table) -> Table {
    let mut out = Table::new();
    for (key, value) in raw {
        match (value, typed.get(key)) {
            (_, None) => {
                out.insert(key.clone(), value.clone());
            }
            (Value::Table(r), Some(Value::Table(t))) => {
                let nested = unknown_keys(r, t);
                if !nested.is_empty() {
                    out.insert(key.clone(), Value::Table(nested));
                }
            }
            _ => {}
        }
    }
    out
}

/// Puts `extras` back into `typed`; typed values win on conflict.
fn merge_unknown(typed: &mut Table, extras: &Table) {
    for (key, value) in extras {
        match (typed.get_mut(key), value) {
            (None, _) => {
                typed.insert(key.clone(), value.clone());
            }
            (Some(Value::Table(t)), Value::Table(e)) => merge_unknown(t, e),
            _ => {}
        }
    }
}
