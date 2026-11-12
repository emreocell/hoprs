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
pub const LOADING: &str = "Loading…";
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
pub const CONNECTING: &str = "Connecting…";
pub const CONNECTED: &str = "Connected";
pub const FILES_ARRIVE: &str = "File panes open here.";
pub const CANNOT_CONNECT: &str = "Can't connect";
pub const KEYRING_UNAVAILABLE: &str =
    "The system keyring was unavailable, so the saved password was not used.";
pub const EDIT_SITE: &str = "Edit site";

// Queue.
pub const TRANSFERS: &str = "Transfers";
pub const QUEUE_ACTIVE: &str = "Active";
pub const QUEUE_QUEUED: &str = "Queued";
pub const QUEUE_FAILED: &str = "Failed";
pub const QUEUE_DONE: &str = "Done";
pub const QUEUE_TOTAL_IDLE: &str = "Idle";
pub const QUEUE_HIDE: &str = "Hide";
pub const QUEUE_SHOW: &str = "Show";
pub const NO_TRANSFERS: &str = "No transfers";

// Status bar.
pub const NOT_CONNECTED: &str = "Not connected";
