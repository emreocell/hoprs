//! SSH connectivity for hoprs: configuration, prompts, host keys, authentication and sessions.

mod auth;
pub mod config;
pub mod known_hosts;
pub mod prompt;
mod session;
mod shell;
#[cfg(feature = "testing")]
pub mod testing;
mod watchdog;

pub use config::{AuthConfig, KeySource, SshConfig};
pub use hoprs_core::ReconnectPolicy;
pub use prompt::{AcceptedSecret, KbdPrompt, Prompter};
#[cfg(feature = "testing")]
pub use prompt::{HostKeyPrompt, PromptCalls, ScriptedPrompter};
pub use session::{HostKeyChange, SessionState, SshSession, host_key_change};
pub use shell::{PtyRequest, ShellChannel, ShellOutput};
