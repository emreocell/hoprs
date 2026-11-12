//! User interaction hooks for FTP (passwords, certificate decisions).

#[cfg(feature = "testing")]
use std::sync::Mutex;
#[cfg(feature = "testing")]
use std::sync::atomic::{AtomicU32, Ordering};

use async_trait::async_trait;
use secrecy::SecretString;

/// Why the certificate chain check refused a server certificate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CertReason {
    /// Past its validity period.
    Expired,
    /// Before its validity period.
    NotYetValid,
    /// Not issued for the host name or IP address connected to.
    WrongName,
    /// Does not chain to a trusted root (self-signed, unknown authority).
    UnknownIssuer,
    /// Anything else, as described by the TLS library.
    Other(String),
}

/// Why a server certificate needs the user's decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CertProblem {
    /// Not valid for the host under the trusted roots, for `reason`.
    Untrusted { reason: CertReason },
    /// Differs from the certificate the user accepted before (`pinned`), and is not valid
    /// under the trusted roots either, for `reason`.
    Changed { pinned: String, reason: CertReason },
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
    /// The certificate of `host:port` chains to a trusted root, so it was accepted although
    /// `pinned` (the configured pin, for another certificate) did not match: the pin is
    /// stale and may be forgotten. The connection no longer uses it. Does nothing by default.
    async fn pin_superseded(&self, host: &str, port: u16, pinned: &str) {
        let _ = (host, port, pinned);
    }
    /// The server logged in with the password returned by [`password`](Self::password).
    /// A typed password never reported here was refused (or never tried), so it must not
    /// be kept. May be reported again for later connections of the same session. Does
    /// nothing by default.
    fn password_accepted(&self) {}
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
    superseded: Mutex<Vec<(String, u16, String)>>,
    certificates: Mutex<Vec<CertificatePrompt>>,
    accepted: AtomicU32,
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
            superseded: Mutex::new(Vec::new()),
            certificates: Mutex::new(Vec::new()),
            accepted: AtomicU32::new(0),
        }
    }

    /// How many times `password_accepted` was called.
    pub fn password_accepted_calls(&self) -> u32 {
        self.accepted.load(Ordering::SeqCst)
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

    /// Every `pin_superseded` call, in order: `(host, port, stale pin)`.
    pub fn superseded_pins(&self) -> Vec<(String, u16, String)> {
        self.superseded.lock().unwrap().clone()
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

    async fn pin_superseded(&self, host: &str, port: u16, pinned: &str) {
        self.superseded
            .lock()
            .unwrap()
            .push((host.to_owned(), port, pinned.to_owned()));
    }

    fn password_accepted(&self) {
        self.accepted.fetch_add(1, Ordering::SeqCst);
    }
}

#[cfg(all(test, feature = "testing"))]
mod tests {
    use super::*;
    use secrecy::ExposeSecret;

    fn untrusted() -> CertProblem {
        CertProblem::Untrusted {
            reason: CertReason::UnknownIssuer,
        }
    }

    #[tokio::test]
    async fn scripted_prompter_answers_counts_and_pins() {
        let p = ScriptedFtpPrompter::new()
            .with_password(Some("pw"))
            .accept_certificate(true);
        assert_eq!(p.password("u", "h").await.unwrap().expose_secret(), "pw");
        assert!(
            p.confirm_certificate("h", 21, "SHA256:AA", &untrusted())
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
                problem: untrusted(),
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
