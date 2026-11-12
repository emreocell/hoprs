//! Every user-visible string, in one place (i18n-ready). Sentence case.

pub const APP_TITLE: &str = "hoprs";
pub const ALREADY_RUNNING: &str = "hoprs is already running";
pub const CANNOT_OPEN_PROFILES: &str = "Cannot open the saved sites";

// Top bar.
pub const BRAND: &str = "hoprs";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const NO_SESSIONS: &str = "No open sessions";
pub const GO_TO_ANYTHING: &str = "Go to anything";
pub const GO_TO_ANYTHING_KEY: &str = "Ctrl K";

// Sidebar.
pub const FILTER_SITES: &str = "Filter sites";
pub const WORKSPACES: &str = "Workspaces";
pub const ALL_SITES: &str = "All sites";
pub const NEW: &str = "+ New";
pub const NO_WORKSPACES: &str = "No workspaces yet";
pub const NO_SITES: &str = "No sites yet";
pub const IMPORT_FILEZILLA: &str = "Import from FileZilla";
pub const SETTINGS: &str = "Settings";
pub const NEW_SITE: &str = "New site";
pub const NO_MATCHES: &str = "No matching sites";
pub const OPEN_WORKSPACE: &str = "Open workspace";
pub const EDIT: &str = "Edit";
pub const SITE_MISSING: &str = "Site missing";
pub const CANNOT_LOAD_SITES: &str = "Can't load the saved sites";
pub const RETRY: &str = "Retry";
pub const UNREADABLE_ENTRY: &str = "Can't read this entry (written by a newer hoprs?)";
pub const OPEN_PROFILE_FOLDER: &str = "Open sites.toml folder";
/// Opening the profile folder failed.
pub fn cannot_open_folder(reason: &str) -> String {
    format!("Can't open the folder: {reason}")
}

/// "1 site", "3 sites".
pub fn site_count(n: usize) -> String {
    if n == 1 {
        "1 site".to_owned()
    } else {
        format!("{n} sites")
    }
}

/// A workspace's count: "3 sites", or "2 of 3 sites" when some are missing.
pub fn workspace_site_count(present: usize, total: usize) -> String {
    if present == total {
        site_count(total)
    } else {
        format!("{present} of {}", site_count(total))
    }
}

/// An unreadable entry: its name, or where it is.
pub fn unreadable_name(name: Option<&str>, file: &str, index: usize) -> String {
    match name {
        Some(name) => name.to_owned(),
        None => format!("Entry {} in {file}", index + 1),
    }
}

// Site form.
pub const SITE_FORM_NEW: &str = "New site";
pub const SITE_FORM_EDIT: &str = "Edit site";
pub const FIELD_NAME: &str = "Name";
pub const FIELD_NAME_HINT: &str = "Defaults to the host";
pub const FIELD_HOST: &str = "Host";
pub const FIELD_PORT: &str = "Port";
pub const FIELD_USER: &str = "User";
pub const FIELD_PROTOCOL: &str = "Protocol";
pub const FIELD_PASSWORD: &str = "Password";
pub const PASSWORD_OPTIONAL: &str = "Optional";
pub const PASSWORD_REPLACE: &str = "Type to replace";
pub const PASSWORD_SAVED: &str = "Saved in the system keyring";
pub const FORGET_PASSWORD: &str = "Forget the saved password";
pub const SSH_OPTIONS: &str = "SSH";
pub const USE_AGENT: &str = "Use ssh-agent";
pub const KEY_FILE: &str = "Key file";
pub const KEY_FILE_HINT: &str = "Path to a private key (optional)";
pub const OPEN_AS: &str = "Open as";
pub const OPEN_AS_TERMINAL: &str = "Terminal";
pub const OPEN_AS_FILES: &str = "Files";
pub const OPEN_AS_BOTH: &str = "Terminal and files";
pub const TERMINAL_LATER: &str = "Terminal tabs arrive in a later version";
pub const FTP_OPTIONS: &str = "FTP";
pub const FTP_MODE: &str = "Transfer mode";
pub const FTP_PASSIVE: &str = "Passive";
pub const FTP_ACTIVE: &str = "Active";
pub const NAT_WORKAROUND: &str = "Ignore the passive address the server sends (NAT)";
pub const START_FOLDERS: &str = "Start folders";
pub const REMOTE_DIR: &str = "Remote folder";
pub const LOCAL_DIR: &str = "Local folder";
pub const FOLDER_HINT: &str = "Optional";
pub const HOST_REQUIRED: &str = "Host is required";
pub const PORT_INVALID: &str = "Port must be a number from 1 to 65535";
pub const SAVE: &str = "Save";
pub const SAVING: &str = "Saving…";
pub const CANCEL: &str = "Cancel";
pub const DELETE: &str = "Delete";
pub const KEEP: &str = "Keep";
pub const DELETE_SITE_CONFIRM: &str =
    "Delete this site? It leaves every workspace and its saved password is deleted.";

// Workspace form.
pub const WORKSPACE_FORM_NEW: &str = "New workspace";
pub const WORKSPACE_FORM_EDIT: &str = "Edit workspace";
pub const WORKSPACE_SITES: &str = "Sites";
pub const WORKSPACE_NO_SITES: &str = "No sites yet. Create a site first.";
pub const NAME_REQUIRED: &str = "Name is required";
pub const DELETE_WORKSPACE_CONFIRM: &str = "Delete this workspace? Its sites are kept.";

// Center.
pub const NO_SESSION_TITLE: &str = "No session open";
pub const NO_SESSION_HINT: &str = "Pick a site in the sidebar to connect.";
pub const KEYRING_UNAVAILABLE: &str =
    "The system keyring was unavailable, so the saved password was not used.";
pub const EDIT_SITE: &str = "Edit site";

// Tabs.
pub const CLOSE_TAB: &str = "×";

// File panes.
pub const LOCAL: &str = "Local";
pub const COL_NAME: &str = "Name";
pub const COL_SIZE: &str = "Size";
pub const COL_MODIFIED: &str = "Modified";
pub const COL_PERMISSIONS: &str = "Permissions";
pub const SORT_ASCENDING: &str = "↑";
pub const SORT_DESCENDING: &str = "↓";
/// The row that goes up one folder.
pub const PARENT_DIR: &str = "..";
/// After a folder name, and between breadcrumb segments.
pub const PATH_SEPARATOR: &str = "/";
/// The breadcrumb's first segment for the home folder.
pub const HOME_DIR: &str = "~";
/// The size column of a folder.
pub const NO_SIZE: &str = "-";
pub const EMPTY_FOLDER: &str = "This folder is empty";
pub const RECONNECT: &str = "Reconnect";
/// Units of [`crate::ui::format::size`], base 1024.
pub const SIZE_UNITS: [&str; 7] = ["B", "KB", "MB", "GB", "TB", "PB", "EB"];
/// Short month names for file dates.
pub const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

// Queue.
pub const TRANSFERS: &str = "Transfers";
pub const QUEUE_ACTIVE: &str = "Active";
pub const QUEUE_QUEUED: &str = "Queued";
pub const QUEUE_FAILED: &str = "Failed";
pub const QUEUE_DONE: &str = "Done";
pub const QUEUE_HIDE: &str = "Hide";
pub const QUEUE_SHOW: &str = "Show";
pub const NO_TRANSFERS: &str = "No transfers";
pub const NO_TRANSFERS_HINT: &str =
    "Select files and press Upload or Download, drag them to the other pane, or drop them here.";
pub const QUEUE_LOADING: &str = "Loading transfers…";
pub const NOTHING_ACTIVE: &str = "Nothing running";
pub const NOTHING_QUEUED: &str = "Nothing waiting";
pub const NOTHING_FAILED: &str = "No failures";
pub const NOTHING_DONE: &str = "Nothing finished yet";
pub const CLEAR_FINISHED: &str = "Clear finished";
pub const UPLOAD_ARROW: &str = "↑";
pub const DOWNLOAD_ARROW: &str = "↓";
pub const WAITING_FOR_WORKER: &str = "Waiting for a worker";
pub const STARTING: &str = "Starting…";
pub const PAUSED: &str = "Paused";
pub const CANCELLED: &str = "Cancelled";
pub const TRANSFER_DONE: &str = "Done";
pub const SKIPPED: &str = "Skipped";
pub const FILE_EXISTS: &str = "File exists —";
pub const OVERWRITE: &str = "Overwrite";
pub const RESUME: &str = "Resume";
pub const SKIP: &str = "Skip";
pub const PAUSE: &str = "Pause";
pub const APPLY_TO_ALL: &str = "Apply to all";
pub const SITE_PAUSED: &str = "Paused — connection lost";
pub const DROP_NEEDS_SITE: &str = "Open a connected site to drop files into it";
pub const DROP_UNSUPPORTED: &str = "Can't upload this path";

/// A queue tab: "Active 3".
pub fn queue_tab(label: &str, count: usize) -> String {
    format!("{label} {count}")
}

/// "41.7 MB/s".
pub fn speed(bytes_per_sec: u64) -> String {
    format!("{}/s", crate::ui::format::size(bytes_per_sec))
}

/// After the total speed: " total, 4 workers".
pub fn total_workers(workers: usize) -> String {
    if workers == 1 {
        " total, 1 worker".to_owned()
    } else {
        format!(" total, {workers} workers")
    }
}

/// "51%".
pub fn percent(fraction: f32) -> String {
    format!("{:.0}%", (fraction * 100.0).floor())
}

/// "Skipped: exists".
pub fn skipped(reason: &str) -> String {
    format!("{SKIPPED}: {reason}")
}

// Status bar.
pub const NOT_CONNECTED: &str = "Not connected";

// File actions.
pub const NEW_FOLDER: &str = "New folder";
pub const RENAME: &str = "Rename";
pub const PERMISSIONS: &str = "Permissions…";
pub const KEY_NEW_FOLDER: &str = "F7";
pub const KEY_RENAME: &str = "F2";
pub const KEY_DELETE: &str = "Del";
pub const UPLOAD: &str = "Upload";
pub const DOWNLOAD: &str = "Download";
pub const KEY_TRANSFER: &str = "Ctrl ↵";
/// The name a new folder starts with.
pub const NEW_FOLDER_NAME: &str = "New folder";
pub const NAME_EMPTY: &str = "The name can't be empty";
pub const NAME_INVALID: &str = "The name can't contain / or be . or ..";
pub const NAME_INVALID_WINDOWS: &str = "The name can't contain \\ on this disk";
pub const NAME_SPACES: &str = "The name can't start or end with a space";
pub const MIXED_PERMISSIONS: &str = "Mixed permissions: showing the first item's";
pub const NAME_TAKEN: &str = "Something with this name is already here";
pub const DISMISS: &str = "Dismiss";
pub const CREATING_FOLDER: &str = "Creating folder…";
pub const RENAMING: &str = "Renaming…";
pub const CHANGING_PERMISSIONS: &str = "Changing permissions…";
pub const DELETE_FOLDERS_NOTE: &str = "Folders are deleted with everything in them.";
pub const PERMISSIONS_TITLE: &str = "Permissions";
pub const PERM_OWNER: &str = "Owner";
pub const PERM_GROUP: &str = "Group";
pub const PERM_OTHERS: &str = "Others";
pub const PERM_READ: &str = "Read";
pub const PERM_WRITE: &str = "Write";
pub const PERM_EXECUTE: &str = "Execute";
pub const PERM_OCTAL: &str = "Octal";
pub const OCTAL_INVALID: &str = "Three or four digits from 0 to 7";
pub const APPLY: &str = "Apply";
/// How many names a confirmation lists before "and N more".
pub const LISTED_NAMES: usize = 5;

/// "New folder 2".
pub fn numbered(name: &str, n: u32) -> String {
    format!("{name} {n}")
}

/// "Delete report.pdf?", "Delete 3 items?".
pub fn delete_question(names: &[&str]) -> String {
    match names {
        [one] => format!("Delete {one}?"),
        many => format!("Delete {} items?", many.len()),
    }
}

/// "and 4 more".
pub fn and_more(n: usize) -> String {
    format!("and {n} more")
}

/// "Deleting… 12 items".
pub fn deleting(done: u64) -> String {
    match done {
        0 => "Deleting…".to_owned(),
        1 => "Deleting… 1 item".to_owned(),
        n => format!("Deleting… {n} items"),
    }
}

/// A delete that stopped after removing some items.
pub fn deleted_then(done: u64, error: &str) -> String {
    if done == 1 {
        format!("Deleted 1 item, then: {error}")
    } else {
        format!("Deleted {done} items, then: {error}")
    }
}
