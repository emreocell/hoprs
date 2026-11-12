//! Importers for other clients' saved sites. Pure parsing: nothing here
//! touches the vault, the keyring or the network.

mod filezilla;

pub use filezilla::{
    ParsedEntry, ParsedPassword, ParsedSite, ParsedSiteManager, default_sitemanager_path,
    parse_sitemanager, read_sitemanager,
};
