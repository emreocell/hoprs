//! SSH connectivity for hoprs: configuration, prompts, host keys, authentication and sessions.

mod auth;
pub mod channels;
pub mod config;
pub mod known_hosts;
pub mod prompt;
mod session;
mod shell;
#[cfg(feature = "testing")]
pub mod testing;
mod watchdog;

pub use auth::{AuthMethod, key_type};
pub use channels::{ChannelSlot, is_channel_limit, sftp_stream_channels};
pub use config::{AuthConfig, KeySource, SshConfig};
pub use hoprs_core::ReconnectPolicy;
pub use prompt::{AcceptedSecret, KbdPrompt, Prompter};
#[cfg(feature = "testing")]
pub use prompt::{HostKeyPrompt, PromptCalls, ScriptedPrompter};
pub use session::{CountedChannel, HostKeyChange, SessionState, SshSession, host_key_change};
pub use shell::{PtyRequest, ShellChannel, ShellHandle, ShellOutput};
