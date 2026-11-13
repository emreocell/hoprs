//! The settings page (center, no modal): workers per site, the default conflict policy,
//! the keepalive, the terminal, and the encrypted backup (export and restore).
//!
//! A change applies at once (the policy to the engine, the workers and the keepalive to
//! the next connections) and is saved to `settings.toml`, unless that file was unreadable at start:
//! then nothing is saved over it.
//!
//! Backup passwords live in the page only while typed: they leave it inside the effect
//! (a [`SecretString`]) and the fields are emptied.

use std::path::PathBuf;

use hoprs_core::Error;
use hoprs_transfer::ConflictPolicy;
use hoprs_vault::{BackupImportReport, ExportReport, ImportMode};
use secrecy::SecretString;

use super::{App, Effect, Form, PickTarget};
use crate::settings::{
    SettingsLoad, TerminalTheme, clamp_font_size, clamp_keepalive, clamp_scrollback, clamp_workers,
};
use crate::ui::site_form::Password;
use crate::ui::strings;

#[derive(Debug, Clone)]
pub enum SettingsMsg {
    Workers(usize),
    Policy(ConflictPolicy),
    /// The terminal's font size (clamped to `10..=24`); applies at once.
    TerminalFontSize(u16),
    /// Scrollback lines (clamped to `1 000..=100 000`), for the terminals opened next.
    TerminalScrollback(usize),
    TerminalTheme(TerminalTheme),
    /// Seconds between keepalives (clamped to `10..=300`), for the next connections.
    Keepalive(u64),
    Backup(BackupMsg),
    Close,
}

#[derive(Debug, Clone)]
pub enum BackupMsg {
    OpenExport,
    OpenRestore,
    /// Closes the export or restore panel.
    Cancel,
    /// "Choose…" for the export or the restore file.
    Choose(PickTarget),
    ExportPath(String),
    /// A typed password: its `Debug` never shows it.
    ExportPassword(Password),
    ExportConfirm(Password),
    Export,
    /// "Overwrite" on the inline question about an existing export file.
    ConfirmOverwrite,
    CancelOverwrite,
    RestorePath(String),
    RestorePassword(Password),
    RestoreMode(ImportMode),
    Restore,
    ConfirmReplace,
    CancelReplace,
}

/// Which backup panel is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackupPanel {
    Closed,
    Export,
    Restore,
}

/// An export or a restore: idle, running (indeterminate progress), or its outcome.
#[derive(Debug, Clone, PartialEq)]
pub enum BackupState<R> {
    Idle,
    Running,
    Done(R),
    Failed(Error),
}

impl<R> BackupState<R> {
    pub fn running(&self) -> bool {
        matches!(self, Self::Running)
    }
}

#[derive(Debug)]
pub struct Backup {
    pub panel: BackupPanel,
    pub export_path: String,
    pub export_password: Password,
    pub export_confirm: Password,
    pub restore_path: String,
    pub restore_password: Password,
    pub restore_mode: ImportMode,
    /// "Replace" asked once and waits for its confirmation.
    pub confirm_replace: bool,
    /// The export path came from the save dialog (which already asked about overwriting).
    pub export_path_picked: bool,
    /// Whether the typed export file exists is being checked.
    pub export_checking: bool,
    /// The typed export file exists: "Overwrite?" waits for its answer.
    pub confirm_overwrite: bool,
    /// What is wrong with the fields of the open panel.
    pub field_error: Option<&'static str>,
    pub export: BackupState<ExportReport>,
    pub restore: BackupState<BackupImportReport>,
}

impl Default for Backup {
    fn default() -> Self {
        Self {
            panel: BackupPanel::Closed,
            export_path: String::new(),
            export_password: Password::default(),
            export_confirm: Password::default(),
            restore_path: String::new(),
            restore_password: Password::default(),
            restore_mode: ImportMode::Merge,
            confirm_replace: false,
            export_path_picked: false,
            export_checking: false,
            confirm_overwrite: false,
            field_error: None,
            export: BackupState::Idle,
            restore: BackupState::Idle,
        }
    }
}

impl Backup {
    /// An export or a restore runs (both lock the vault; one at a time).
    pub fn running(&self) -> bool {
        self.export.running() || self.restore.running()
    }

    /// Empties every password field, and drops the questions that would send one.
    pub(super) fn forget_passwords(&mut self) {
        self.export_password = Password::default();
        self.export_confirm = Password::default();
        self.restore_password = Password::default();
        self.confirm_replace = false;
        self.confirm_overwrite = false;
        self.export_checking = false;
    }
}

/// The path typed in a field, or the reason it can't be used.
fn path_of(field: &str) -> Result<PathBuf, &'static str> {
    let trimmed = field.trim();
    if trimmed.is_empty() {
        Err(strings::BACKUP_PATH_REQUIRED)
    } else {
        Ok(PathBuf::from(trimmed))
    }
}

impl App {
    pub(super) fn open_settings(&mut self) -> Vec<Effect> {
        self.form = Some(Form::Settings);
        Vec::new()
    }

    pub(super) fn settings_loaded(&mut self, loaded: SettingsLoad) {
        self.settings = loaded.settings;
        self.settings_load_error = loaded.error;
    }

    pub(super) fn settings_msg(&mut self, msg: SettingsMsg) -> Vec<Effect> {
        match msg {
            SettingsMsg::Workers(workers) => {
                let workers = clamp_workers(workers);
                if workers == self.settings.workers_per_site {
                    return Vec::new();
                }
                self.settings.workers_per_site = workers;
                let mut effects = vec![Effect::SetWorkers(workers)];
                effects.extend(self.save_settings_effect());
                effects
            }
            SettingsMsg::Policy(policy) => {
                if policy == self.settings.default_policy {
                    return Vec::new();
                }
                self.settings.default_policy = policy;
                // The engine runs under this policy from now on, whatever an answer "to
                // all" set before.
                self.queue.set_session_policy(None);
                let mut effects = vec![Effect::SetPolicy(policy)];
                effects.extend(self.save_settings_effect());
                effects
            }
            SettingsMsg::TerminalFontSize(size) => {
                let size = clamp_font_size(size);
                if size == self.settings.terminal_font_size {
                    return Vec::new();
                }
                self.settings.terminal_font_size = size;
                self.save_settings_effect().into_iter().collect()
            }
            SettingsMsg::TerminalScrollback(lines) => {
                let lines = clamp_scrollback(lines);
                if lines == self.settings.terminal_scrollback {
                    return Vec::new();
                }
                self.settings.terminal_scrollback = lines;
                self.save_settings_effect().into_iter().collect()
            }
            SettingsMsg::TerminalTheme(theme) => {
                if theme == self.settings.terminal_theme {
                    return Vec::new();
                }
                self.settings.terminal_theme = theme;
                for tab in &mut self.tabs {
                    if let Some(terminal) = tab.terminal_mut() {
                        terminal.model.set_theme(theme.colors());
                    }
                }
                self.save_settings_effect().into_iter().collect()
            }
            SettingsMsg::Keepalive(secs) => {
                let secs = clamp_keepalive(secs);
                if secs == self.settings.keepalive_secs {
                    return Vec::new();
                }
                self.settings.keepalive_secs = secs;
                let mut effects = vec![Effect::SetKeepalive(secs)];
                effects.extend(self.save_settings_effect());
                effects
            }
            SettingsMsg::Backup(msg) => self.backup_msg(msg),
            SettingsMsg::Close => {
                if matches!(self.form, Some(Form::Settings)) {
                    self.form = None;
                }
                // Typed passwords don't outlive the page.
                self.backup.forget_passwords();
                Vec::new()
            }
        }
    }

    /// Saves the settings, unless the file was unreadable: it is never overwritten. One
    /// save at a time: a change made while one runs is saved when it ends, so the file
    /// always ends up with the latest settings (two writes never race).
    fn save_settings_effect(&mut self) -> Option<Effect> {
        if self.settings_load_error.is_some() {
            return None;
        }
        if self.settings_saving {
            self.settings_dirty = true;
            return None;
        }
        self.settings_saving = true;
        Some(Effect::SaveSettings(self.settings))
    }

    pub(super) fn settings_saved(&mut self, result: Result<(), Error>) -> Vec<Effect> {
        self.settings_saving = false;
        self.settings_save_error = result.err();
        if std::mem::take(&mut self.settings_dirty) {
            return self.save_settings_effect().into_iter().collect();
        }
        Vec::new()
    }

    fn backup_msg(&mut self, msg: BackupMsg) -> Vec<Effect> {
        let backup = &mut self.backup;
        match msg {
            BackupMsg::OpenExport | BackupMsg::OpenRestore => {
                let panel = if matches!(msg, BackupMsg::OpenExport) {
                    BackupPanel::Export
                } else {
                    BackupPanel::Restore
                };
                if backup.panel != panel {
                    // A password typed for one panel never goes to the other.
                    backup.forget_passwords();
                }
                backup.panel = panel;
                backup.field_error = None;
            }
            BackupMsg::Cancel => {
                backup.panel = BackupPanel::Closed;
                backup.field_error = None;
                backup.forget_passwords();
            }
            BackupMsg::Choose(target) => {
                if matches!(target, PickTarget::ExportBackup | PickTarget::RestoreBackup) {
                    return vec![Effect::PickFile(target)];
                }
            }
            BackupMsg::ExportPath(path) => {
                backup.export_path = path;
                backup.export_path_picked = false;
                backup.export_checking = false;
                backup.confirm_overwrite = false;
            }
            BackupMsg::ExportPassword(password) => backup.export_password = password,
            BackupMsg::ExportConfirm(password) => backup.export_confirm = password,
            BackupMsg::Export => {
                if backup.running() || backup.export_checking || backup.confirm_overwrite {
                    return Vec::new();
                }
                let path = match path_of(&backup.export_path) {
                    Ok(path) => path,
                    Err(why) => {
                        backup.field_error = Some(why);
                        return Vec::new();
                    }
                };
                if backup.export_password.0.is_empty() {
                    backup.field_error = Some(strings::BACKUP_PASSWORD_REQUIRED);
                    return Vec::new();
                }
                if backup.export_password != backup.export_confirm {
                    backup.field_error = Some(strings::BACKUP_PASSWORDS_DIFFER);
                    return Vec::new();
                }
                backup.field_error = None;
                if backup.export_path_picked {
                    return self.start_export();
                }
                // A typed path: an existing file is overwritten only once confirmed.
                backup.export_checking = true;
                return vec![Effect::CheckExportTarget(path)];
            }
            BackupMsg::ConfirmOverwrite => {
                if backup.confirm_overwrite {
                    backup.confirm_overwrite = false;
                    return self.start_export();
                }
            }
            BackupMsg::CancelOverwrite => backup.confirm_overwrite = false,
            BackupMsg::RestorePath(path) => {
                backup.restore_path = path;
                backup.confirm_replace = false;
            }
            BackupMsg::RestorePassword(password) => backup.restore_password = password,
            BackupMsg::RestoreMode(mode) => {
                backup.restore_mode = mode;
                backup.confirm_replace = false;
            }
            BackupMsg::Restore => {
                if backup.running() {
                    return Vec::new();
                }
                if let Err(why) = path_of(&backup.restore_path) {
                    backup.field_error = Some(why);
                    return Vec::new();
                }
                if backup.restore_password.0.is_empty() {
                    backup.field_error = Some(strings::BACKUP_PASSWORD_REQUIRED);
                    return Vec::new();
                }
                backup.field_error = None;
                if backup.restore_mode == ImportMode::Replace {
                    // Replace drops everything not in the backup: asked once more.
                    backup.confirm_replace = true;
                    return Vec::new();
                }
                return self.start_restore();
            }
            BackupMsg::ConfirmReplace => {
                if backup.confirm_replace && backup.restore_mode == ImportMode::Replace {
                    backup.confirm_replace = false;
                    return self.start_restore();
                }
            }
            BackupMsg::CancelReplace => backup.confirm_replace = false,
        }
        Vec::new()
    }

    /// Whether the typed export file exists came back.
    pub(super) fn export_target_checked(&mut self, path: PathBuf, exists: bool) -> Vec<Effect> {
        let backup = &mut self.backup;
        // A path edited meanwhile (or the panel left) drops this answer.
        if !backup.export_checking || path_of(&backup.export_path).ok() != Some(path) {
            return Vec::new();
        }
        backup.export_checking = false;
        if exists {
            backup.confirm_overwrite = true;
            return Vec::new();
        }
        self.start_export()
    }

    /// Sends the export with the typed password, which leaves the page.
    fn start_export(&mut self) -> Vec<Effect> {
        let backup = &mut self.backup;
        let Ok(path) = path_of(&backup.export_path) else {
            return Vec::new();
        };
        if backup.running()
            || backup.export_password.0.is_empty()
            || backup.export_password != backup.export_confirm
        {
            return Vec::new();
        }
        // From here on only the effect holds the password.
        let password = SecretString::from(std::mem::take(&mut backup.export_password.0));
        backup.export_confirm = Password::default();
        backup.export = BackupState::Running;
        vec![Effect::ExportBackup { path, password }]
    }

    /// Sends the restore with the typed password, which leaves the page.
    fn start_restore(&mut self) -> Vec<Effect> {
        let backup = &mut self.backup;
        let Ok(path) = path_of(&backup.restore_path) else {
            return Vec::new();
        };
        if backup.running() || backup.restore_password.0.is_empty() {
            return Vec::new();
        }
        let password = SecretString::from(std::mem::take(&mut backup.restore_password.0));
        backup.restore = BackupState::Running;
        vec![Effect::RestoreBackup {
            path,
            password,
            mode: backup.restore_mode,
        }]
    }

    pub(super) fn backup_exported(&mut self, result: Result<ExportReport, Error>) {
        if self.backup.export.running() {
            self.backup.export = match result {
                Ok(report) => BackupState::Done(report),
                Err(error) => BackupState::Failed(error),
            };
        }
    }

    /// A restore finished; what it changed reaches the sidebar.
    pub(super) fn backup_restored(
        &mut self,
        result: Result<BackupImportReport, Error>,
    ) -> Vec<Effect> {
        if !self.backup.restore.running() {
            return Vec::new();
        }
        match result {
            Ok(report) => {
                self.backup.restore = BackupState::Done(report);
                vec![self.load_profiles()]
            }
            Err(error) => {
                self.backup.restore = BackupState::Failed(error);
                Vec::new()
            }
        }
    }

    /// The system file dialog answered (`None`: cancelled).
    pub(super) fn file_picked(&mut self, target: PickTarget, path: Option<PathBuf>) {
        let Some(path) = path else {
            return;
        };
        let path = path.to_string_lossy().into_owned();
        match target {
            PickTarget::Sitemanager => {
                self.import.path = path;
                self.import.path_error = None;
            }
            PickTarget::ExportBackup => {
                self.backup.export_path = path;
                // The save dialog asked about an existing file already.
                self.backup.export_path_picked = true;
                self.backup.export_checking = false;
                self.backup.confirm_overwrite = false;
            }
            PickTarget::RestoreBackup => {
                self.backup.restore_path = path;
                self.backup.confirm_replace = false;
            }
        }
    }

    /// An import or a backup runs: the progress indicator moves.
    pub fn busy(&self) -> bool {
        self.import.running() || self.backup.running()
    }
}
