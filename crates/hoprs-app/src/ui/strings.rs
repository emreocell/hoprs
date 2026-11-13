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
/// The tab strip's actions on a connected SSH tab: another tab on its session.
pub const NEW_TERMINAL_TAB: &str = "New terminal tab";
pub const NEW_FILES_TAB: &str = "New files tab";
/// A terminal tab until the terminal itself lands (Plan 6 Task 4).
/// The terminal tab (Plan 6 Task 4).
pub const TERMINAL_OVERFLOWED: &str = "Output was too fast and the shell was closed";
pub const TERMINAL_REOPEN: &str = "Reopen";
pub const TERMINAL_RESTART: &str = "Restart shell";
pub const TERMINAL_SHELL_CLOSED: &str = "Shell closed";
pub const TERMINAL_OPEN_FAILED: &str = "Can't open a shell";
pub const TERMINAL_RETRY: &str = "Retry";
/// Written into the scrollback when a new shell starts after a reconnect.
pub const TERMINAL_RECONNECTED: &str = "— reconnected —";
pub const TERMINAL_SEARCH_PLACEHOLDER: &str = "Search scrollback";
pub const TERMINAL_SEARCH_NEXT: &str = "Next";
pub const TERMINAL_SEARCH_PREVIOUS: &str = "Previous";
pub const TERMINAL_SEARCH_CLOSE: &str = "Close";
pub const TERMINAL_SEARCH_NO_MATCH: &str = "No match";
/// "Shell exited (code N)", or without a code when the server sent none.
pub fn shell_exited(code: Option<u32>) -> String {
    match code {
        Some(code) => format!("Shell exited (code {code})"),
        None => "Shell exited".to_owned(),
    }
}

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
/// Removes done, skipped, cancelled and failed rows (the engine's `clear_finished`).
pub const CLEAR_FINISHED: &str = "Clear finished and failed";
/// [`CLEAR_FINISHED`] in a narrow window.
pub const CLEAR_FINISHED_SHORT: &str = "Clear finished";
pub const UPLOAD_ARROW: &str = "↑";
pub const DOWNLOAD_ARROW: &str = "↓";
pub const WAITING_FOR_WORKER: &str = "Waiting for a worker";
pub const STARTING: &str = "Starting…";
/// A folder whose files are being copied.
pub const TRANSFERRING: &str = "Transferring…";
/// A failed row the user dismissed (still failed in the engine; Retry stays).
pub const DISMISSED: &str = "Dismissed";
pub const PAUSED: &str = "Paused";
pub const CANCELLED: &str = "Cancelled";
pub const TRANSFER_DONE: &str = "Done";
pub const SKIPPED: &str = "Skipped";
pub const FILE_EXISTS: &str = "File exists —";
pub const OVERWRITE: &str = "Overwrite";
pub const RESUME: &str = "Resume";
pub const SKIP: &str = "Skip";
pub const PAUSE: &str = "Pause";
/// The engine also makes the answer its policy for every later conflict of this run.
pub const APPLY_TO_ALL: &str = "Apply to all (this session)";
pub const SITE_PAUSED: &str = "Paused — connection lost";
pub const DROP_NEEDS_SITE: &str = "Open a connected site to drop files into it";
pub const DROP_PANE_BUSY: &str =
    "The remote folder is busy or not listed yet; drop again in a moment";
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

/// A file of unknown size: "12 MB, 3.4 MB/s".
pub fn bytes_and_speed(done: u64, bytes_per_sec: u64) -> String {
    format!(
        "{}, {}",
        crate::ui::format::size(done),
        speed(bytes_per_sec)
    )
}

/// In the queue head once an answer went "to all": "Conflicts: overwrite (this session)".
pub fn session_policy(policy: hoprs_transfer::ConflictPolicy) -> String {
    use hoprs_transfer::ConflictPolicy::*;
    let name = match policy {
        Ask => "ask",
        Overwrite => "overwrite",
        Skip => "skip",
        OverwriteIfNewer => "overwrite if newer",
        Resume => "resume",
    };
    format!("Conflicts: {name} (this session)")
}

/// The same in a narrow window, as a short mono tag: "overwrite: session".
pub fn session_policy_tag(policy: hoprs_transfer::ConflictPolicy) -> String {
    use hoprs_transfer::ConflictPolicy::*;
    let name = match policy {
        Ask => "ask",
        Overwrite => "overwrite",
        Skip => "skip",
        OverwriteIfNewer => "if newer",
        Resume => "resume",
    };
    format!("{name}: session")
}

/// The queue database could not be opened: the engine runs on an in-memory queue.
pub fn queue_unavailable(reason: &str) -> String {
    format!("Transfer queue unavailable: {reason}. Transfers this session won't survive a restart.")
}

/// "Skipped: exists".
pub fn skipped(reason: &str) -> String {
    format!("{SKIPPED}: {reason}")
}

// Status bar.
pub const NOT_CONNECTED: &str = "Not connected";

// Status bar and connection states.
pub const CONNECTING: &str = "Connecting…";
pub const CONNECTED: &str = "Connected";
pub const WAITING_FOR_YOU: &str = "Waiting for you";
pub const CLOSING: &str = "Saving tabs and closing…";
pub const CLOSE_WORKSPACE: &str = "Close tabs";
pub const RECONNECTING_NOW: &str = "Reconnecting…";

/// A workspace's saved folder is gone; the pane shows a folder above it instead.
pub fn saved_folder_gone(saved: &str) -> String {
    format!("The saved folder {saved} is gone; showing the nearest folder instead.")
}

/// "Reconnecting (2/5)".
pub fn reconnecting(attempt: u32, of: u32) -> String {
    format!("Reconnecting ({attempt}/{of})")
}

/// "Disconnected — Connection lost".
pub fn disconnected(reason: &str) -> String {
    format!("Disconnected — {reason}")
}

/// `user@host:port`.
pub fn endpoint(user: &str, host: &str, port: u16) -> String {
    format!("{user}@{host}:{port}")
}

/// How an SSH session logged in: "ed25519 via ssh-agent", "rsa key", "password"...
pub fn ssh_auth(method: &hoprs_ssh::AuthMethod) -> String {
    use hoprs_ssh::AuthMethod;
    match method {
        AuthMethod::None => "no authentication".to_owned(),
        AuthMethod::Agent { key_type } => format!("{key_type} via ssh-agent"),
        AuthMethod::KeyFile { key_type } => format!("{key_type} key"),
        AuthMethod::Password => "password".to_owned(),
        AuthMethod::KeyboardInteractive => "keyboard-interactive".to_owned(),
    }
}

/// An FTP site's security, in place of a login method.
pub fn ftp_auth(protocol: hoprs_core::Protocol) -> Option<&'static str> {
    use hoprs_core::Protocol;
    match protocol {
        Protocol::Ssh => None,
        Protocol::Ftp => Some("FTP (unencrypted)"),
        Protocol::FtpsExplicit => Some("FTPS explicit"),
        Protocol::FtpsImplicit => Some("FTPS implicit"),
    }
}

/// "3 of 3 sites in Shop".
pub fn workspace_status(connected: usize, total: usize, name: &str) -> String {
    format!("{connected} of {} in {name}", site_count(total))
}

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

// Security and credential dialogs.
pub const HOST_KEY_TITLE: &str = "Unknown host key";
pub const HOST_KEY_INTRO: &str = "hoprs has not connected to this server before. Check the fingerprint with its administrator before you trust it.";
pub const TRUST_AND_CONNECT: &str = "Trust and connect";
pub const CERT_UNTRUSTED_TITLE: &str = "Untrusted certificate";
pub const CERT_UNTRUSTED_INTRO: &str = "The server's certificate can't be verified. Check the fingerprint with its administrator before you trust it.";
pub const CERT_CHANGED_TITLE: &str = "Certificate changed";
pub const CERT_CHANGED_INTRO: &str = "The server's certificate is not the one you trusted before. Someone may be intercepting the connection.";
pub const TRUST_CERTIFICATE: &str = "Trust this certificate";
pub const LABEL_HOST: &str = "Host";
pub const LABEL_ACCOUNT: &str = "Account";
pub const LABEL_KEY_TYPE: &str = "Key type";
pub const LABEL_FINGERPRINT: &str = "Fingerprint (SHA-256)";
pub const LABEL_REASON: &str = "Reason";
pub const LABEL_TRUSTED_BEFORE: &str = "Trusted before";
pub const LABEL_PRESENTED_NOW: &str = "Presented now";
pub const LABEL_KEY_FILE: &str = "Key";
pub const PASSWORD_TITLE: &str = "Password";
pub const PASSPHRASE_TITLE: &str = "Key passphrase";
pub const FIELD_PASSPHRASE: &str = "Passphrase";
pub const KBD_TITLE: &str = "Authentication";
pub const SAVE_IN_KEYRING: &str = "Save in the system keyring";
pub const SAVE_IN_KEYRING_HINT: &str = "Saved only once the connection succeeds";
pub const CONNECT: &str = "Connect";
pub const UNLOCK: &str = "Unlock";
pub const CONTINUE: &str = "Continue";

/// Under a dialog, when others wait behind it: "1 more waiting".
pub fn dialogs_waiting(n: usize) -> String {
    format!("{n} more waiting")
}

/// The warning of a host known with other key types.
pub fn other_key_types(types: &[String]) -> String {
    format!(
        "This host is known with a different key type ({}). Someone may be impersonating it.",
        types.join(", ")
    )
}

/// Why a certificate needs the user's decision.
pub fn cert_reason(reason: &hoprs_remote::CertReason) -> String {
    use hoprs_remote::CertReason;
    match reason {
        CertReason::Expired => "Expired".to_owned(),
        CertReason::NotYetValid => "Not valid yet".to_owned(),
        CertReason::WrongName => "Issued for a different name".to_owned(),
        CertReason::UnknownIssuer => {
            "Unknown authority (self-signed or untrusted issuer)".to_owned()
        }
        CertReason::Other(text) => text.clone(),
    }
}

// Changed host key, in the tab.
pub const HOST_KEY_CHANGED_TITLE: &str = "The host key has changed";
pub const HOST_KEY_CHANGED_BODY: &str = "The server presented a different key than the one recorded for it. That happens when the server was reinstalled, or when someone intercepts the connection. hoprs won't connect while the old entry is in known_hosts.";
pub const HOST_KEY_CHANGED_SESSION_BODY: &str = "The server presented a different key than the one this session verified when it connected. That happens when the server was reinstalled, or when someone intercepts the connection. A new session checks the key against known_hosts again.";
pub const REMOVE_OLD_ENTRY: &str = "Remove the old entry…";
pub const CLOSE_AND_RECONNECT: &str = "Close and reconnect";
pub const READING_KNOWN_HOSTS: &str = "Reading known_hosts…";
pub const REMOVING_KNOWN_HOSTS: &str = "Removing…";
pub const REMOVE: &str = "Remove";
pub const CONNECT_AGAIN: &str = "Connect again";
pub const NO_KNOWN_HOSTS_ENTRY: &str =
    "known_hosts has no entry for this host any more. Connect again to check the new key.";

/// "known_hosts line 3".
pub fn known_hosts_line(line: usize) -> String {
    format!("known_hosts line {line}")
}

/// The removal question: "Remove 2 lines for [h]:2222 from known_hosts? …".
pub fn remove_known_hosts_question(lines: usize, host: &str) -> String {
    let lines = if lines == 1 {
        "1 line".to_owned()
    } else {
        format!("{lines} lines")
    };
    format!(
        "Remove {lines} for {host} from known_hosts? The next connect asks you to trust the new key."
    )
}

/// The warning for lines that also name other hosts.
pub fn also_covers_other_hosts(lines: &[usize]) -> String {
    let list: Vec<String> = lines.iter().map(ToString::to_string).collect();
    match lines {
        [line] => format!(
            "Line {line} also covers other hosts (a pattern or a list of names): removing it forgets this key for them too."
        ),
        _ => format!(
            "Lines {} also cover other hosts (patterns or lists of names): removing them forgets these keys for them too.",
            list.join(", ")
        ),
    }
}

/// A connected tab whose site's host or port changed (an edit, a restore).
pub const SITE_ENDPOINT_CHANGED: &str =
    "The site's host or port changed: this session was closed. Reconnect to use the new settings.";

// Import from FileZilla.
pub const IMPORT_TITLE: &str = "Import from FileZilla";
pub const IMPORT_STEP_FILE: &str = "1  File";
pub const IMPORT_STEP_FOLDERS: &str = "2  Folders";
pub const IMPORT_STEP_REPORT: &str = "3  Report";
pub const IMPORT_FILE_LABEL: &str = "FileZilla's site manager file";
pub const IMPORT_FILE_HINT: &str = "Usually sitemanager.xml in FileZilla's configuration folder. Passwords go to the system keyring.";
pub const IMPORT_PATH_REQUIRED: &str = "Choose the sitemanager.xml file";
pub const CHOOSE_FILE: &str = "Choose…";
pub const NEXT: &str = "Next";
pub const BACK: &str = "Back";
pub const IMPORT_FOLDERS_QUESTION: &str = "Turn FileZilla folders into workspaces?";
pub const IMPORT_FOLDERS_HINT: &str = "Yes: one workspace per folder, named by its path, with the folder's sites. No: the folder path goes in front of each site name (\"Work/Alpha\").";
pub const IMPORT_FOLDERS_YES: &str = "Yes, make workspaces";
pub const IMPORT_FOLDERS_NO: &str = "No, keep them in the names";
pub const IMPORT_RUNNING: &str = "Importing…";
pub const IMPORT_IMPORTED: &str = "Imported";
pub const IMPORT_SKIPPED: &str = "Skipped";
pub const IMPORT_WORKSPACES_CREATED: &str = "Workspaces created";
pub const IMPORT_WORKSPACES_NOT_CREATED: &str = "Workspaces not created";
pub const IMPORT_FAILURES: &str = "Failures";
pub const IMPORT_NONE: &str = "None";
pub const IMPORT_FAILED_KEEP_SOURCE: &str =
    "Some entries could not be saved: keep sitemanager.xml and import again once fixed.";
pub const IMPORT_PASSWORDS_KEEP_SOURCE: &str =
    "The keyring refused the passwords of these sites: keep sitemanager.xml, it still has them.";
pub const DELETE_SITEMANAGER: &str = "Delete sitemanager.xml";
pub const DELETE_SITEMANAGER_CONFIRM: &str =
    "Delete FileZilla's sitemanager.xml? FileZilla will no longer have these sites.";
pub const DELETING_SITEMANAGER: &str = "Deleting…";
pub const SITEMANAGER_DELETED: &str = "sitemanager.xml deleted";
pub const DONE: &str = "Done";
pub const PICK_SITEMANAGER: &str = "Choose FileZilla's sitemanager.xml";
pub const PICK_FILTER_XML: &str = "XML files";

// Settings.
pub const SETTINGS_TITLE: &str = "Settings";
pub const SETTINGS_TRANSFERS: &str = "Transfers";
pub const SETTINGS_WORKERS: &str = "Workers per site";
pub const SETTINGS_WORKERS_HINT: &str =
    "Parallel transfers per site, 1 to 16. Applies to the next connections.";
pub const SETTINGS_POLICY: &str = "When a file already exists";
pub const SETTINGS_POLICY_HINT: &str =
    "Applies to the next transfers. Ask shows the choice in the queue.";
pub const SETTINGS_UNREADABLE: &str = "settings.toml can't be read, so the defaults apply. Changes last until hoprs closes and are not saved, to keep the file as it is: fix or delete it to save settings.";
pub const SETTINGS_SAVE_FAILED: &str = "Can't save settings.toml";
pub const SETTINGS_TERMINAL: &str = "Terminal";
pub const SETTINGS_FONT_SIZE: &str = "Font size";
pub const SETTINGS_SCROLLBACK: &str = "Scrollback lines";
pub const SETTINGS_SCROLLBACK_HINT: &str = "Applies to terminals opened from now on.";
pub const SETTINGS_THEME: &str = "Theme";
pub const THEME_HOPRS_DARK: &str = "hoprs dark";
pub const POLICY_ASK: &str = "Ask";
pub const POLICY_OVERWRITE: &str = "Overwrite";
pub const POLICY_SKIP: &str = "Skip";
pub const POLICY_OVERWRITE_IF_NEWER: &str = "Overwrite if newer";
pub const POLICY_RESUME: &str = "Resume";
pub const DECREASE: &str = "−";
pub const INCREASE: &str = "+";

// Backup.
pub const BACKUP: &str = "Backup";
pub const BACKUP_HINT: &str = "One encrypted file with every site, workspace and saved password. Keep the password: without it the backup can't be opened.";
pub const EXPORT_BACKUP: &str = "Export backup…";
pub const RESTORE_BACKUP: &str = "Restore backup…";
pub const BACKUP_FILE: &str = "File";
pub const BACKUP_PASSWORD: &str = "Password";
pub const BACKUP_PASSWORD_CONFIRM: &str = "Password again";
pub const BACKUP_PATH_REQUIRED: &str = "Choose a file";
pub const BACKUP_PASSWORD_REQUIRED: &str = "Enter a password";
pub const BACKUP_PASSWORDS_DIFFER: &str = "The passwords don't match";
pub const EXPORT: &str = "Export";
pub const EXPORT_OVERWRITE_CONFIRM: &str =
    "This file already exists. Overwrite it with the new backup?";
pub const RESTORE: &str = "Restore";
pub const EXPORTING: &str = "Encrypting the backup… this takes a few seconds";
pub const RESTORING: &str = "Decrypting the backup… this takes a few seconds";
pub const RESTORE_MODE: &str = "Mode";
pub const RESTORE_MERGE: &str = "Merge";
pub const RESTORE_MERGE_HINT: &str = "Adds what is new; sites and workspaces you have stay.";
pub const RESTORE_REPLACE: &str = "Replace";
pub const RESTORE_REPLACE_HINT: &str =
    "Everything becomes the backup: sites, workspaces and passwords not in it are removed.";
pub const RESTORE_REPLACE_CONFIRM: &str =
    "Replace every site, workspace and saved password with the backup's?";
pub const REPLACE: &str = "Replace";
pub const EXPORT_FAILED: &str = "The backup was not written";
pub const RESTORE_FAILED: &str = "Nothing was restored";
pub const BACKUP_SECRET_ERRORS: &str = "Passwords not restored";
pub const BACKUP_MISSING_SECRETS: &str = "Saved without their password (not in the keyring)";
pub const BACKUP_SKIPPED_UNREADABLE: &str = "Not exported (unreadable entries)";
pub const BACKUP_SKIPPED_SITES: &str = "Sites already here (kept)";
pub const BACKUP_SKIPPED_WORKSPACES: &str = "Workspaces already here (kept)";
pub const BACKUP_FILE_NAME: &str = "hoprs-backup.hoprs";
pub const PICK_EXPORT: &str = "Save the hoprs backup";
pub const PICK_RESTORE: &str = "Choose a hoprs backup";

/// "Exported 12 sites, 3 workspaces and 9 passwords."
pub fn export_done(sites: usize, workspaces: usize, secrets: usize) -> String {
    format!(
        "Exported {}, {} and {}.",
        count(sites, "site", "sites"),
        count(workspaces, "workspace", "workspaces"),
        count(secrets, "password", "passwords")
    )
}

/// "Restored 2 sites, 1 workspace and 2 passwords."
pub fn restore_done(sites: usize, workspaces: usize, secrets: usize) -> String {
    format!(
        "Restored {}, {} and {}.",
        count(sites, "site", "sites"),
        count(workspaces, "workspace", "workspaces"),
        count(secrets, "password", "passwords")
    )
}

/// A report section's head: "Imported  4".
pub fn section_count(title: &str, count: usize) -> String {
    format!("{title}  {count}")
}

fn count(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}
