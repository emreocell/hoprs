//! Prompt-aware connect timeout.
//!
//! A connection attempt (TCP, version exchange, key exchange, authentication) must not hang
//! on a silent server, but it must also not time out while the user is reading a host key
//! fingerprint or typing a password. [`WatchedPrompter`] marks prompts in flight; [`expired`]
//! counts `timeout` of network time only, restarting the deadline after every prompt.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use secrecy::SecretString;
use tokio::sync::Notify;

use crate::prompt::{KbdPrompt, Prompter};

/// Prompter decorator that records when a prompt is waiting for the user.
pub(crate) struct WatchedPrompter {
    inner: Arc<dyn Prompter>,
    in_flight: AtomicUsize,
    changed: Notify,
}

impl WatchedPrompter {
    pub(crate) fn new(inner: Arc<dyn Prompter>) -> Self {
        Self {
            inner,
            in_flight: AtomicUsize::new(0),
            changed: Notify::new(),
        }
    }

    async fn watch<T>(&self, prompt: impl Future<Output = T>) -> T {
        self.in_flight.fetch_add(1, Ordering::SeqCst);
        self.changed.notify_waiters();
        let _done = Done(self);
        prompt.await
    }
}

/// Ends a prompt even if its future is dropped mid-way.
struct Done<'a>(&'a WatchedPrompter);

impl Drop for Done<'_> {
    fn drop(&mut self) {
        self.0.in_flight.fetch_sub(1, Ordering::SeqCst);
        self.0.changed.notify_waiters();
    }
}

#[async_trait]
impl Prompter for WatchedPrompter {
    async fn confirm_host_key(
        &self,
        host: &str,
        port: u16,
        key_type: &str,
        fingerprint: &str,
    ) -> bool {
        self.watch(
            self.inner
                .confirm_host_key(host, port, key_type, fingerprint),
        )
        .await
    }

    async fn password(&self, user: &str, host: &str) -> Option<SecretString> {
        self.watch(self.inner.password(user, host)).await
    }

    async fn key_passphrase(&self, path: &Path) -> Option<SecretString> {
        self.watch(self.inner.key_passphrase(path)).await
    }

    async fn keyboard_interactive(
        &self,
        name: &str,
        instructions: &str,
        prompts: &[KbdPrompt],
    ) -> Option<Vec<SecretString>> {
        self.watch(self.inner.keyboard_interactive(name, instructions, prompts))
            .await
    }
}

/// Resolves once `timeout` passes with no prompt in flight. Each prompt start or end
/// restarts the deadline; while a prompt is open the clock is stopped.
pub(crate) async fn expired(prompter: &WatchedPrompter, timeout: Duration) {
    loop {
        let changed = prompter.changed.notified();
        tokio::pin!(changed);
        // Register before reading the counter so no start/end notification is missed.
        changed.as_mut().enable();
        if prompter.in_flight.load(Ordering::SeqCst) == 0 {
            tokio::select! {
                _ = tokio::time::sleep(timeout) => return,
                _ = changed => {}
            }
        } else {
            changed.await;
        }
    }
}

#[cfg(all(test, feature = "testing"))]
mod tests {
    use super::*;
    use crate::prompt::ScriptedPrompter;

    #[tokio::test(start_paused = true)]
    async fn expires_without_prompts() {
        let p = WatchedPrompter::new(Arc::new(ScriptedPrompter::new()));
        let start = tokio::time::Instant::now();
        expired(&p, Duration::from_secs(2)).await;
        assert_eq!(start.elapsed(), Duration::from_secs(2));
    }

    #[tokio::test(start_paused = true)]
    async fn prompt_stops_the_clock() {
        struct Slow;
        #[async_trait]
        impl Prompter for Slow {
            async fn confirm_host_key(&self, _: &str, _: u16, _: &str, _: &str) -> bool {
                tokio::time::sleep(Duration::from_secs(10)).await;
                true
            }
            async fn password(&self, _: &str, _: &str) -> Option<SecretString> {
                None
            }
            async fn key_passphrase(&self, _: &Path) -> Option<SecretString> {
                None
            }
            async fn keyboard_interactive(
                &self,
                _: &str,
                _: &str,
                _: &[KbdPrompt],
            ) -> Option<Vec<SecretString>> {
                None
            }
        }
        let p = WatchedPrompter::new(Arc::new(Slow));
        let start = tokio::time::Instant::now();
        let work = async {
            tokio::time::sleep(Duration::from_secs(1)).await;
            p.confirm_host_key("h", 22, "ssh-ed25519", "SHA256:x").await;
        };
        tokio::select! {
            _ = work => {}
            _ = expired(&p, Duration::from_secs(2)) => panic!("expired during a prompt"),
        }
        // 1 s of network time, 10 s prompt; the deadline then restarts at 2 s.
        expired(&p, Duration::from_secs(2)).await;
        assert_eq!(start.elapsed(), Duration::from_secs(13));
    }
}
