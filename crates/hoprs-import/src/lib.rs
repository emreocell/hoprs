//! Importers for other clients' saved sites. Parsing is pure (nothing
//! touches the vault, the keyring or the network); [`apply`] writes a parsed
//! import into a vault.

mod apply;
mod filezilla;

pub use apply::{ImportOptions, ImportReport, ImportedSite, SkippedSite, apply, delete_source};

pub use filezilla::{
    ParsedEntry, ParsedPassword, ParsedSite, ParsedSiteManager, default_sitemanager_path,
    parse_sitemanager, read_sitemanager,
};
