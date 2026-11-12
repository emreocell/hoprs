//! FTPS certificate verification.
//!
//! A server certificate is accepted when it chains to a trusted root (the system roots plus
//! [`FtpConfig::extra_roots`]) and is valid for the host name or IP, or, failing that, when
//! its SHA-256 fingerprint equals [`FtpConfig::pinned_fingerprint`]. Anything else is
//! rejected, and the certificate and the reason are left in a [`RejectedSlot`] so the caller
//! can ask the user. Handshake signatures are always verified with the aws-lc-rs provider.

use std::fmt;
use std::sync::{Arc, Mutex, OnceLock};

use hoprs_core::{Error, ErrorKind, Result};
use rustls::client::WebPkiServerVerifier;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{
    CertificateError, ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme,
};
use sha2::{Digest, Sha256};
use suppaftp::tokio::AsyncRustlsConnector;
use suppaftp::tokio_rustls::TlsConnector;

use super::config::FtpConfig;
use super::prompt::CertProblem;

/// A certificate the verifier refused, and why.
#[derive(Debug, Clone)]
pub(crate) struct RejectedCert {
    pub(crate) der: CertificateDer<'static>,
    pub(crate) problem: CertProblem,
}

/// Where the verifier leaves the last certificate it refused.
pub(crate) type RejectedSlot = Arc<Mutex<Option<RejectedCert>>>;

/// `SHA256:` followed by the certificate's SHA-256 digest in uppercase hex pairs separated
/// by `:`, the format of `openssl x509 -noout -fingerprint -sha256`.
pub fn fingerprint(cert_der: &[u8]) -> String {
    let hex: Vec<String> = Sha256::digest(cert_der)
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect();
    format!("SHA256:{}", hex.join(":"))
}

/// System trust anchors, loaded once per process. Certificates the platform store cannot
/// load are skipped; if none load, only [`FtpConfig::extra_roots`] (and pins) are trusted.
fn system_roots() -> &'static [CertificateDer<'static>] {
    static ROOTS: OnceLock<Vec<CertificateDer<'static>>> = OnceLock::new();
    ROOTS.get_or_init(|| rustls_native_certs::load_native_certs().certs)
}

/// Verifier for `cfg`: system roots plus `cfg.extra_roots`, then `cfg.pinned_fingerprint`.
/// An `extra_roots` file that cannot be read or holds no certificate is a `Tls` error.
pub(crate) fn verifier(cfg: &FtpConfig, slot: RejectedSlot) -> Result<Arc<PinningVerifier>> {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let mut roots = RootCertStore::empty();
    // Unparsable system certificates are skipped, like every TLS client does.
    roots.add_parsable_certificates(system_roots().iter().cloned());
    for path in &cfg.extra_roots {
        let bad_root = |detail: String| {
            Error::new(
                ErrorKind::Tls,
                format!("Cannot use trusted certificate {}", path.display()),
            )
            .with_detail(detail)
        };
        let certs = CertificateDer::pem_file_iter(path)
            .and_then(|it| it.collect::<std::result::Result<Vec<_>, _>>())
            .map_err(|e| bad_root(e.to_string()))?;
        if certs.is_empty() {
            return Err(bad_root("no certificate in the file".into()));
        }
        for cert in certs {
            roots.add(cert).map_err(|e| bad_root(e.to_string()))?;
        }
    }
    // With no root at all (no system store, no extra roots) only the pin can accept.
    let webpki = if roots.is_empty() {
        None
    } else {
        let v = WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider.clone())
            .build()
            .map_err(|e| {
                Error::new(ErrorKind::Tls, "Cannot build the certificate verifier")
                    .with_detail(e.to_string())
            })?;
        Some(v)
    };
    Ok(Arc::new(PinningVerifier {
        webpki,
        pinned: cfg.pinned_fingerprint.clone(),
        provider,
        slot,
    }))
}

/// See the module docs.
pub(crate) struct PinningVerifier {
    /// `None` when there is no trusted root at all.
    webpki: Option<Arc<WebPkiServerVerifier>>,
    pinned: Option<String>,
    provider: Arc<CryptoProvider>,
    slot: RejectedSlot,
}

impl fmt::Debug for PinningVerifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PinningVerifier")
            .field("has_roots", &self.webpki.is_some())
            .field("pinned", &self.pinned)
            .finish_non_exhaustive()
    }
}

impl ServerCertVerifier for PinningVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        let chain_error = match &self.webpki {
            Some(v) => {
                match v.verify_server_cert(
                    end_entity,
                    intermediates,
                    server_name,
                    ocsp_response,
                    now,
                ) {
                    Ok(ok) => return Ok(ok),
                    Err(e) => e,
                }
            }
            None => rustls::Error::InvalidCertificate(CertificateError::UnknownIssuer),
        };
        let fp = fingerprint(end_entity);
        let problem = match &self.pinned {
            Some(pinned) if pinned.eq_ignore_ascii_case(&fp) => {
                return Ok(ServerCertVerified::assertion());
            }
            Some(pinned) => CertProblem::Changed {
                pinned: pinned.clone(),
            },
            None => CertProblem::Untrusted,
        };
        *self.slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(RejectedCert {
            der: end_entity.clone().into_owned(),
            problem,
        });
        Err(chain_error)
    }

    // Handshake signatures are always checked, pinned certificate or not: the pin only
    // replaces the chain check, never the proof that the server holds the key.
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// A TLS connector for `cfg`'s control and data connections. Both use the same verifier, so
/// a data connection cannot present a certificate the control connection would refuse. Data
/// connections may resume the control connection's TLS session (rustls' default in-memory
/// session cache, per connector), which servers with `require_ssl_reuse` expect.
pub(crate) fn connector(cfg: &FtpConfig, slot: RejectedSlot) -> Result<AsyncRustlsConnector> {
    let verifier = verifier(cfg, slot)?;
    let config = ClientConfig::builder_with_provider(verifier.provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| Error::new(ErrorKind::Tls, "Cannot configure TLS").with_detail(e.to_string()))?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    Ok(AsyncRustlsConnector::from(TlsConnector::from(Arc::new(
        config,
    ))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ftp::FtpSecurity;
    use std::path::PathBuf;

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../docker/fixtures/ftps")
            .join(name)
    }

    fn cert(name: &str) -> CertificateDer<'static> {
        CertificateDer::from_pem_file(fixture(name)).unwrap()
    }

    fn server_fp() -> String {
        let hex = std::fs::read_to_string(fixture("server.fingerprint")).unwrap();
        format!("SHA256:{}", hex.trim())
    }

    fn cfg(extra_roots: &[&str], pin: Option<String>) -> FtpConfig {
        let mut c = FtpConfig::new("127.0.0.1", 21, "u", FtpSecurity::ExplicitTls);
        c.extra_roots = extra_roots.iter().map(|n| fixture(n)).collect();
        c.pinned_fingerprint = pin;
        c
    }

    /// Runs the verifier on `leaf` for `host`; returns whether it accepted and the slot.
    fn check(cfg: &FtpConfig, leaf: &str, host: &str) -> (bool, Option<RejectedCert>) {
        let slot = RejectedSlot::default();
        let v = verifier(cfg, slot.clone()).unwrap();
        let name = ServerName::try_from(host.to_owned()).unwrap();
        let ok = v
            .verify_server_cert(&cert(leaf), &[], &name, &[], UnixTime::now())
            .is_ok();
        let rejected = slot.lock().unwrap().take();
        (ok, rejected)
    }

    #[test]
    fn fingerprint_matches_openssl() {
        assert_eq!(fingerprint(&cert("server.pem")), server_fp());
        let fp = fingerprint(b"");
        // SHA-256 of nothing.
        assert!(fp.starts_with("SHA256:E3:B0:C4:42:"), "{fp}");
        assert_eq!(fp.len(), "SHA256:".len() + 32 * 3 - 1);
    }

    #[test]
    fn trusted_root_accepts_for_ip_and_name() {
        let c = cfg(&["ca.pem"], None);
        assert!(matches!(check(&c, "server.pem", "127.0.0.1"), (true, None)));
        assert!(matches!(check(&c, "server.pem", "localhost"), (true, None)));
    }

    #[test]
    fn untrusted_is_rejected_and_kept() {
        let (ok, rejected) = check(&cfg(&[], None), "server.pem", "127.0.0.1");
        assert!(!ok);
        let r = rejected.unwrap();
        assert_eq!(r.problem, CertProblem::Untrusted);
        assert_eq!(fingerprint(&r.der), server_fp());
    }

    #[test]
    fn wrong_name_is_untrusted_even_with_the_root() {
        let (ok, rejected) = check(&cfg(&["ca.pem"], None), "server.pem", "example.com");
        assert!(!ok);
        assert_eq!(rejected.unwrap().problem, CertProblem::Untrusted);
    }

    #[test]
    fn pin_accepts_untrusted() {
        let c = cfg(&[], Some(server_fp()));
        assert!(matches!(check(&c, "server.pem", "127.0.0.1"), (true, None)));
        // The pin is about the certificate, not the name.
        assert!(matches!(
            check(&c, "server.pem", "example.com"),
            (true, None)
        ));
        // Lowercase pins match too.
        let c = cfg(&[], Some(server_fp().to_lowercase()));
        assert!(matches!(check(&c, "server.pem", "127.0.0.1"), (true, None)));
    }

    #[test]
    fn different_certificate_is_changed() {
        let c = cfg(&[], Some(server_fp()));
        let (ok, rejected) = check(&c, "other.pem", "127.0.0.1");
        assert!(!ok);
        let r = rejected.unwrap();
        assert_eq!(
            r.problem,
            CertProblem::Changed {
                pinned: server_fp()
            }
        );
        assert_eq!(fingerprint(&r.der), fingerprint(&cert("other.pem")));
    }

    #[test]
    fn unreadable_extra_root_is_an_error() {
        let c = cfg(&["missing.pem"], None);
        let e = verifier(&c, RejectedSlot::default()).unwrap_err();
        assert_eq!(e.kind, hoprs_core::ErrorKind::Tls, "{e:?}");
        assert!(e.message.contains("missing.pem"), "{e:?}");
    }

    #[test]
    fn signature_schemes_come_from_the_provider() {
        let v = verifier(&cfg(&[], None), RejectedSlot::default()).unwrap();
        assert!(!v.supported_verify_schemes().is_empty());
    }
}
