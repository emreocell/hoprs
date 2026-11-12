//! User interaction hooks (host key confirmation, passwords, passphrases).

use std::path::Path;
#[cfg(feature = "testing")]
use std::sync::atomic::{AtomicU32, Ordering};

use async_trait::async_trait;
use secrecy::SecretString;

/// One keyboard-interactive prompt.
#[derive(Debug, Clone)]
pub struct KbdPrompt {
    pub text: String,
    pub echo: bool,
}

/// Asks the user for decisions and secrets. `None` / `false` means cancelled.
#[async_trait]
pub trait Prompter: Send + Sync {
    async fn confirm_host_key(
        &self,
        host: &str,
        port: u16,
        key_type: &str,
        fingerprint: &str,
    ) -> bool;
    async fn password(&self, user: &str, host: &str) -> Option<SecretString>;
    async fn key_passphrase(&self, path: &Path) -> Option<SecretString>;
    async fn keyboard_interactive(
        &self,
        name: &str,
        instructions: &str,
        prompts: &[KbdPrompt],
    ) -> Option<Vec<SecretString>>;
}

/// Number of times each prompt kind was invoked.
#[cfg(feature = "testing")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PromptCalls {
    pub host: u32,
    pub password: u32,
    pub passphrase: u32,
    pub kbd: u32,
}

/// Prompter with canned answers that counts calls. For tests.
#[cfg(feature = "testing")]
pub struct ScriptedPrompter {
    accept_host: bool,
    password: Option<String>,
    passphrase: Option<String>,
    kbd_answers: Option<Vec<String>>,
    host: AtomicU32,
    password_calls: AtomicU32,
    passphrase_calls: AtomicU32,
    kbd: AtomicU32,
}

#[cfg(feature = "testing")]
impl Default for ScriptedPrompter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "testing")]
impl ScriptedPrompter {
    /// Rejects host keys and cancels every other prompt until configured.
    pub fn new() -> Self {
        Self {
            accept_host: false,
            password: None,
            passphrase: None,
            kbd_answers: None,
            host: AtomicU32::new(0),
            password_calls: AtomicU32::new(0),
            passphrase_calls: AtomicU32::new(0),
            kbd: AtomicU32::new(0),
        }
    }

    pub fn accept_host(mut self, accept: bool) -> Self {
        self.accept_host = accept;
        self
    }

    pub fn with_password(mut self, password: Option<&str>) -> Self {
        self.password = password.map(str::to_owned);
        self
    }

    pub fn with_passphrase(mut self, passphrase: Option<&str>) -> Self {
        self.passphrase = passphrase.map(str::to_owned);
        self
    }

    pub fn with_kbd_answers(mut self, answers: Option<Vec<&str>>) -> Self {
        self.kbd_answers = answers.map(|a| a.into_iter().map(str::to_owned).collect());
        self
    }

    pub fn calls(&self) -> PromptCalls {
        PromptCalls {
            host: self.host.load(Ordering::SeqCst),
            password: self.password_calls.load(Ordering::SeqCst),
            passphrase: self.passphrase_calls.load(Ordering::SeqCst),
            kbd: self.kbd.load(Ordering::SeqCst),
        }
    }
}

#[cfg(feature = "testing")]
#[async_trait]
impl Prompter for ScriptedPrompter {
    async fn confirm_host_key(&self, _: &str, _: u16, _: &str, _: &str) -> bool {
        self.host.fetch_add(1, Ordering::SeqCst);
        self.accept_host
    }

    async fn password(&self, _: &str, _: &str) -> Option<SecretString> {
        self.password_calls.fetch_add(1, Ordering::SeqCst);
        self.password.as_deref().map(SecretString::from)
    }

    async fn key_passphrase(&self, _: &Path) -> Option<SecretString> {
        self.passphrase_calls.fetch_add(1, Ordering::SeqCst);
        self.passphrase.as_deref().map(SecretString::from)
    }

    async fn keyboard_interactive(
        &self,
        _: &str,
        _: &str,
        _: &[KbdPrompt],
    ) -> Option<Vec<SecretString>> {
        self.kbd.fetch_add(1, Ordering::SeqCst);
        self.kbd_answers
            .as_ref()
            .map(|a| a.iter().map(|s| SecretString::from(s.as_str())).collect())
    }
}

#[cfg(all(test, feature = "testing"))]
mod tests {
    use super::*;
    use secrecy::ExposeSecret;

    #[tokio::test]
    async fn scripted_prompter_answers_and_counts() {
        let p = ScriptedPrompter::new()
            .accept_host(true)
            .with_password(Some("pw"))
            .with_kbd_answers(Some(vec!["a", "b"]));
        assert!(p.confirm_host_key("h", 22, "ssh-ed25519", "SHA256:x").await);
        assert_eq!(p.password("u", "h").await.unwrap().expose_secret(), "pw");
        assert!(p.key_passphrase(Path::new("/k")).await.is_none());
        let kbd = p.keyboard_interactive("", "", &[]).await.unwrap();
        assert_eq!(kbd.len(), 2);
        assert_eq!(
            p.calls(),
            PromptCalls {
                host: 1,
                password: 1,
                passphrase: 1,
                kbd: 1
            }
        );
    }
}
