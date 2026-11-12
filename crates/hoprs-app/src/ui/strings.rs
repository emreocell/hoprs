//! Every user-visible string, in one place (i18n-ready). Sentence case.

pub const APP_TITLE: &str = "hoprs";
pub const ALREADY_RUNNING: &str = "hoprs is already running";

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

// Center.
pub const NO_SESSION_TITLE: &str = "No session open";
pub const NO_SESSION_HINT: &str = "Pick a site in the sidebar to connect.";

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
