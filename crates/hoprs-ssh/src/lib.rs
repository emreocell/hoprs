//! SSH connectivity for hoprs: configuration, prompts and reconnect policy.

pub mod config;
pub mod known_hosts;
pub mod prompt;

pub use config::{AuthConfig, KeySource, ReconnectPolicy, SshConfig};
pub use prompt::{KbdPrompt, Prompter};
#[cfg(feature = "testing")]
pub use prompt::{PromptCalls, ScriptedPrompter};
