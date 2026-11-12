//! Site secrets (password, key passphrase) in a credential store.
//!
//! Each secret lives under the service `hoprs/<site-id>` with the account
//! `password` or `key-passphrase`. Values only travel as [`SecretString`].

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};

use hoprs_core::{Error, ErrorKind, Result, SiteId};
use keyring_core::CredentialStore;
use secrecy::{ExposeSecret, SecretString};

/// Which secret of a site.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SecretKind {
    Password,
    KeyPassphrase,
}

impl SecretKind {
    /// Account name of this secret in the credential store.
    pub fn account(self) -> &'static str {
        match self {
            Self::Password => "password",
            Self::KeyPassphrase => "key-passphrase",
        }
    }
}

/// Credential-store service name of a site's secrets.
pub(crate) fn service(site: &SiteId) -> String {
    format!("hoprs/{site}")
}

/// Where site secrets are kept.
pub trait SecretStore: Send + Sync {
    /// The secret, or `None` if there is none.
    fn get(&self, site: &SiteId, kind: SecretKind) -> Result<Option<SecretString>>;
    /// Stores `value`, replacing any previous one.
    fn set(&self, site: &SiteId, kind: SecretKind, value: &SecretString) -> Result<()>;
    /// Deletes the secret. Deleting a missing secret is `Ok`.
    fn delete(&self, site: &SiteId, kind: SecretKind) -> Result<()>;
}

/// Secrets in the system keyring (Secret Service on Linux, Keychain on
/// macOS, Credential Manager on Windows), through the `keyring` crate.
pub struct KeyringStore {
    store: Arc<CredentialStore>,
}

impl KeyringStore {
    /// Connects to the platform keyring.
    ///
    /// Fails with `Io` "System keyring unavailable" when there is no usable
    /// backend (e.g. no Secret Service or no D-Bus session).
    ///
    /// On Linux this opens a D-Bus connection to the Secret Service. Through
    /// `keyring` v4 the platform store also becomes keyring-core's
    /// process-wide default store.
    pub fn new() -> Result<Self> {
        if let Err(err) = keyring::Entry::store_status() {
            return Err(unavailable(err));
        }
        match keyring_core::get_default_store() {
            Some(store) => Ok(Self { store }),
            None => Err(unavailable(&keyring_core::Error::NoDefaultStore)),
        }
    }

    /// Uses `store` instead of the platform keyring (e.g. keyring-core's
    /// mock store in tests).
    pub fn with_store(store: Arc<CredentialStore>) -> Self {
        Self { store }
    }

    fn entry(&self, site: &SiteId, kind: SecretKind) -> Result<keyring_core::Entry> {
        self.store
            .build(&service(site), kind.account(), None)
            .map_err(|e| map_error(&e))
    }
}

impl fmt::Debug for KeyringStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeyringStore")
            .field("vendor", &self.store.vendor())
            .finish()
    }
}

impl SecretStore for KeyringStore {
    fn get(&self, site: &SiteId, kind: SecretKind) -> Result<Option<SecretString>> {
        match self.entry(site, kind)?.get_password() {
            Ok(value) => Ok(Some(SecretString::from(value))),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(err) => Err(map_error(&err)),
        }
    }

    fn set(&self, site: &SiteId, kind: SecretKind, value: &SecretString) -> Result<()> {
        self.entry(site, kind)?
            .set_password(value.expose_secret())
            .map_err(|e| map_error(&e))
    }

    fn delete(&self, site: &SiteId, kind: SecretKind) -> Result<()> {
        match self.entry(site, kind)?.delete_credential() {
            Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
            Err(err) => Err(map_error(&err)),
        }
    }
}

/// The keyring backend cannot be reached at all.
fn unavailable(err: &keyring_core::Error) -> Error {
    Error::new(ErrorKind::Io, "System keyring unavailable").with_detail(err.to_string())
}

/// Maps a keyring error to `Io`. The detail never carries secret bytes:
/// variants that hold them (or entries that may) get a fixed description.
fn map_error(err: &keyring_core::Error) -> Error {
    use keyring_core::Error as K;
    let detail = match err {
        K::PlatformFailure(_) | K::NoStorageAccess(_) | K::NoDefaultStore => {
            return unavailable(err);
        }
        K::BadEncoding(_) => "stored secret is not valid UTF-8".to_owned(),
        K::BadDataFormat(..) => "stored secret is malformed".to_owned(),
        K::Ambiguous(entries) => format!("{} matching keyring entries", entries.len()),
        other => other.to_string(),
    };
    Error::new(ErrorKind::Io, "System keyring error").with_detail(detail)
}

/// Secrets kept in memory only: for tests and dry-run imports.
#[derive(Default)]
pub struct MemoryStore {
    entries: Mutex<HashMap<(SiteId, SecretKind), SecretString>>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl MemoryStore {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<(SiteId, SecretKind), SecretString>> {
        // A panic while holding the lock cannot leave the map half-updated.
        self.entries.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Prints the number of entries, never their values.
impl fmt::Debug for MemoryStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MemoryStore")
            .field("entries", &self.lock().len())
            .finish()
    }
}

impl SecretStore for MemoryStore {
    fn get(&self, site: &SiteId, kind: SecretKind) -> Result<Option<SecretString>> {
        Ok(self.lock().get(&(site.clone(), kind)).cloned())
    }

    fn set(&self, site: &SiteId, kind: SecretKind, value: &SecretString) -> Result<()> {
        self.lock().insert((site.clone(), kind), value.clone());
        Ok(())
    }

    fn delete(&self, site: &SiteId, kind: SecretKind) -> Result<()> {
        self.lock().remove(&(site.clone(), kind));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use keyring_core::mock;

    use super::*;

    fn secret(s: &str) -> SecretString {
        SecretString::from(s)
    }

    fn exposed(s: Option<SecretString>) -> Option<String> {
        s.map(|s| s.expose_secret().to_owned())
    }

    #[test]
    fn accounts_and_service() {
        assert_eq!(SecretKind::Password.account(), "password");
        assert_eq!(SecretKind::KeyPassphrase.account(), "key-passphrase");
        assert_eq!(service(&SiteId("abc".into())), "hoprs/abc");
    }

    #[test]
    fn memory_store_roundtrip() {
        let store = MemoryStore::new();
        let site = SiteId::new();
        assert_eq!(
            exposed(store.get(&site, SecretKind::Password).unwrap()),
            None
        );
        store.delete(&site, SecretKind::Password).unwrap();
        store
            .set(&site, SecretKind::Password, &secret("hunter2"))
            .unwrap();
        store
            .set(&site, SecretKind::KeyPassphrase, &secret("phrase"))
            .unwrap();
        assert_eq!(
            exposed(store.get(&site, SecretKind::Password).unwrap()).as_deref(),
            Some("hunter2")
        );
        store.delete(&site, SecretKind::Password).unwrap();
        assert_eq!(
            exposed(store.get(&site, SecretKind::Password).unwrap()),
            None
        );
        assert_eq!(
            exposed(store.get(&site, SecretKind::KeyPassphrase).unwrap()).as_deref(),
            Some("phrase")
        );
        let debug = format!("{store:?}");
        assert!(!debug.contains("phrase"), "{debug}");
    }

    #[test]
    fn keyring_store_uses_service_and_account() {
        let mock: Arc<CredentialStore> = mock::Store::new().unwrap();
        let store = KeyringStore::with_store(mock.clone());
        let site = SiteId::new();
        assert_eq!(
            exposed(store.get(&site, SecretKind::Password).unwrap()),
            None
        );
        store.delete(&site, SecretKind::Password).unwrap();

        store
            .set(&site, SecretKind::Password, &secret("hunter2"))
            .unwrap();
        let entry = mock
            .build(&format!("hoprs/{site}"), "password", None)
            .unwrap();
        assert_eq!(entry.get_password().unwrap(), "hunter2");
        assert_eq!(
            exposed(store.get(&site, SecretKind::Password).unwrap()).as_deref(),
            Some("hunter2")
        );
        assert_eq!(
            exposed(store.get(&site, SecretKind::KeyPassphrase).unwrap()),
            None
        );

        store.delete(&site, SecretKind::Password).unwrap();
        assert!(matches!(
            entry.get_password(),
            Err(keyring_core::Error::NoEntry)
        ));
        assert_eq!(
            exposed(store.get(&site, SecretKind::Password).unwrap()),
            None
        );
        assert!(!format!("{store:?}").contains("hunter2"));
    }

    #[test]
    fn keyring_backend_failure_is_unavailable() {
        let mock: Arc<CredentialStore> = mock::Store::new().unwrap();
        let store = KeyringStore::with_store(mock.clone());
        let site = SiteId::new();
        let entry = mock
            .build(&format!("hoprs/{site}"), "password", None)
            .unwrap();
        let cred: &mock::Cred = entry.as_any().downcast_ref().unwrap();

        let failures = [
            keyring_core::Error::PlatformFailure("no D-Bus session".into()),
            keyring_core::Error::NoStorageAccess("collection locked".into()),
            keyring_core::Error::NoDefaultStore,
        ];
        for failure in failures {
            let shown = failure.to_string();
            cred.set_error(failure);
            let err = store
                .set(&site, SecretKind::Password, &secret("hunter2"))
                .unwrap_err();
            assert_eq!(err.kind, ErrorKind::Io);
            assert_eq!(err.message, "System keyring unavailable");
            assert_eq!(err.detail.as_deref(), Some(shown.as_str()));
        }
        assert_eq!(
            exposed(store.get(&site, SecretKind::Password).unwrap()),
            None
        );

        cred.set_error(keyring_core::Error::PlatformFailure("down".into()));
        let err = store.get(&site, SecretKind::Password).unwrap_err();
        assert_eq!(err.message, "System keyring unavailable");
        cred.set_error(keyring_core::Error::PlatformFailure("down".into()));
        let err = store.delete(&site, SecretKind::Password).unwrap_err();
        assert_eq!(err.message, "System keyring unavailable");

        // Other store errors are Io too, with a detail that holds no secret.
        cred.set_error(keyring_core::Error::BadEncoding(b"hunter2\xff".to_vec()));
        let err = store.get(&site, SecretKind::Password).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Io);
        assert!(!format!("{err:?}").contains("hunter2"), "{err:?}");
    }
}
