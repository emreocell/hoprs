//! SSH connectivity for hoprs: configuration, prompts, host keys, authentication and sessions.

mod auth;
pub mod config;
pub mod known_hosts;
pub mod prompt;
mod session;
#[cfg(feature = "testing")]
pub mod testing;
mod watchdog;

pub use config::{AuthConfig, KeySource, ReconnectPolicy, SshConfig};
pub use prompt::{KbdPrompt, Prompter};
#[cfg(feature = "testing")]
pub use prompt::{PromptCalls, ScriptedPrompter};
pub use session::SshSession;
