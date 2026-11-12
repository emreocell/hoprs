//! User interaction hooks for FTP (passwords, certificate decisions).

#[cfg(feature = "testing")]
use std::sync::Mutex;
#[cfg(feature = "testing")]
use std::sync::atomic::{AtomicU32, Ordering};

use async_trait::async_trait;
use secrecy::SecretString;

/// Why a server certificate needs the user's decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CertProblem {
    /// Not valid for the host under the trusted roots (self-signed, unknown CA, ...).
    Untrusted,
    /// Differs from the certificate the user accepted before.
    Changed { pinned: String },
}

/// Asks the user for decisions and secrets. `None` / `false` means cancelled.
#[async_trait]
pub trait FtpPrompter: Send + Sync {
    async fn password(&self, user: &str, host: &str) -> Option<SecretString>;
    /// Shown with the certificate's SHA-256 fingerprint; `true` accepts (and pins) it.
    async fn confirm_certificate(
        &self,
        host: &str,
        port: u16,
        fingerprint: &str,
        problem: &CertProblem,
    ) -> bool;
    /// Persists an accepted fingerprint for `host:port`.
    async fn store_pin(&self, host: &str, port: u16, fingerprint: &str);
}

/// Number of times each prompt kind was invoked.
#[cfg(feature = "testing")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FtpPromptCalls {
    pub password: u32,
    pub certificate: u32,
}

/// Prompter with canned answers that counts calls and records pins. For tests.
#[cfg(feature = "testing")]
pub struct ScriptedFtpPrompter {
    password: Option<String>,
    accept_certificate: bool,
    password_calls: AtomicU32,
    certificate_calls: AtomicU32,
    pins: Mutex<Vec<(String, u16, String)>>,
    certificates: Mutex<Vec<CertificatePrompt>>,
}

/// The arguments of one `confirm_certificate` call.
#[cfg(feature = "testing")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertificatePrompt {
    pub host: String,
    pub port: u16,
    pub fingerprint: String,
    pub problem: CertProblem,
}

#[cfg(feature = "testing")]
impl Default for ScriptedFtpPrompter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "testing")]
impl ScriptedFtpPrompter {
    /// Cancels the password prompt and rejects certificates until configured.
    pub fn new() -> Self {
        Self {
            password: None,
            accept_certificate: false,
            password_calls: AtomicU32::new(0),
            certificate_calls: AtomicU32::new(0),
            pins: Mutex::new(Vec::new()),
            certificates: Mutex::new(Vec::new()),
        }
    }

    pub fn with_password(mut self, password: Option<&str>) -> Self {
        self.password = password.map(str::to_owned);
        self
    }

    pub fn accept_certificate(mut self, accept: bool) -> Self {
        self.accept_certificate = accept;
        self
    }

    /// Every `store_pin` call, in order: `(host, port, fingerprint)`.
    pub fn pins(&self) -> Vec<(String, u16, String)> {
        self.pins.lock().unwrap().clone()
    }

    /// Every `confirm_certificate` call, in order.
    pub fn certificate_prompts(&self) -> Vec<CertificatePrompt> {
        self.certificates.lock().unwrap().clone()
    }

    pub fn calls(&self) -> FtpPromptCalls {
        FtpPromptCalls {
            password: self.password_calls.load(Ordering::SeqCst),
            certificate: self.certificate_calls.load(Ordering::SeqCst),
        }
    }
}

#[cfg(feature = "testing")]
#[async_trait]
impl FtpPrompter for ScriptedFtpPrompter {
    async fn password(&self, _: &str, _: &str) -> Option<SecretString> {
        self.password_calls.fetch_add(1, Ordering::SeqCst);
        self.password.as_deref().map(SecretString::from)
    }

    async fn confirm_certificate(
        &self,
        host: &str,
        port: u16,
        fingerprint: &str,
        problem: &CertProblem,
    ) -> bool {
        self.certificate_calls.fetch_add(1, Ordering::SeqCst);
        self.certificates.lock().unwrap().push(CertificatePrompt {
            host: host.to_owned(),
            port,
            fingerprint: fingerprint.to_owned(),
            problem: problem.clone(),
        });
        self.accept_certificate
    }

    async fn store_pin(&self, host: &str, port: u16, fingerprint: &str) {
        self.pins
            .lock()
            .unwrap()
            .push((host.to_owned(), port, fingerprint.to_owned()));
    }
}

#[cfg(all(test, feature = "testing"))]
mod tests {
    use super::*;
    use secrecy::ExposeSecret;

    #[tokio::test]
    async fn scripted_prompter_answers_counts_and_pins() {
        let p = ScriptedFtpPrompter::new()
            .with_password(Some("pw"))
            .accept_certificate(true);
        assert_eq!(p.password("u", "h").await.unwrap().expose_secret(), "pw");
        assert!(
            p.confirm_certificate("h", 21, "SHA256:AA", &CertProblem::Untrusted)
                .await
        );
        p.store_pin("h", 21, "SHA256:AA").await;
        assert_eq!(p.pins(), vec![("h".to_owned(), 21, "SHA256:AA".to_owned())]);
        assert_eq!(
            p.certificate_prompts(),
            vec![CertificatePrompt {
                host: "h".to_owned(),
                port: 21,
                fingerprint: "SHA256:AA".to_owned(),
                problem: CertProblem::Untrusted,
            }]
        );
        assert_eq!(
            p.calls(),
            FtpPromptCalls {
                password: 1,
                certificate: 1
            }
        );
        assert!(
            ScriptedFtpPrompter::new()
                .password("u", "h")
                .await
                .is_none()
        );
    }
}
