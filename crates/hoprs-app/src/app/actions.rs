//! File actions in a pane: new folder, rename (both edited inline), delete (after an
//! in-place confirmation listing the names) and, on the remote side, permissions.
//!
//! An action runs as one [`Effect::FileOp`] tagged with an id. The pane remembers the
//! operation in flight ([`Busy`]) and takes no other action until it is done; the result
//! comes back as [`Message::FileOpDone`]. A result for a closed tab, or one that is not
//! the pane's operation, is dropped. A failure shows inline above the table (never a
//! modal); either way the folder is listed again if the pane is still in it.

use hoprs_core::{EntryKind, Error, RemoteEntry, RemotePath};

use super::files::{Pane, PaneStatus, RequestId, Row, Selection};
use super::tabs::{SiteConn, Tab, TabId};
use super::{App, Effect};
use crate::backend::{DeleteTarget, Side};
use crate::ui::strings;

/// The inline editor, the confirmation or the dialog a pane shows; one at a time.
#[derive(Clone, Debug, PartialEq)]
pub enum PaneAction {
    Edit(Edit),
    /// "Delete these?", with the entries as they were listed.
    ConfirmDelete(Vec<RemoteEntry>),
    Permissions(Permissions),
}

/// A name being typed in a row.
#[derive(Clone, Debug, PartialEq)]
pub struct Edit {
    pub kind: EditKind,
    pub value: String,
    /// Why the name was refused, shown inline.
    pub error: Option<&'static str>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditKind {
    /// A row above the list for the new folder's name.
    NewFolder,
    /// The name cell of the entry called `from`.
    Rename { from: String },
}

/// The permissions dialog: the rwx grid and the octal value, kept in step.
#[derive(Clone, Debug, PartialEq)]
pub struct Permissions {
    /// What the mode applies to.
    pub names: Vec<String>,
    /// Permission bits and setuid/setgid/sticky (`0o7777` at most).
    pub mode: u32,
    /// The octal field as typed.
    pub octal: String,
    /// The octal field holds no valid mode; Apply is off.
    pub invalid: bool,
    /// The entries had different modes; the dialog starts from the first one's.
    pub mixed: bool,
}

/// The nine rwx bits, owner first, as the grid shows them.
pub const PERMISSION_BITS: [[u32; 3]; 3] = [
    [0o400, 0o200, 0o100],
    [0o040, 0o020, 0o010],
    [0o004, 0o002, 0o001],
];

impl Permissions {
    pub fn new(names: Vec<String>, mode: u32) -> Self {
        let mode = mode & 0o7777;
        Self {
            names,
            mode,
            octal: octal(mode),
            invalid: false,
            mixed: false,
        }
    }

    pub fn has(&self, bit: u32) -> bool {
        self.mode & bit != 0
    }

    /// A grid box: flips one rwx bit; the octal field follows.
    pub fn toggle(&mut self, bit: u32) {
        self.mode ^= bit & 0o777;
        self.octal = octal(self.mode);
        self.invalid = false;
    }

    /// The octal field: three or four octal digits set the mode (the grid follows);
    /// anything else is kept as typed and marked invalid.
    pub fn set_octal(&mut self, text: String) {
        match parse_octal(&text) {
            Some(mode) => {
                self.mode = mode;
                self.invalid = false;
            }
            None => self.invalid = true,
        }
        self.octal = text;
    }
}

/// `755`, or `4755` when a setuid/setgid/sticky bit is set.
pub fn octal(mode: u32) -> String {
    if mode & 0o7000 != 0 {
        format!("{:04o}", mode & 0o7777)
    } else {
        format!("{:03o}", mode & 0o777)
    }
}

/// Three or four octal digits.
pub fn parse_octal(text: &str) -> Option<u32> {
    let text = text.trim();
    if !(3..=4).contains(&text.len()) || !text.bytes().all(|b| (b'0'..=b'7').contains(&b)) {
        return None;
    }
    u32::from_str_radix(text, 8).ok()
}

/// An operation in flight in a pane.
#[derive(Clone, Debug, PartialEq)]
pub struct Busy {
    pub op: RequestId,
    pub kind: BusyKind,
    /// Items removed so far (delete).
    pub done: u64,
    /// The folder the pane was in when it started.
    pub folder: RemotePath,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum BusyKind {
    NewFolder,
    Rename,
    Delete,
    Permissions,
}

/// What an [`Effect::FileOp`] does.
#[derive(Clone, Debug, PartialEq)]
pub enum FileAction {
    Mkdir(RemotePath),
    Rename {
        from: RemotePath,
        to: RemotePath,
    },
    Delete(Vec<DeleteTarget>),
    /// The same mode on each path, in order.
    Chmod {
        paths: Vec<RemotePath>,
        mode: u32,
    },
}

impl FileAction {
    fn busy_kind(&self) -> BusyKind {
        match self {
            FileAction::Mkdir(_) => BusyKind::NewFolder,
            FileAction::Rename { .. } => BusyKind::Rename,
            FileAction::Delete(_) => BusyKind::Delete,
            FileAction::Chmod { .. } => BusyKind::Permissions,
        }
    }
}

/// Whether `name` can be a new name in a folder holding `entries` (`except` is the entry
/// being renamed).
///
/// The listing may be stale, so the backend checks again before a rename (see
/// [`crate::backend::ops::rename_no_replace`]).
pub fn check_name(
    name: &str,
    entries: &[RemoteEntry],
    except: Option<&str>,
    side: Side,
) -> Result<(), &'static str> {
    if name.trim().is_empty() {
        return Err(strings::NAME_EMPTY);
    }
    if name.trim() != name {
        return Err(strings::NAME_SPACES);
    }
    if name == "." || name == ".." || name.contains('/') || name.contains('\0') {
        return Err(strings::NAME_INVALID);
    }
    // On Windows the local disk takes `\` as a separator too.
    if cfg!(windows) && side == Side::Local && name.contains('\\') {
        return Err(strings::NAME_INVALID_WINDOWS);
    }
    if except != Some(name) && entries.iter().any(|e| e.name == name) {
        return Err(strings::NAME_TAKEN);
    }
    Ok(())
}

/// "New folder", or "New folder 2", 3… if taken.
fn new_folder_name(entries: &[RemoteEntry]) -> String {
    let taken = |name: &str| entries.iter().any(|e| e.name == name);
    let mut name = strings::NEW_FOLDER_NAME.to_owned();
    let mut n = 1;
    while taken(&name) {
        n += 1;
        name = strings::numbered(strings::NEW_FOLDER_NAME, n);
    }
    name
}

/// How much of a name the editor preselects: the stem of a file (`report` of
/// `report.pdf`), all of a folder's name. In characters.
fn preselect(name: &str, is_dir: bool) -> usize {
    let end = match name.rfind('.') {
        Some(dot) if dot > 0 && !is_dir => dot,
        _ => name.len(),
    };
    name[..end].chars().count()
}

impl Pane {
    /// The selected entries (`..` aside), in row order: what Rename, Delete and
    /// Permissions act on.
    pub fn selected_entries(&self) -> Vec<&RemoteEntry> {
        self.selection
            .rows
            .iter()
            .filter_map(|&row| match self.row(row) {
                Some(Row::Entry(entry)) => Some(entry),
                Some(Row::Up) | None => None,
            })
            .collect()
    }

    /// What Permissions… acts on: the selected entries but links. A chmod through a link
    /// (SFTP setstat, `set_permissions`, FTP `SITE CHMOD`) changes its target, and the
    /// link's own `lrwxrwxrwx` would start the dialog at 777.
    pub fn permission_targets(&self) -> Vec<&RemoteEntry> {
        self.selected_entries()
            .into_iter()
            .filter(|e| e.kind != EntryKind::Symlink)
            .collect()
    }
}

impl Tab {
    /// Whether a pane can start an action now, given its site's connection `conn`: its
    /// folder is listed (and, remote, it has a session) and no other operation runs in it.
    pub fn can_act(&self, conn: &SiteConn, side: Side) -> bool {
        matches!(
            self.pane_status(conn, side),
            PaneStatus::Full | PaneStatus::Empty
        ) && self.pane(side).busy.is_none()
            && (side == Side::Local || conn.session().is_some())
    }

    /// Whether Upload (local) / Download (remote) can send the selection of `side` to
    /// the other pane: both panes listed, connected, and something selected.
    pub fn can_transfer(&self, conn: &SiteConn, side: Side) -> bool {
        self.can_act(conn, Side::Local)
            && self.can_act(conn, Side::Remote)
            && !self.pane(side).selected_entries().is_empty()
    }
}

impl App {
    pub(super) fn can_act(&self, index: usize, side: Side) -> bool {
        self.tabs[index].can_act(self.tab_conn(index), side)
    }

    fn selected_entries(&self, index: usize, side: Side) -> Vec<&RemoteEntry> {
        self.tabs[index].pane(side).selected_entries()
    }

    /// F7 / "New folder": the inline name row, its name preselected.
    pub(super) fn start_new_folder(&mut self, index: usize, side: Side) -> Vec<Effect> {
        if !self.can_act(index, side) {
            return Vec::new();
        }
        let tab = &mut self.tabs[index];
        let tab_id = tab.id;
        let pane = tab.pane_mut(side);
        let value = new_folder_name(&pane.entries);
        let select_end = preselect(&value, true);
        pane.action = Some(PaneAction::Edit(Edit {
            kind: EditKind::NewFolder,
            value,
            error: None,
        }));
        vec![Effect::FocusEdit {
            tab: tab_id,
            side,
            select_end,
        }]
    }

    /// F2 / "Rename": the cursor's entry if it is selected, else the first selected one.
    pub(super) fn start_rename(&mut self, index: usize, side: Side) -> Vec<Effect> {
        if !self.can_act(index, side) {
            return Vec::new();
        }
        let pane = self.tabs[index].pane(side);
        let cursor = pane
            .selection
            .cursor
            .filter(|&c| pane.selection.contains(c))
            .and_then(|c| match pane.row(c) {
                Some(Row::Entry(entry)) => Some(entry),
                _ => None,
            });
        let Some(entry) = cursor.or_else(|| self.selected_entries(index, side).first().copied())
        else {
            return Vec::new();
        };
        let (name, is_dir) = (entry.name.clone(), entry.kind == EntryKind::Dir);
        let select_end = preselect(&name, is_dir);
        let tab = &mut self.tabs[index];
        let tab_id = tab.id;
        let pane = tab.pane_mut(side);
        pane.action = Some(PaneAction::Edit(Edit {
            kind: EditKind::Rename { from: name.clone() },
            value: name.clone(),
            error: None,
        }));
        // The editor lives in the entry's row: bring it into view, or it would take the
        // pane's keys from off screen.
        let mut effects = Vec::new();
        let row = usize::from(pane.has_up())
            + pane
                .entries
                .iter()
                .position(|e| e.name == name)
                .unwrap_or(0);
        if let Some(offset_y) = pane.scroll_to_show(row) {
            pane.offset_y = offset_y;
            effects.push(Effect::ScrollPane {
                tab: tab_id,
                side,
                offset_y,
            });
        }
        effects.push(Effect::FocusEdit {
            tab: tab_id,
            side,
            select_end,
        });
        effects
    }

    /// Del / "Delete": asks first, listing the names.
    pub(super) fn start_delete(&mut self, index: usize, side: Side) -> Vec<Effect> {
        if !self.can_act(index, side) {
            return Vec::new();
        }
        let targets: Vec<RemoteEntry> = self
            .selected_entries(index, side)
            .into_iter()
            .cloned()
            .collect();
        if !targets.is_empty() {
            self.tabs[index].pane_mut(side).action = Some(PaneAction::ConfirmDelete(targets));
        }
        Vec::new()
    }

    /// "Permissions…" (remote only): the dialog, from the first selected entry's mode.
    pub(super) fn start_permissions(&mut self, index: usize, side: Side) -> Vec<Effect> {
        if side != Side::Remote || !self.can_act(index, side) {
            return Vec::new();
        }
        let selected = self.tabs[index].pane(side).permission_targets();
        let Some(first) = selected.first() else {
            return Vec::new();
        };
        let mode_of = |e: &RemoteEntry| {
            let default = if e.kind == EntryKind::Dir {
                0o755
            } else {
                0o644
            };
            e.mode.unwrap_or(default) & 0o7777
        };
        let mode = mode_of(first);
        let mixed = selected.iter().any(|e| mode_of(e) != mode);
        let names = selected.iter().map(|e| e.name.clone()).collect();
        let mut dialog = Permissions::new(names, mode);
        dialog.mixed = mixed;
        self.tabs[index].pane_mut(side).action = Some(PaneAction::Permissions(dialog));
        Vec::new()
    }

    /// Enter in the inline editor: checks the name, then creates or renames.
    pub(super) fn commit_edit(&mut self, index: usize, side: Side) -> Vec<Effect> {
        let pane = self.tabs[index].pane_mut(side);
        let Some(PaneAction::Edit(edit)) = &mut pane.action else {
            return Vec::new();
        };
        let except = match &edit.kind {
            EditKind::Rename { from } => Some(from.as_str()),
            EditKind::NewFolder => None,
        };
        if let Err(error) = check_name(&edit.value, &pane.entries, except, side) {
            edit.error = Some(error);
            return Vec::new();
        }
        let name = edit.value.clone();
        let action = match &edit.kind {
            EditKind::NewFolder => FileAction::Mkdir(pane.path.join(&name)),
            // Same name: nothing to do.
            EditKind::Rename { from } if *from == name => {
                pane.action = None;
                return Vec::new();
            }
            EditKind::Rename { from } => FileAction::Rename {
                from: pane.path.join(from),
                to: pane.path.join(&name),
            },
        };
        pane.action = None;
        pane.select_after = Some(name);
        self.start_op(index, side, action)
    }

    /// "Delete" in the confirmation.
    pub(super) fn confirm_delete(&mut self, index: usize, side: Side) -> Vec<Effect> {
        let pane = self.tabs[index].pane_mut(side);
        let Some(PaneAction::ConfirmDelete(entries)) = pane.action.take() else {
            return Vec::new();
        };
        let targets = entries
            .iter()
            .map(|e| DeleteTarget {
                path: pane.path.join(&e.name),
                kind: e.kind,
            })
            .collect();
        self.start_op(index, side, FileAction::Delete(targets))
    }

    /// "Apply" in the permissions dialog.
    pub(super) fn apply_permissions(&mut self, index: usize, side: Side) -> Vec<Effect> {
        let pane = self.tabs[index].pane_mut(side);
        let Some(PaneAction::Permissions(dialog)) = &pane.action else {
            return Vec::new();
        };
        if dialog.invalid {
            return Vec::new();
        }
        let paths = dialog.names.iter().map(|n| pane.path.join(n)).collect();
        let mode = dialog.mode;
        pane.action = None;
        self.start_op(index, side, FileAction::Chmod { paths, mode })
    }

    /// Starts `action` in the pane: marks it busy and hands the operation to the effects.
    fn start_op(&mut self, index: usize, side: Side, action: FileAction) -> Vec<Effect> {
        let op = RequestId(self.next_id());
        let session = self.tab_session(index);
        let tab = &mut self.tabs[index];
        let tab_id = tab.id;
        let pane = tab.pane_mut(side);
        pane.action_error = None;
        pane.busy = Some(Busy {
            op,
            kind: action.busy_kind(),
            done: 0,
            folder: pane.path.clone(),
        });
        vec![Effect::FileOp {
            tab: tab_id,
            side,
            op,
            session: match side {
                Side::Local => None,
                Side::Remote => session,
            },
            action,
        }]
    }

    /// A delete's running count.
    pub(super) fn file_op_progress(
        &mut self,
        tab: TabId,
        side: Side,
        op: RequestId,
        done: u64,
    ) -> Vec<Effect> {
        if let Some(busy) = self
            .tab_index(tab)
            .and_then(|i| self.tabs[i].pane_mut(side).busy.as_mut())
            .filter(|busy| busy.op == op)
        {
            busy.done = busy.done.max(done);
        }
        Vec::new()
    }

    /// An operation finished. Dropped for a closed tab or an operation the pane no longer
    /// waits for. A failure shows inline; the folder is listed again if the pane is still
    /// in it (a partial delete then shows what is left).
    pub(super) fn file_op_done(
        &mut self,
        tab: TabId,
        side: Side,
        op: RequestId,
        result: Result<(), Error>,
    ) -> Vec<Effect> {
        let Some(index) = self.tab_index(tab) else {
            return Vec::new();
        };
        let pane = self.tabs[index].pane_mut(side);
        let Some(busy) = pane.busy.take_if(|busy| busy.op == op) else {
            return Vec::new();
        };
        if let Err(mut error) = result {
            // A delete that stopped half way: say how far it got.
            if busy.kind == BusyKind::Delete && busy.done > 0 {
                error.message = strings::deleted_then(busy.done, &error.message);
            }
            pane.action_error = Some(error);
            pane.select_after = None;
        }
        if busy.folder != pane.path {
            // The user went elsewhere meanwhile: that folder is not on screen.
            pane.select_after = None;
            return Vec::new();
        }
        self.request_listing(index, side)
    }

    /// Escape, Cancel or Keep: closes the editor, confirmation or dialog.
    pub(super) fn cancel_action(&mut self, index: usize, side: Side) {
        self.tabs[index].pane_mut(side).action = None;
    }

    /// After a listing: selects the entry an action just created or renamed, and returns
    /// the scroll that brings it into view.
    pub(super) fn select_new_entry(&mut self, index: usize, side: Side) -> Vec<Effect> {
        let tab = &mut self.tabs[index];
        let tab_id = tab.id;
        let pane = tab.pane_mut(side);
        let Some(name) = pane.select_after.take() else {
            return Vec::new();
        };
        let up = usize::from(pane.has_up());
        let Some(row) = pane
            .entries
            .iter()
            .position(|e| e.name == name)
            .map(|i| i + up)
        else {
            return Vec::new();
        };
        pane.selection = Selection::default();
        pane.selection.move_to(row, false);
        match pane.scroll_to_show(row) {
            Some(offset_y) => {
                pane.offset_y = offset_y;
                vec![Effect::ScrollPane {
                    tab: tab_id,
                    side,
                    offset_y,
                }]
            }
            None => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn octal_formats_and_parses() {
        assert_eq!(octal(0o755), "755");
        assert_eq!(octal(0o040644), "644", "type bits are not permissions");
        assert_eq!(octal(0o4755), "4755");
        assert_eq!(parse_octal("640"), Some(0o640));
        assert_eq!(parse_octal(" 0755 "), Some(0o755));
        assert_eq!(parse_octal("2775"), Some(0o2775));
        for bad in ["", "75", "789", "12345", "abc", "-75"] {
            assert_eq!(parse_octal(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn names_are_checked() {
        let entry = |name: &str| RemoteEntry {
            name: name.into(),
            path: RemotePath::root().join(name),
            kind: EntryKind::File,
            size: 0,
            modified: None,
            mode: None,
        };
        let entries = [entry("a.txt"), entry("New folder")];
        assert_eq!(check_name("b.txt", &entries, None, Side::Remote), Ok(()));
        assert_eq!(
            check_name("  ", &entries, None, Side::Remote),
            Err(strings::NAME_EMPTY)
        );
        for bad in [".", "..", "a/b"] {
            assert_eq!(
                check_name(bad, &entries, None, Side::Remote),
                Err(strings::NAME_INVALID),
                "{bad:?}"
            );
        }
        assert_eq!(
            check_name("a.txt", &entries, None, Side::Remote),
            Err(strings::NAME_TAKEN)
        );
        assert_eq!(
            check_name("a.txt", &entries, Some("a.txt"), Side::Remote),
            Ok(())
        );
        for padded in [" b.txt", "b.txt ", "b.txt\t"] {
            assert_eq!(
                check_name(padded, &entries, None, Side::Local),
                Err(strings::NAME_SPACES),
                "{padded:?}"
            );
        }
        // `\` is a separator only on a Windows local disk.
        assert_eq!(check_name("a\\b", &entries, None, Side::Remote), Ok(()));
        let local = check_name("a\\b", &entries, None, Side::Local);
        if cfg!(windows) {
            assert_eq!(local, Err(strings::NAME_INVALID_WINDOWS));
        } else {
            assert_eq!(local, Ok(()));
        }
        assert_eq!(new_folder_name(&entries), "New folder 2");
        assert_eq!(new_folder_name(&[]), "New folder");
        assert_eq!(preselect("report.pdf", false), 6);
        assert_eq!(preselect(".bashrc", false), 7);
        assert_eq!(preselect("site.d", true), 6);
        assert_eq!(preselect("café.txt", false), 4, "in characters");
    }
}
