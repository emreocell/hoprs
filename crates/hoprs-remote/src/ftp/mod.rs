//! FTP and FTPS backend.

mod config;
// Used by the testing helpers until `FtpFs` (Plan 3, Task 5) uses them.
#[cfg_attr(not(feature = "testing"), allow(dead_code))]
pub(crate) mod errors;
mod prompt;
#[cfg_attr(not(feature = "testing"), allow(dead_code))]
pub(crate) mod session;
#[cfg_attr(not(feature = "testing"), allow(dead_code))]
mod tls;

pub use config::{FtpConfig, FtpSecurity, TransferMode};
pub use prompt::{CertProblem, FtpPrompter};
#[cfg(feature = "testing")]
pub use prompt::{CertificatePrompt, FtpPromptCalls, ScriptedFtpPrompter};
pub use tls::fingerprint;
