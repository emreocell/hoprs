//! Every user-visible string, in one place (i18n-ready). Sentence case.

use crate::i18n::{tr, tr_args};

pub fn app_title() -> &'static str {
    tr("app-title")
}
pub fn already_running() -> &'static str {
    tr("already-running")
}
pub fn cannot_open_profiles() -> &'static str {
    tr("cannot-open-profiles")
}

// Top bar.
pub fn brand() -> &'static str {
    tr("brand")
}
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub fn no_sessions() -> &'static str {
    tr("no-sessions")
}
pub fn go_to_anything() -> &'static str {
    tr("go-to-anything")
}
pub fn go_to_anything_key() -> &'static str {
    tr("go-to-anything-key")
}

// The Ctrl+K palette.
pub fn palette_placeholder() -> &'static str {
    tr("palette-placeholder")
}
pub fn palette_workspaces_placeholder() -> &'static str {
    tr("palette-workspaces-placeholder")
}
pub fn palette_no_results() -> &'static str {
    tr("palette-no-results")
}
pub fn palette_group_action() -> &'static str {
    tr("palette-group-action")
}
pub fn palette_group_tab() -> &'static str {
    tr("palette-group-tab")
}
pub fn palette_group_site() -> &'static str {
    tr("palette-group-site")
}
pub fn palette_group_workspace() -> &'static str {
    tr("palette-group-workspace")
}
pub fn palette_kind_terminal() -> &'static str {
    tr("palette-kind-terminal")
}
pub fn palette_kind_files() -> &'static str {
    tr("palette-kind-files")
}
pub fn palette_toggle_queue() -> &'static str {
    tr("palette-toggle-queue")
}
pub fn palette_toggle_log() -> &'static str {
    tr("palette-toggle-log")
}
pub fn palette_close_tab() -> &'static str {
    tr("palette-close-tab")
}
pub fn palette_open_workspace() -> &'static str {
    tr("palette-open-workspace")
}
pub fn palette_edit() -> &'static str {
    tr("palette-edit")
}
pub fn palette_upload() -> &'static str {
    tr("palette-upload")
}
pub fn palette_download() -> &'static str {
    tr("palette-download")
}
pub fn palette_copy() -> &'static str {
    tr("palette-copy")
}
pub fn palette_paste() -> &'static str {
    tr("palette-paste")
}
pub fn palette_find() -> &'static str {
    tr("palette-find")
}
pub fn palette_key_copy() -> &'static str {
    tr("palette-key-copy")
}
pub fn palette_key_paste() -> &'static str {
    tr("palette-key-paste")
}
pub fn palette_key_find() -> &'static str {
    tr("palette-key-find")
}

// Sidebar.
pub fn filter_sites() -> &'static str {
    tr("filter-sites")
}
pub fn workspaces() -> &'static str {
    tr("workspaces")
}
pub fn all_sites() -> &'static str {
    tr("all-sites")
}
pub fn new() -> &'static str {
    tr("new")
}
pub fn no_workspaces() -> &'static str {
    tr("no-workspaces")
}
pub fn no_sites() -> &'static str {
    tr("no-sites")
}
pub fn import_filezilla() -> &'static str {
    tr("import-filezilla")
}
pub fn settings() -> &'static str {
    tr("settings")
}
pub fn new_site() -> &'static str {
    tr("new-site")
}
pub fn no_matches() -> &'static str {
    tr("no-matches")
}
pub fn open_workspace() -> &'static str {
    tr("open-workspace")
}
pub fn edit() -> &'static str {
    tr("edit")
}
pub fn site_missing() -> &'static str {
    tr("site-missing")
}
pub fn cannot_load_sites() -> &'static str {
    tr("cannot-load-sites")
}
pub fn retry() -> &'static str {
    tr("retry")
}
pub fn unreadable_entry() -> &'static str {
    tr("unreadable-entry")
}
pub fn open_profile_folder() -> &'static str {
    tr("open-profile-folder")
}
/// Opening the profile folder failed.
pub fn cannot_open_folder(reason: &str) -> String {
    tr_args("cannot-open-folder", &[("reason", reason.into())])
}

/// "1 site", "3 sites".
pub fn site_count(n: usize) -> String {
    tr_args("site-count", &[("count", n.into())])
}

/// A workspace's count: "3 sites", or "2 of 3 sites" when some are missing.
pub fn workspace_site_count(present: usize, total: usize) -> String {
    if present == total {
        site_count(total)
    } else {
        tr_args(
            "workspace-site-count-partial",
            &[("present", present.into()), ("total", total.into())],
        )
    }
}

/// An unreadable entry: its name, or where it is.
pub fn unreadable_name(name: Option<&str>, file: &str, index: usize) -> String {
    match name {
        Some(name) => name.to_owned(),
        None => tr_args(
            "unreadable-name-fallback",
            &[("number", (index + 1).into()), ("file", file.into())],
        ),
    }
}

// Site form.
pub fn site_form_new() -> &'static str {
    tr("site-form-new")
}
pub fn site_form_edit() -> &'static str {
    tr("site-form-edit")
}
pub fn field_name() -> &'static str {
    tr("field-name")
}
pub fn field_name_hint() -> &'static str {
    tr("field-name-hint")
}
pub fn field_host() -> &'static str {
    tr("field-host")
}
pub fn field_port() -> &'static str {
    tr("field-port")
}
pub fn field_user() -> &'static str {
    tr("field-user")
}
pub fn field_protocol() -> &'static str {
    tr("field-protocol")
}
pub fn field_password() -> &'static str {
    tr("field-password")
}
pub fn password_optional() -> &'static str {
    tr("password-optional")
}
pub fn password_replace() -> &'static str {
    tr("password-replace")
}
pub fn password_saved() -> &'static str {
    tr("password-saved")
}
pub fn forget_password() -> &'static str {
    tr("forget-password")
}
pub fn ssh_options() -> &'static str {
    tr("ssh-options")
}
pub fn use_agent() -> &'static str {
    tr("use-agent")
}
pub fn key_file() -> &'static str {
    tr("key-file")
}
pub fn key_file_hint() -> &'static str {
    tr("key-file-hint")
}
pub fn open_as() -> &'static str {
    tr("open-as")
}
pub fn open_as_terminal() -> &'static str {
    tr("open-as-terminal")
}
pub fn open_as_files() -> &'static str {
    tr("open-as-files")
}
pub fn open_as_both() -> &'static str {
    tr("open-as-both")
}
pub fn ftp_options() -> &'static str {
    tr("ftp-options")
}
pub fn ftp_mode() -> &'static str {
    tr("ftp-mode")
}
pub fn ftp_passive() -> &'static str {
    tr("ftp-passive")
}
pub fn ftp_active() -> &'static str {
    tr("ftp-active")
}
pub fn nat_workaround() -> &'static str {
    tr("nat-workaround")
}
pub fn start_folders() -> &'static str {
    tr("start-folders")
}
pub fn remote_dir() -> &'static str {
    tr("remote-dir")
}
pub fn local_dir() -> &'static str {
    tr("local-dir")
}
pub fn folder_hint() -> &'static str {
    tr("folder-hint")
}
pub fn host_required() -> &'static str {
    tr("host-required")
}
pub fn port_invalid() -> &'static str {
    tr("port-invalid")
}
pub fn save() -> &'static str {
    tr("save")
}
pub fn saving() -> &'static str {
    tr("saving")
}
pub fn cancel() -> &'static str {
    tr("cancel")
}
pub fn delete() -> &'static str {
    tr("delete")
}
pub fn keep() -> &'static str {
    tr("keep")
}
pub fn delete_site_confirm() -> &'static str {
    tr("delete-site-confirm")
}

// Workspace form.
pub fn workspace_form_new() -> &'static str {
    tr("workspace-form-new")
}
pub fn workspace_form_edit() -> &'static str {
    tr("workspace-form-edit")
}
pub fn workspace_sites() -> &'static str {
    tr("workspace-sites")
}
pub fn workspace_no_sites() -> &'static str {
    tr("workspace-no-sites")
}
pub fn name_required() -> &'static str {
    tr("name-required")
}
pub fn delete_workspace_confirm() -> &'static str {
    tr("delete-workspace-confirm")
}

// Center.
pub fn no_session_title() -> &'static str {
    tr("no-session-title")
}
pub fn no_session_hint() -> &'static str {
    tr("no-session-hint")
}
pub fn keyring_unavailable() -> &'static str {
    tr("keyring-unavailable")
}
pub fn edit_site() -> &'static str {
    tr("edit-site")
}

// Tabs.
pub fn close_tab() -> &'static str {
    tr("close-tab")
}
/// The tab strip's actions on a connected SSH tab: another tab on its session.
pub fn new_terminal_tab() -> &'static str {
    tr("new-terminal-tab")
}
pub fn new_files_tab() -> &'static str {
    tr("new-files-tab")
}
pub fn terminal_overflowed() -> &'static str {
    tr("terminal-overflowed")
}
pub fn terminal_reopen() -> &'static str {
    tr("terminal-reopen")
}
pub fn terminal_restart() -> &'static str {
    tr("terminal-restart")
}
pub fn terminal_shell_closed() -> &'static str {
    tr("terminal-shell-closed")
}
pub fn terminal_open_failed() -> &'static str {
    tr("terminal-open-failed")
}
pub fn terminal_retry() -> &'static str {
    tr("terminal-retry")
}
pub fn terminal_waiting_for_connection() -> &'static str {
    tr("terminal-waiting-for-connection")
}
pub fn terminal_input_dropped() -> &'static str {
    tr("terminal-input-dropped")
}
/// Written into the scrollback when a new shell starts after a reconnect.
pub fn terminal_reconnected() -> &'static str {
    tr("terminal-reconnected")
}
pub fn terminal_search_placeholder() -> &'static str {
    tr("terminal-search-placeholder")
}
pub fn terminal_search_next() -> &'static str {
    tr("terminal-search-next")
}
pub fn terminal_search_previous() -> &'static str {
    tr("terminal-search-previous")
}
pub fn terminal_search_close() -> &'static str {
    tr("terminal-search-close")
}
pub fn terminal_search_no_match() -> &'static str {
    tr("terminal-search-no-match")
}
/// "Shell exited (code N)", or without a code when the server sent none.
pub fn shell_exited(code: Option<u32>) -> String {
    match code {
        Some(code) => tr_args("shell-exited-code", &[("code", code.into())]),
        None => tr("shell-exited").to_owned(),
    }
}

// File panes.
pub fn local() -> &'static str {
    tr("local")
}
pub fn col_name() -> &'static str {
    tr("col-name")
}
pub fn col_size() -> &'static str {
    tr("col-size")
}
pub fn col_modified() -> &'static str {
    tr("col-modified")
}
pub fn col_permissions() -> &'static str {
    tr("col-permissions")
}
pub fn sort_ascending() -> &'static str {
    tr("sort-ascending")
}
pub fn sort_descending() -> &'static str {
    tr("sort-descending")
}
/// The row that goes up one folder.
pub fn parent_dir() -> &'static str {
    tr("parent-dir")
}
/// After a folder name, and between breadcrumb segments.
pub fn path_separator() -> &'static str {
    tr("path-separator")
}
/// The breadcrumb's first segment for the home folder.
pub fn home_dir() -> &'static str {
    tr("home-dir")
}
/// The size column of a folder.
pub fn no_size() -> &'static str {
    tr("no-size")
}
pub fn empty_folder() -> &'static str {
    tr("empty-folder")
}
pub fn reconnect() -> &'static str {
    tr("reconnect")
}
/// Units of [`crate::ui::format::size`], base 1024.
pub fn size_unit(i: usize) -> &'static str {
    match i {
        0 => tr("size-unit-b"),
        1 => tr("size-unit-kb"),
        2 => tr("size-unit-mb"),
        3 => tr("size-unit-gb"),
        4 => tr("size-unit-tb"),
        5 => tr("size-unit-pb"),
        6 => tr("size-unit-eb"),
        _ => "",
    }
}
/// Short month names for file dates.
pub fn month(m0: u32) -> &'static str {
    match m0 {
        0 => tr("month-1"),
        1 => tr("month-2"),
        2 => tr("month-3"),
        3 => tr("month-4"),
        4 => tr("month-5"),
        5 => tr("month-6"),
        6 => tr("month-7"),
        7 => tr("month-8"),
        8 => tr("month-9"),
        9 => tr("month-10"),
        10 => tr("month-11"),
        11 => tr("month-12"),
        _ => "",
    }
}

// Queue.
pub fn transfers() -> &'static str {
    tr("transfers")
}
pub fn queue_active() -> &'static str {
    tr("queue-active")
}
pub fn queue_queued() -> &'static str {
    tr("queue-queued")
}
pub fn queue_failed() -> &'static str {
    tr("queue-failed")
}
pub fn queue_done() -> &'static str {
    tr("queue-done")
}
pub fn queue_hide() -> &'static str {
    tr("queue-hide")
}
pub fn queue_show() -> &'static str {
    tr("queue-show")
}
pub fn no_transfers() -> &'static str {
    tr("no-transfers")
}
pub fn no_transfers_hint() -> &'static str {
    tr("no-transfers-hint")
}
pub fn queue_loading() -> &'static str {
    tr("queue-loading")
}
pub fn nothing_active() -> &'static str {
    tr("nothing-active")
}
pub fn nothing_queued() -> &'static str {
    tr("nothing-queued")
}
pub fn nothing_failed() -> &'static str {
    tr("nothing-failed")
}
pub fn nothing_done() -> &'static str {
    tr("nothing-done")
}
/// Removes done, skipped, cancelled and failed rows (the engine's `clear_finished`).
pub fn clear_finished() -> &'static str {
    tr("clear-finished")
}
/// [`CLEAR_FINISHED`] in a narrow window.
pub fn clear_finished_short() -> &'static str {
    tr("clear-finished-short")
}
// Technical log panel.
pub fn log() -> &'static str {
    tr("log")
}
pub fn log_error() -> &'static str {
    tr("log-error")
}
pub fn log_warn() -> &'static str {
    tr("log-warn")
}
pub fn log_info() -> &'static str {
    tr("log-info")
}
pub fn log_debug() -> &'static str {
    tr("log-debug")
}
pub fn filter_log() -> &'static str {
    tr("filter-log")
}
/// Copies the lines the filters show (all of them without a filter).
pub fn copy_shown() -> &'static str {
    tr("copy-shown")
}
pub fn clear() -> &'static str {
    tr("clear")
}
pub fn clear_hint() -> &'static str {
    tr("clear-hint")
}
pub fn log_level_hint() -> &'static str {
    tr("log-level-hint")
}
pub fn no_log_lines() -> &'static str {
    tr("no-log-lines")
}
pub fn no_log_matches() -> &'static str {
    tr("no-log-matches")
}

/// The log panel's count: "1 line", "120 lines".
pub fn log_lines(n: usize) -> String {
    tr_args("log-lines", &[("count", n.into())])
}

pub fn upload_arrow() -> &'static str {
    tr("upload-arrow")
}
pub fn download_arrow() -> &'static str {
    tr("download-arrow")
}
pub fn waiting_for_worker() -> &'static str {
    tr("waiting-for-worker")
}
pub fn starting() -> &'static str {
    tr("starting")
}
/// A folder whose files are being copied.
pub fn transferring() -> &'static str {
    tr("transferring")
}
/// A failed row the user dismissed (still failed in the engine; Retry stays).
pub fn dismissed() -> &'static str {
    tr("dismissed")
}
pub fn paused() -> &'static str {
    tr("paused")
}
pub fn cancelled() -> &'static str {
    tr("cancelled")
}
pub fn transfer_done() -> &'static str {
    tr("transfer-done")
}
pub fn file_exists() -> &'static str {
    tr("file-exists")
}
pub fn overwrite() -> &'static str {
    tr("overwrite")
}
pub fn resume() -> &'static str {
    tr("resume")
}
pub fn skip() -> &'static str {
    tr("skip")
}
pub fn pause() -> &'static str {
    tr("pause")
}
/// The engine also makes the answer its policy for every later conflict of this run.
pub fn apply_to_all() -> &'static str {
    tr("apply-to-all")
}
pub fn site_paused() -> &'static str {
    tr("site-paused")
}
pub fn drop_needs_site() -> &'static str {
    tr("drop-needs-site")
}
pub fn drop_pane_busy() -> &'static str {
    tr("drop-pane-busy")
}
pub fn drop_unsupported() -> &'static str {
    tr("drop-unsupported")
}

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
    // The leading space is part of the text (it follows the speed); Fluent trims it, so it is added here.
    format!(
        " {}",
        tr_args("total-workers", &[("workers", workers.into())])
    )
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
        Ask => tr("policy-ask"),
        Overwrite => tr("policy-overwrite"),
        Skip => tr("policy-skip"),
        OverwriteIfNewer => tr("policy-overwrite-if-newer"),
        Resume => tr("policy-resume"),
    };
    tr_args("session-policy", &[("name", name.into())])
}

/// The same in a narrow window, as a short mono tag: "overwrite: session".
pub fn session_policy_tag(policy: hoprs_transfer::ConflictPolicy) -> String {
    use hoprs_transfer::ConflictPolicy::*;
    let name = match policy {
        Ask => tr("policy-ask"),
        Overwrite => tr("policy-overwrite"),
        Skip => tr("policy-skip"),
        OverwriteIfNewer => tr("policy-if-newer"),
        Resume => tr("policy-resume"),
    };
    tr_args("session-policy-tag", &[("name", name.into())])
}

/// The queue database could not be opened: the engine runs on an in-memory queue.
pub fn queue_unavailable(reason: &str) -> String {
    tr_args("queue-unavailable", &[("reason", reason.into())])
}

/// A skip reason the engine stored, in the user's language; an unknown one passes through.
pub fn skip_reason(raw: &str) -> String {
    match raw {
        "exists" => tr("skip-exists").to_owned(),
        "not newer" => tr("skip-not-newer").to_owned(),
        "already complete" => tr("skip-already-complete").to_owned(),
        other => other.to_owned(),
    }
}

/// "Skipped: exists".
pub fn skipped(reason: &str) -> String {
    tr_args(
        "skipped",
        &[("reason", skip_reason(reason).as_str().into())],
    )
}

// Status bar.
pub fn not_connected() -> &'static str {
    tr("not-connected")
}

// Status bar and connection states.
pub fn connecting() -> &'static str {
    tr("connecting")
}
pub fn connected() -> &'static str {
    tr("connected")
}
pub fn waiting_for_you() -> &'static str {
    tr("waiting-for-you")
}
pub fn closing() -> &'static str {
    tr("closing")
}
pub fn close_workspace() -> &'static str {
    tr("close-workspace")
}
pub fn reconnecting_now() -> &'static str {
    tr("reconnecting-now")
}

/// A workspace's saved folder is gone; the pane shows a folder above it instead.
pub fn saved_folder_gone(saved: &str) -> String {
    tr_args("saved-folder-gone", &[("saved", saved.into())])
}

/// "Reconnecting (2/5)".
pub fn reconnecting(attempt: u32, of: u32) -> String {
    tr_args(
        "reconnecting",
        &[("attempt", attempt.into()), ("of", of.into())],
    )
}

/// "Disconnected — Connection lost".
pub fn disconnected(reason: &str) -> String {
    tr_args("disconnected", &[("reason", reason.into())])
}

/// `user@host:port`.
pub fn endpoint(user: &str, host: &str, port: u16) -> String {
    format!("{user}@{host}:{port}")
}

/// How an SSH session logged in: "ed25519 via ssh-agent", "rsa key", "password"...
pub fn ssh_auth(method: &hoprs_ssh::AuthMethod) -> String {
    use hoprs_ssh::AuthMethod;
    match method {
        AuthMethod::None => tr("auth-none").to_owned(),
        AuthMethod::Agent { key_type } => tr_args("auth-agent", &[("key_type", key_type.into())]),
        AuthMethod::KeyFile { key_type } => {
            tr_args("auth-key-file", &[("key_type", key_type.into())])
        }
        AuthMethod::Password => tr("auth-password").to_owned(),
        AuthMethod::KeyboardInteractive => tr("auth-keyboard-interactive").to_owned(),
    }
}

/// An FTP site's security, in place of a login method.
pub fn ftp_auth(protocol: hoprs_core::Protocol) -> Option<&'static str> {
    use hoprs_core::Protocol;
    match protocol {
        Protocol::Ssh => None,
        Protocol::Ftp => Some(tr("ftp-auth-ftp")),
        Protocol::FtpsExplicit => Some(tr("ftp-auth-ftps-explicit")),
        Protocol::FtpsImplicit => Some(tr("ftp-auth-ftps-implicit")),
    }
}

/// "3 of 3 sites in Shop".
pub fn workspace_status(connected: usize, total: usize, name: &str) -> String {
    tr_args(
        "workspace-status",
        &[
            ("connected", connected.into()),
            ("sites", site_count(total).as_str().into()),
            ("name", name.into()),
        ],
    )
}

// File actions.
pub fn new_folder() -> &'static str {
    tr("new-folder")
}
pub fn rename() -> &'static str {
    tr("rename")
}
pub fn permissions() -> &'static str {
    tr("permissions")
}
/// Short toolbar labels for the narrowest panes (`ui::fit::Toolbar::Narrow`).
pub fn new_folder_short() -> &'static str {
    tr("new-folder-short")
}
pub fn permissions_short() -> &'static str {
    tr("permissions-short")
}
pub fn key_new_folder() -> &'static str {
    tr("key-new-folder")
}
pub fn key_rename() -> &'static str {
    tr("key-rename")
}
pub fn key_delete() -> &'static str {
    tr("key-delete")
}
pub fn key_edit() -> &'static str {
    tr("key-edit")
}
pub fn upload() -> &'static str {
    tr("upload")
}
pub fn download() -> &'static str {
    tr("download")
}
pub fn key_transfer() -> &'static str {
    tr("key-transfer")
}
/// The name a new folder starts with.
pub fn new_folder_name() -> &'static str {
    tr("new-folder-name")
}
pub fn name_empty() -> &'static str {
    tr("name-empty")
}
pub fn name_invalid() -> &'static str {
    tr("name-invalid")
}
pub fn name_invalid_windows() -> &'static str {
    tr("name-invalid-windows")
}
pub fn name_spaces() -> &'static str {
    tr("name-spaces")
}
pub fn mixed_permissions() -> &'static str {
    tr("mixed-permissions")
}
pub fn name_taken() -> &'static str {
    tr("name-taken")
}
pub fn dismiss() -> &'static str {
    tr("dismiss")
}
pub fn creating_folder() -> &'static str {
    tr("creating-folder")
}
pub fn renaming() -> &'static str {
    tr("renaming")
}
pub fn changing_permissions() -> &'static str {
    tr("changing-permissions")
}
pub fn delete_folders_note() -> &'static str {
    tr("delete-folders-note")
}
pub fn permissions_title() -> &'static str {
    tr("permissions-title")
}
pub fn perm_owner() -> &'static str {
    tr("perm-owner")
}
pub fn perm_group() -> &'static str {
    tr("perm-group")
}
pub fn perm_others() -> &'static str {
    tr("perm-others")
}
pub fn perm_read() -> &'static str {
    tr("perm-read")
}
pub fn perm_write() -> &'static str {
    tr("perm-write")
}
pub fn perm_execute() -> &'static str {
    tr("perm-execute")
}
pub fn perm_octal() -> &'static str {
    tr("perm-octal")
}
pub fn octal_invalid() -> &'static str {
    tr("octal-invalid")
}
pub fn apply() -> &'static str {
    tr("apply")
}
/// How many names a confirmation lists before "and N more".
pub const LISTED_NAMES: usize = 5;

/// "New folder 2".
pub fn numbered(name: &str, n: u32) -> String {
    format!("{name} {n}")
}

/// "Delete report.pdf?", "Delete 3 items?".
pub fn delete_question(names: &[&str]) -> String {
    match names {
        [one] => tr_args("delete-question-one", &[("name", (*one).into())]),
        many => tr_args("delete-question-many", &[("count", many.len().into())]),
    }
}

/// "and 4 more".
pub fn and_more(n: usize) -> String {
    tr_args("and-more", &[("count", n.into())])
}

/// "Deleting… 12 items".
pub fn deleting(done: u64) -> String {
    tr_args("deleting", &[("done", done.into())])
}

/// A delete that stopped after removing some items.
pub fn deleted_then(done: u64, error: &str) -> String {
    tr_args(
        "deleted-then",
        &[("done", done.into()), ("error", error.into())],
    )
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
pub const SETTINGS_KEEPALIVE: &str = "Keepalive";
pub const SETTINGS_KEEPALIVE_HINT: &str = "How often an idle connection is checked, so servers and routers keep it open. Applies to the next connections.";

/// A keepalive interval: "30 s".
pub fn seconds(secs: u64) -> String {
    format!("{secs} s")
}
pub const SETTINGS_UNREADABLE: &str = "settings.toml can't be read, so the defaults apply. Changes last until hoprs closes and are not saved, to keep the file as it is: fix or delete it to save settings.";
pub const SETTINGS_SAVE_FAILED: &str = "Can't save settings.toml";
pub const SETTINGS_TERMINAL: &str = "Terminal";
pub const SETTINGS_FONT_SIZE: &str = "Font size";
pub const SETTINGS_SCROLLBACK: &str = "Scrollback lines";
pub const SETTINGS_SCROLLBACK_HINT: &str = "Applies to terminals opened from now on.";
pub const SETTINGS_THEME: &str = "Theme";
pub fn settings_language() -> &'static str {
    tr("settings-language")
}
pub fn language_system() -> &'static str {
    tr("language-system")
}
pub fn language_english() -> &'static str {
    tr("language-english")
}
pub fn language_pt_br() -> &'static str {
    tr("language-pt-br")
}
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

// Remote editing.
/// The Edit action (remote pane).
pub const EDIT_FILE: &str = "Edit";
pub const EDIT_NO_FOLDERS: &str = "Folders can't be edited";
pub const EDIT_NO_LINKS: &str = "Links can't be edited";
pub const EDIT_TOO_LARGE: &str = "Files over 50 MB can't be edited";
pub const WAITING_FOR_CONNECTION: &str = "Waiting for connection…";
pub const REMOTE_MAY_BE_TRUNCATED: &str = "The server's copy may be incomplete until then.";
pub const CHANGED_ON_SERVER: &str = "Changed on the server since you opened it";
pub const KEEP_REMOTE: &str = "Keep remote";
pub const SAVE_A_COPY: &str = "Save a copy";
pub const RELOAD_IN_EDITOR: &str = "Now the server's version: reload it in your editor.";
pub const UPLOAD_AND_CLOSE: &str = "Upload pending changes and close";
pub const CLOSE_KEEP_COPIES: &str = "Close and keep local copies";
pub const UPLOAD_AND_QUIT: &str = "Upload and quit";
pub const QUIT_KEEP_COPIES: &str = "Quit and keep local copies";
pub const CHECKING_EDITS: &str = "Checking the remote edits…";
pub const STOP_KEEP_COPIES: &str = "Stop and keep local copies";
/// While "Upload" runs (an upload under way has no bound but its protocol's timeouts).
pub const UPLOADING_EDITS: &str = "Uploading… (Stop to keep local copies instead)";
/// The kept notice, before a remote path: "Server copy of".
pub const SERVER_COPY_OF: &str = "Server copy of";
/// …after it.
pub const MAY_BE_INCOMPLETE: &str = "may be incomplete";
/// The kept notice after a close that did not ask (the site moved, "Close and
/// reconnect"): the editor's file is no longer linked to the server.
pub const KEPT_SAVES_STOPPED: &str = "The session closed: further saves in the editor won't be uploaded. Your changes are in the recovered copy; open the file again to go on editing.";
pub const EDIT_ONE_FILE: &str = "Select one file to edit";
pub const UPLOADED: &str = "Uploaded";
pub const EDITING: &str = "Editing";
pub const OPENING: &str = "Opening";
pub const CLOSE_WORKSPACE_EDITS_TITLE: &str =
    "Some remote edits of this workspace are not uploaded";
pub const KEEPING_COPIES: &str = "Saving the local copies…";
pub const CLOSE_EDITS_TITLE: &str = "Some remote edits are not uploaded";
pub const QUIT_EDITS_TITLE: &str = "Some remote edits are not uploaded";
pub const QUIT_EDITS_HINT: &str = "The window stays open until you choose. Nothing is deleted.";
pub const EDIT_UNSENT: &str = "not uploaded";
pub const EDIT_CONFLICT: &str = "changed on the server";
pub const EDITOR_COMMAND: &str = "Editor";
/// Windows' example names `code.cmd`: the command runs without a shell, which does not
/// find `.cmd` shims from a bare name.
pub const EDITOR_COMMAND_HINT: &str = if cfg!(windows) {
    "system opens remote files in the system default editor. Or type a command; the file's path is added last. For example: notepad++.exe, code.cmd --wait."
} else if cfg!(target_os = "macos") {
    "system opens remote files in the system default editor. Or type a command; the file's path is added last. For example: open -W -a \"Visual Studio Code\" (a full path also works)."
} else {
    "system opens remote files in the system default editor. Or type a command; the file's path is added last. For example: gedit, code --wait."
};
/// Next to the editor field when it holds `system`.
pub const EDITOR_SYSTEM_DEFAULT: &str = "System default";
pub const EDITOR_COMMAND_EMPTY: &str = "Type a command";
pub const SETTINGS_EDITING: &str = "Remote editing";

/// "Upload failed: Permission denied".
pub fn upload_failed(reason: &str) -> String {
    format!("Upload failed: {reason}")
}

/// "Your version was saved to <path>".
pub fn your_version_at(path: &str) -> String {
    format!("Your version was saved to {path}")
}

/// Where "keep" puts the copies: "Unsent changes are copied to <dir>".
pub fn copies_go_to(dir: &str) -> String {
    format!("Unsent changes are copied to {dir}")
}

/// "2 files could not be uploaded".
pub fn not_uploaded(n: usize) -> String {
    if n == 1 {
        "1 file could not be uploaded".to_owned()
    } else {
        format!("{n} files could not be uploaded")
    }
}

/// The persistent notice after a close kept unsent edits: "Kept 2 unsent edits as
/// recovered copies in" (the folder follows, in mono).
pub fn kept_notice(n: usize) -> String {
    if n == 1 {
        "Kept 1 unsent edit as a recovered copy in".to_owned()
    } else {
        format!("Kept {n} unsent edits as recovered copies in")
    }
}

/// The delete confirmation of a site with files open for editing.
pub fn delete_site_edits(n: usize) -> String {
    if n == 1 {
        "1 file of this site is open for editing: its unsent changes will be kept as a recovered copy, never deleted.".to_owned()
    } else {
        format!(
            "{n} files of this site are open for editing: their unsent changes will be kept as recovered copies, never deleted."
        )
    }
}

#[cfg(test)]
mod tests {
    use crate::i18n::{Catalog, Language};

    fn pt(key: &str, args: &[(&str, crate::i18n::Arg)]) -> String {
        Catalog::new(Language::PtBr).tr_args(key, args)
    }

    #[test]
    fn pt_br_site_count_forms() {
        for (n, want) in [(0usize, "0 sites"), (1, "1 site"), (2, "2 sites")] {
            assert_eq!(pt("site-count", &[("count", n.into())]), want);
        }
    }

    #[test]
    fn pt_br_workspace_site_count_partial() {
        let partial = |present: usize, total: usize| {
            pt(
                "workspace-site-count-partial",
                &[("present", present.into()), ("total", total.into())],
            )
        };
        assert_eq!(partial(0, 1), "0 de 1 site");
        assert_eq!(partial(1, 2), "1 de 2 sites");
        assert_eq!(partial(2, 3), "2 de 3 sites");
    }

    #[test]
    fn pt_br_unreadable_name_and_folder_and_shell() {
        assert_eq!(
            pt(
                "unreadable-name-fallback",
                &[("number", 3usize.into()), ("file", "sites.toml".into())]
            ),
            "Entrada 3 em sites.toml"
        );
        assert_eq!(
            pt("cannot-open-folder", &[("reason", "negado".into())]),
            "Não foi possível abrir a pasta: negado"
        );
        assert_eq!(
            pt("shell-exited-code", &[("code", 130u32.into())]),
            "Shell encerrado (código 130)"
        );
        assert_eq!(pt("shell-exited", &[]), "Shell encerrado");
    }

    #[test]
    fn pt_br_log_lines_and_total_workers() {
        for (n, want) in [(0usize, "0 linhas"), (1, "1 linha"), (2, "2 linhas")] {
            assert_eq!(pt("log-lines", &[("count", n.into())]), want);
        }
        for (n, want) in [
            (0usize, "no total, 0 workers"),
            (1, "no total, 1 worker"),
            (4, "no total, 4 workers"),
        ] {
            assert_eq!(pt("total-workers", &[("workers", n.into())]), want);
        }
    }

    #[test]
    fn total_workers_keeps_its_leading_space() {
        assert_eq!(super::total_workers(1), " total, 1 worker");
        assert_eq!(super::total_workers(4), " total, 4 workers");
    }

    #[test]
    fn pt_br_session_policy_and_queue_texts() {
        assert_eq!(
            pt("session-policy", &[("name", "sobrescrever".into())]),
            "Conflitos: sobrescrever (nesta sessão)"
        );
        assert_eq!(
            pt("session-policy-tag", &[("name", "se mais novo".into())]),
            "se mais novo: sessão"
        );
        assert!(
            pt("queue-unavailable", &[("reason", "disco cheio".into())]).contains("disco cheio")
        );
        assert_eq!(
            pt("saved-folder-gone", &[("saved", "/srv".into())]),
            "A pasta salva /srv não existe mais; mostrando a pasta mais próxima."
        );
    }

    #[test]
    fn pt_br_skip_reasons_translate_and_unknown_ones_pass_through() {
        let cat = Catalog::new(Language::PtBr);
        assert_eq!(cat.tr("skip-exists"), "já existe");
        assert_eq!(cat.tr("skip-not-newer"), "não é mais novo");
        assert_eq!(cat.tr("skip-already-complete"), "já está completo");
        assert_eq!(
            pt("skipped", &[("reason", "já existe".into())]),
            "Ignorado: já existe"
        );
        // English process language: known reasons read as before, others unchanged.
        assert_eq!(super::skip_reason("exists"), "exists");
        assert_eq!(super::skip_reason("not newer"), "not newer");
        assert_eq!(super::skip_reason("already complete"), "already complete");
        assert_eq!(super::skip_reason("disk full"), "disk full");
        assert_eq!(super::skipped("disk full"), "Skipped: disk full");
    }

    #[test]
    fn pt_br_status_and_auth_texts() {
        assert_eq!(
            pt(
                "reconnecting",
                &[("attempt", 2u32.into()), ("of", 5u32.into())]
            ),
            "Reconectando (2/5)"
        );
        assert_eq!(
            pt("disconnected", &[("reason", "queda".into())]),
            "Desconectado — queda"
        );
        assert_eq!(
            pt("auth-agent", &[("key_type", "ed25519".into())]),
            "ed25519 via ssh-agent"
        );
        assert_eq!(
            pt("auth-key-file", &[("key_type", "rsa".into())]),
            "chave rsa"
        );
        assert_eq!(
            pt(
                "workspace-status",
                &[
                    ("connected", 2usize.into()),
                    ("sites", "3 sites".into()),
                    ("name", "Loja".into())
                ]
            ),
            "2 de 3 sites em Loja"
        );
    }

    #[test]
    fn pt_br_delete_texts() {
        assert_eq!(
            pt("delete-question-one", &[("name", "a.txt".into())]),
            "Excluir a.txt?"
        );
        assert_eq!(
            pt("delete-question-many", &[("count", 3usize.into())]),
            "Excluir 3 itens?"
        );
        assert_eq!(pt("and-more", &[("count", 4usize.into())]), "e mais 4");
        for (n, want) in [
            (0u64, "Excluindo…"),
            (1, "Excluindo… 1 item"),
            (12, "Excluindo… 12 itens"),
        ] {
            assert_eq!(pt("deleting", &[("done", n.into())]), want);
        }
        let then = |n: u64| {
            pt(
                "deleted-then",
                &[("done", n.into()), ("error", "negado".into())],
            )
        };
        assert_eq!(then(0), "0 itens excluídos; depois: negado");
        assert_eq!(then(1), "1 item excluído; depois: negado");
        assert_eq!(then(2), "2 itens excluídos; depois: negado");
    }

    #[test]
    fn pt_br_size_units_and_months() {
        let cat = Catalog::new(Language::PtBr);
        assert_eq!(cat.tr("size-unit-kb"), "KB");
        let months: Vec<_> = (1..=12).map(|n| cat.tr(&format!("month-{n}"))).collect();
        assert_eq!(
            months.join(" "),
            "jan fev mar abr mai jun jul ago set out nov dez"
        );
    }
}
