//! "Import from FileZilla", in three steps in the center (no modal): the file, the
//! folders question, then the report, with "Delete sitemanager.xml" behind an inline
//! confirmation.

use std::path::PathBuf;

use hoprs_core::Error;
use hoprs_import::ImportReport;

use super::{App, Effect, Form, PickTarget};
use crate::ui::strings;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportStep {
    /// Which `sitemanager.xml`.
    File,
    /// "Turn FileZilla folders into workspaces?"
    Folders,
    /// The import runs (off the UI thread).
    Running,
    Report,
}

/// "Delete sitemanager.xml" after the report.
#[derive(Debug, Clone, PartialEq)]
pub enum SourceDelete {
    /// Not offered: no report yet, or the import had failures (the file is still the only
    /// copy of what could not be written).
    Unavailable,
    Offered,
    /// Asked in place; nothing is deleted before the confirmation.
    Confirming,
    Deleting,
    Deleted,
    Failed(Error),
}

#[derive(Debug, Clone)]
pub enum ImportMsg {
    Path(String),
    /// "Choose…": the system file dialog.
    Choose,
    Next,
    Back,
    /// The answer to the folders question starts the import.
    Start {
        folders_as_workspaces: bool,
    },
    Delete,
    ConfirmDelete,
    CancelDelete,
    Close,
}

#[derive(Debug)]
pub struct ImportFlow {
    pub step: ImportStep,
    /// The file, as typed or chosen (shown in mono).
    pub path: String,
    pub path_error: Option<&'static str>,
    /// Reading or parsing the file failed: shown at step 1.
    pub error: Option<Error>,
    pub report: Option<ImportReport>,
    /// The file the report came from: the one "Delete" removes.
    pub source: Option<PathBuf>,
    pub delete: SourceDelete,
}

impl ImportFlow {
    /// Step 1, at `default` (FileZilla's usual place), if known.
    pub fn new(default: Option<&PathBuf>) -> Self {
        Self {
            step: ImportStep::File,
            path: default
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default(),
            path_error: None,
            error: None,
            report: None,
            source: None,
            delete: SourceDelete::Unavailable,
        }
    }

    pub fn running(&self) -> bool {
        self.step == ImportStep::Running
    }
}

impl App {
    /// The sidebar's "Import from FileZilla": shows the flow, from the start unless an
    /// import is running.
    pub(super) fn open_import(&mut self) -> Vec<Effect> {
        if !self.import.running() && !matches!(self.form, Some(Form::Import)) {
            self.import = ImportFlow::new(self.filezilla_default.as_ref());
        }
        self.form = Some(Form::Import);
        Vec::new()
    }

    pub(super) fn import_msg(&mut self, msg: ImportMsg) -> Vec<Effect> {
        let flow = &mut self.import;
        match msg {
            ImportMsg::Path(path) => {
                flow.path = path;
                flow.path_error = None;
            }
            ImportMsg::Choose => {
                if flow.step == ImportStep::File {
                    return vec![Effect::PickFile(PickTarget::Sitemanager)];
                }
            }
            ImportMsg::Next => {
                if flow.step == ImportStep::File {
                    if flow.path.trim().is_empty() {
                        flow.path_error = Some(strings::IMPORT_PATH_REQUIRED);
                    } else {
                        flow.error = None;
                        flow.step = ImportStep::Folders;
                    }
                }
            }
            ImportMsg::Back => {
                if flow.step == ImportStep::Folders {
                    flow.step = ImportStep::File;
                }
            }
            ImportMsg::Start {
                folders_as_workspaces,
            } => {
                if flow.step == ImportStep::Folders {
                    let path = PathBuf::from(flow.path.trim());
                    flow.step = ImportStep::Running;
                    flow.source = Some(path.clone());
                    return vec![Effect::ImportFilezilla {
                        path,
                        folders_as_workspaces,
                    }];
                }
            }
            ImportMsg::Delete => {
                if flow.delete == SourceDelete::Offered {
                    flow.delete = SourceDelete::Confirming;
                }
            }
            ImportMsg::CancelDelete => {
                if flow.delete == SourceDelete::Confirming {
                    flow.delete = SourceDelete::Offered;
                }
            }
            ImportMsg::ConfirmDelete => {
                if flow.delete == SourceDelete::Confirming
                    && let Some(source) = flow.source.clone()
                {
                    flow.delete = SourceDelete::Deleting;
                    return vec![Effect::DeleteSitemanager(source)];
                }
            }
            ImportMsg::Close => {
                if matches!(self.form, Some(Form::Import)) {
                    self.form = None;
                }
            }
        }
        Vec::new()
    }

    /// The import finished. Its sites reach the sidebar whatever is shown now.
    pub(super) fn imported(&mut self, result: Result<ImportReport, Error>) -> Vec<Effect> {
        let flow = &mut self.import;
        if !flow.running() {
            return Vec::new();
        }
        match result {
            Ok(report) => {
                flow.delete = if report.failures.is_empty() {
                    SourceDelete::Offered
                } else {
                    SourceDelete::Unavailable
                };
                flow.report = Some(report);
                flow.step = ImportStep::Report;
                vec![Effect::LoadProfiles]
            }
            Err(error) => {
                // Nothing was applied: back to the file, with the reason.
                flow.error = Some(error);
                flow.source = None;
                flow.step = ImportStep::File;
                Vec::new()
            }
        }
    }

    pub(super) fn sitemanager_deleted(&mut self, result: Result<(), Error>) {
        let flow = &mut self.import;
        if flow.delete == SourceDelete::Deleting {
            flow.delete = match result {
                Ok(()) => SourceDelete::Deleted,
                Err(error) => SourceDelete::Failed(error),
            };
        }
    }
}
