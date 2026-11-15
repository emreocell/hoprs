//! FTPS certificate verification.
//!
//! A server certificate is accepted when it chains to a trusted root (the system roots plus
//! [`FtpConfig::extra_roots`]) and is valid for the host name or IP, or, failing that, when
//! its SHA-256 fingerprint equals [`FtpConfig::pinned_fingerprint`]. Anything else is
//! rejected, and the certificate and the reason are left in a [`RejectedSlot`] so the caller
//! can ask the user. A trusted chain accepted while the pin names another certificate raises
//! the [`StalePin`] flag (the pin is stale; ruling R5 of Plan 3). Handshake signatures are
//! always verified with the aws-lc-rs provider.

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
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
use super::prompt::{CertProblem, CertReason};

/// A certificate the verifier refused, and why.
#[derive(Debug, Clone)]
pub(crate) struct RejectedCert {
    pub(crate) der: CertificateDer<'static>,
    pub(crate) problem: CertProblem,
}

/// Where the verifier leaves the last certificate it refused.
pub(crate) type RejectedSlot = Arc<Mutex<Option<RejectedCert>>>;

/// Raised by the verifier when it accepts a certificate by its trusted chain although
/// `pinned_fingerprint` names a different certificate: the pin is stale.
pub(crate) type StalePin = Arc<AtomicBool>;

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
pub(crate) fn verifier(
    cfg: &FtpConfig,
    slot: RejectedSlot,
    stale: StalePin,
) -> Result<Arc<PinningVerifier>> {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let mut roots = RootCertStore::empty();
    // Unparsable system certificates are skipped, like every TLS client does.
    roots.add_parsable_certificates(system_roots().iter().cloned());
    for root in &cfg.extra_roots {
        let path = root.display().to_string();
        let bad_root = |detail: String| {
            Error::coded(
                ErrorKind::Tls,
                "tls-trusted-cert-unusable",
                format!("Cannot use trusted certificate {path}"),
            )
            .arg("path", path.to_string())
            .with_detail(detail)
        };
        let certs = CertificateDer::pem_file_iter(root)
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
                Error::coded(
                    ErrorKind::Tls,
                    "tls-verifier-build-failed",
                    "Cannot build the certificate verifier",
                )
                .with_detail(e.to_string())
            })?;
        Some(v)
    };
    Ok(Arc::new(PinningVerifier {
        webpki,
        pinned: cfg.pinned_fingerprint.clone(),
        provider,
        slot,
        stale,
    }))
}

/// Why the chain check (webpki) refused a certificate.
pub(crate) fn cert_reason(chain_error: &rustls::Error) -> CertReason {
    use CertificateError as E;
    match chain_error {
        rustls::Error::InvalidCertificate(e) => match e {
            E::Expired | E::ExpiredContext { .. } => CertReason::Expired,
            E::NotValidYet | E::NotValidYetContext { .. } => CertReason::NotYetValid,
            E::NotValidForName | E::NotValidForNameContext { .. } => CertReason::WrongName,
            E::UnknownIssuer => CertReason::UnknownIssuer,
            // A CA certificate presented as the server's own: what `openssl req -x509` makes
            // by default, i.e. a self-signed certificate.
            E::Other(other)
                if matches!(
                    other.0.downcast_ref::<webpki::Error>(),
                    Some(webpki::Error::CaUsedAsEndEntity)
                ) =>
            {
                CertReason::UnknownIssuer
            }
            _ => CertReason::Other(chain_error.to_string()),
        },
        _ => CertReason::Other(chain_error.to_string()),
    }
}

/// See the module docs.
pub(crate) struct PinningVerifier {
    /// `None` when there is no trusted root at all.
    webpki: Option<Arc<WebPkiServerVerifier>>,
    pinned: Option<String>,
    provider: Arc<CryptoProvider>,
    slot: RejectedSlot,
    stale: StalePin,
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
                    Ok(ok) => {
                        if let Some(pinned) = &self.pinned
                            && !pinned.eq_ignore_ascii_case(&fingerprint(end_entity))
                        {
                            self.stale.store(true, Ordering::SeqCst);
                        }
                        return Ok(ok);
                    }
                    Err(e) => e,
                }
            }
            None => rustls::Error::InvalidCertificate(CertificateError::UnknownIssuer),
        };
        let fp = fingerprint(end_entity);
        let reason = cert_reason(&chain_error);
        let problem = match &self.pinned {
            Some(pinned) if pinned.eq_ignore_ascii_case(&fp) => {
                return Ok(ServerCertVerified::assertion());
            }
            Some(pinned) => CertProblem::Changed {
                pinned: pinned.clone(),
                reason,
            },
            None => CertProblem::Untrusted { reason },
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
pub(crate) fn connector(
    cfg: &FtpConfig,
    slot: RejectedSlot,
    stale: StalePin,
) -> Result<AsyncRustlsConnector> {
    let verifier = verifier(cfg, slot, stale)?;
    let config = ClientConfig::builder_with_provider(verifier.provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| {
            Error::coded(
                ErrorKind::Tls,
                "tls-configure-failed",
                "Cannot configure TLS",
            )
            .with_detail(e.to_string())
        })?
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
    use crate::ftp::prompt::CertReason;
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
        check_der(cfg, &cert(leaf), host)
    }

    fn check_der(
        cfg: &FtpConfig,
        leaf: &CertificateDer<'_>,
        host: &str,
    ) -> (bool, Option<RejectedCert>) {
        let slot = RejectedSlot::default();
        let v = verifier(cfg, slot.clone(), StalePin::default()).unwrap();
        let name = ServerName::try_from(host.to_owned()).unwrap();
        let ok = v
            .verify_server_cert(leaf, &[], &name, &[], UnixTime::now())
            .is_ok();
        let rejected = slot.lock().unwrap().take();
        (ok, rejected)
    }

    /// Runs the verifier on `leaf` for `host`; returns whether it reported a stale pin.
    fn stale_after(cfg: &FtpConfig, leaf: &str, host: &str) -> bool {
        let stale = StalePin::default();
        let v = verifier(cfg, RejectedSlot::default(), stale.clone()).unwrap();
        let name = ServerName::try_from(host.to_owned()).unwrap();
        let _ = v.verify_server_cert(&cert(leaf), &[], &name, &[], UnixTime::now());
        stale.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn untrusted(reason: CertReason) -> CertProblem {
        CertProblem::Untrusted { reason }
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
        assert_eq!(r.problem, untrusted(CertReason::UnknownIssuer));
        assert_eq!(fingerprint(&r.der), server_fp());
    }

    #[test]
    fn wrong_name_is_untrusted_even_with_the_root() {
        let (ok, rejected) = check(&cfg(&["ca.pem"], None), "server.pem", "example.com");
        assert!(!ok);
        assert_eq!(rejected.unwrap().problem, untrusted(CertReason::WrongName));
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

    /// Ruling R5: a certificate that chains to a trusted root is accepted even when a pin
    /// for another certificate exists; the stale pin is never consulted, only reported.
    #[test]
    fn trusted_chain_beats_a_stale_pin() {
        let c = cfg(&["ca.pem"], Some(fingerprint(&cert("other.pem"))));
        assert!(matches!(check(&c, "server.pem", "127.0.0.1"), (true, None)));
        assert!(stale_after(&c, "server.pem", "127.0.0.1"));
    }

    #[test]
    fn stale_pin_is_only_reported_when_the_chain_overrides_a_different_pin() {
        // Trusted chain, no pin: nothing is stale.
        assert!(!stale_after(
            &cfg(&["ca.pem"], None),
            "server.pem",
            "127.0.0.1"
        ));
        // Trusted chain and a pin for this very certificate (any case): not stale.
        let c = cfg(&["ca.pem"], Some(server_fp().to_lowercase()));
        assert!(!stale_after(&c, "server.pem", "127.0.0.1"));
        // Accepted by the pin alone: not stale.
        assert!(!stale_after(
            &cfg(&[], Some(server_fp())),
            "server.pem",
            "127.0.0.1"
        ));
        // Rejected (changed certificate): not stale either, the user decides.
        let c = cfg(&[], Some(server_fp()));
        assert!(!stale_after(&c, "other.pem", "127.0.0.1"));
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
                pinned: server_fp(),
                reason: CertReason::UnknownIssuer,
            }
        );
        assert_eq!(fingerprint(&r.der), fingerprint(&cert("other.pem")));
    }

    /// A test CA (written to a PEM file in `dir`) and leaf certificates it signs.
    struct TestCa {
        issuer: rcgen::CertifiedIssuer<'static, rcgen::KeyPair>,
        pem: PathBuf,
    }

    impl TestCa {
        fn new(dir: &std::path::Path) -> TestCa {
            let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
            params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
            let issuer =
                rcgen::CertifiedIssuer::self_signed(params, rcgen::KeyPair::generate().unwrap())
                    .unwrap();
            let pem = dir.join("test-ca.pem");
            std::fs::write(&pem, issuer.pem()).unwrap();
            TestCa { issuer, pem }
        }

        /// A leaf for `name`, valid from `from` to `until` (years, January 1st).
        fn leaf(&self, name: &str, from: i32, until: i32) -> CertificateDer<'static> {
            let mut params = rcgen::CertificateParams::new(vec![name.to_owned()]).unwrap();
            params.not_before = rcgen::date_time_ymd(from, 1, 1);
            params.not_after = rcgen::date_time_ymd(until, 1, 1);
            let key = rcgen::KeyPair::generate().unwrap();
            params.signed_by(&key, &self.issuer).unwrap().der().clone()
        }

        fn cfg(&self, pin: Option<String>) -> FtpConfig {
            let mut c = FtpConfig::new("localhost", 21, "u", FtpSecurity::ExplicitTls);
            c.extra_roots = vec![self.pem.clone()];
            c.pinned_fingerprint = pin;
            c
        }
    }

    #[test]
    fn cert_reason_maps_webpki_errors() {
        let dir = tempfile::TempDir::new().unwrap();
        let ca = TestCa::new(dir.path());
        let reason = |cfg: &FtpConfig, leaf: &CertificateDer<'_>, host: &str| {
            let (ok, rejected) = check_der(cfg, leaf, host);
            assert!(!ok);
            rejected.unwrap().problem
        };
        let cfg = ca.cfg(None);
        // Sanity: a current leaf for the right name is trusted.
        let good = ca.leaf("localhost", 2000, 4000);
        assert!(matches!(check_der(&cfg, &good, "localhost"), (true, None)));

        let expired = ca.leaf("localhost", 2000, 2001);
        assert_eq!(
            reason(&cfg, &expired, "localhost"),
            untrusted(CertReason::Expired)
        );
        let future = ca.leaf("localhost", 3000, 4000);
        assert_eq!(
            reason(&cfg, &future, "localhost"),
            untrusted(CertReason::NotYetValid)
        );
        let other_name = ca.leaf("example.com", 2000, 4000);
        assert_eq!(
            reason(&cfg, &other_name, "localhost"),
            untrusted(CertReason::WrongName)
        );
        // Without the test CA the issuer is unknown.
        assert_eq!(
            reason(&cfg_plain(), &good, "localhost"),
            untrusted(CertReason::UnknownIssuer)
        );
        // A pinned certificate that changed carries the reason too.
        let pinned = fingerprint(&good);
        assert_eq!(
            reason(&ca.cfg(Some(pinned.clone())), &expired, "localhost"),
            CertProblem::Changed {
                pinned,
                reason: CertReason::Expired,
            }
        );
        // Anything else keeps rustls' description.
        assert_eq!(
            cert_reason(&rustls::Error::InvalidCertificate(
                CertificateError::BadSignature
            )),
            CertReason::Other(
                rustls::Error::InvalidCertificate(CertificateError::BadSignature).to_string()
            )
        );
    }

    fn cfg_plain() -> FtpConfig {
        FtpConfig::new("localhost", 21, "u", FtpSecurity::ExplicitTls)
    }

    #[test]
    fn unreadable_extra_root_is_an_error() {
        let c = cfg(&["missing.pem"], None);
        let e = verifier(&c, RejectedSlot::default(), StalePin::default()).unwrap_err();
        assert_eq!(e.kind, hoprs_core::ErrorKind::Tls, "{e:?}");
        assert!(e.message.contains("missing.pem"), "{e:?}");
    }

    #[test]
    fn signature_schemes_come_from_the_provider() {
        let v = verifier(
            &cfg(&[], None),
            RejectedSlot::default(),
            StalePin::default(),
        )
        .unwrap();
        assert!(!v.supported_verify_schemes().is_empty());
    }
}
