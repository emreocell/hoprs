//! Client authentication sequence: key, then password.
//!
//! Each step runs only if the server's last reply still lists its method.

use std::fmt;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use hoprs_core::{Error, ErrorKind, Result};
use russh::client::{self, AuthResult};
use russh::keys::{self, PrivateKey, PrivateKeyWithHashAlg};
use russh::{MethodKind, MethodSet};
use secrecy::{ExposeSecret, SecretString};
use zeroize::Zeroizing;

use crate::config::{KeySource, SshConfig};
use crate::prompt::Prompter;
use crate::session::map_russh_error;

/// Default key files tried when the profile has no key, in this order.
const DEFAULT_KEY_FILES: [&str; 3] = ["id_ed25519", "id_ecdsa", "id_rsa"];

/// Authenticates `cfg.user` on an established connection.
pub(crate) async fn authenticate<H: client::Handler>(
    handle: &mut client::Handle<H>,
    cfg: &SshConfig,
    prompter: &dyn Prompter,
) -> Result<()> {
    let user = cfg.user.as_str();
    let mut methods = match handle
        .authenticate_none(user)
        .await
        .map_err(map_russh_error)?
    {
        AuthResult::Success => return Ok(()),
        AuthResult::Failure {
            remaining_methods, ..
        } => remaining_methods,
    };
    let mut tried: Vec<&'static str> = Vec::new();
    let mut skipped: Vec<(PathBuf, SkipReason)> = Vec::new();

    if methods.contains(&MethodKind::PublicKey) {
        for path in key_files(&cfg.auth.key) {
            let configured = match &cfg.auth.key {
                KeySource::File { passphrase, .. } => passphrase.as_ref(),
                _ => None,
            };
            let key = match load_key(&path, configured, prompter).await {
                Ok(key) => key,
                Err(reason) => {
                    skipped.push((path, reason));
                    continue;
                }
            };
            note(&mut tried, MethodKind::PublicKey);
            let hash_alg = if key.algorithm().is_rsa() {
                handle
                    .best_supported_rsa_hash()
                    .await
                    .ok()
                    .flatten()
                    .flatten()
            } else {
                None
            };
            let key = PrivateKeyWithHashAlg::new(Arc::new(key), hash_alg);
            let result = handle
                .authenticate_publickey(user, key)
                .await
                .map_err(map_russh_error)?;
            if update(result, &mut methods) {
                return Ok(());
            }
            if !methods.contains(&MethodKind::PublicKey) {
                break;
            }
        }
    }

    if methods.contains(&MethodKind::Password) {
        let password = match &cfg.auth.password {
            Some(p) => p.clone(),
            None => prompter
                .password(user, &cfg.host)
                .await
                .ok_or_else(cancelled)?,
        };
        note(&mut tried, MethodKind::Password);
        let result = handle
            .authenticate_password(user, password.expose_secret())
            .await
            .map_err(map_russh_error)?;
        if update(result, &mut methods) {
            return Ok(());
        }
    }

    if handle.is_closed() {
        return Err(Error::new(ErrorKind::Network, "Connection lost"));
    }
    Err(Error::new(ErrorKind::Auth, "Authentication failed")
        .with_detail(failure_detail(&tried, &skipped, &methods)))
}

fn cancelled() -> Error {
    Error::new(ErrorKind::Auth, "Cancelled by user")
}

/// Records the server's reply; `true` means authenticated.
fn update(result: AuthResult, methods: &mut MethodSet) -> bool {
    match result {
        AuthResult::Success => true,
        AuthResult::Failure {
            remaining_methods, ..
        } => {
            *methods = remaining_methods;
            false
        }
    }
}

fn note(tried: &mut Vec<&'static str>, method: MethodKind) {
    let name: &'static str = (&method).into();
    if !tried.contains(&name) {
        tried.push(name);
    }
}

/// `tried: publickey (skipped /k: wrong passphrase), password; server offers: ...`.
/// Method names and key paths only; never secrets.
fn failure_detail(
    tried: &[&'static str],
    skipped: &[(PathBuf, SkipReason)],
    methods: &MethodSet,
) -> String {
    let publickey: &'static str = (&MethodKind::PublicKey).into();
    let mut names = tried.to_vec();
    if !skipped.is_empty() && !names.contains(&publickey) {
        names.insert(0, publickey);
    }
    let parts: Vec<String> = names
        .iter()
        .map(|&name| {
            if name == publickey && !skipped.is_empty() {
                let why: Vec<String> = skipped
                    .iter()
                    .map(|(path, reason)| format!("skipped {}: {reason}", path.display()))
                    .collect();
                format!("{name} ({})", why.join("; "))
            } else {
                name.to_owned()
            }
        })
        .collect();
    let tried = if parts.is_empty() {
        "none".to_owned()
    } else {
        parts.join(", ")
    };
    let offered: Vec<&'static str> = methods.iter().map(<&'static str>::from).collect();
    format!("tried: {tried}; server offers: {}", offered.join(", "))
}

/// Why a key file of the "key" step was not offered to the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SkipReason {
    MissingFile,
    Unreadable,
    Unparseable,
    PassphraseCancelled,
    WrongPassphrase,
}

impl fmt::Display for SkipReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::MissingFile => "missing file",
            Self::Unreadable => "unreadable",
            Self::Unparseable => "unparseable",
            Self::PassphraseCancelled => "passphrase cancelled",
            Self::WrongPassphrase => "wrong passphrase",
        })
    }
}

/// Key files of the "key" step, in order.
fn key_files(source: &KeySource) -> Vec<PathBuf> {
    match source {
        KeySource::None => Vec::new(),
        KeySource::File { path, .. } => vec![path.clone()],
        KeySource::DefaultFiles => std::env::home_dir()
            .map(|home| {
                DEFAULT_KEY_FILES
                    .iter()
                    .map(|name| home.join(".ssh").join(name))
                    .filter(|p| p.is_file())
                    .collect()
            })
            .unwrap_or_default(),
    }
}

/// Loads a private key; encrypted keys use `configured` or ask once. `Err` skips the key.
async fn load_key(
    path: &Path,
    configured: Option<&SecretString>,
    prompter: &dyn Prompter,
) -> std::result::Result<PrivateKey, SkipReason> {
    let text = read_key_text(path).map_err(|e| match e.kind() {
        io::ErrorKind::NotFound => SkipReason::MissingFile,
        _ => SkipReason::Unreadable,
    })?;
    match keys::decode_secret_key(&text, None) {
        Ok(key) => Ok(key),
        Err(keys::Error::KeyIsEncrypted) => {
            let passphrase = match configured {
                Some(p) => p.clone(),
                None => prompter
                    .key_passphrase(path)
                    .await
                    .ok_or(SkipReason::PassphraseCancelled)?,
            };
            keys::decode_secret_key(&text, Some(passphrase.expose_secret()))
                .map_err(|_| SkipReason::WrongPassphrase)
        }
        Err(_) => Err(SkipReason::Unparseable),
    }
}

/// Reads the key file into a buffer sized up front (no reallocation) and wiped on drop.
fn read_key_text(path: &Path) -> io::Result<Zeroizing<String>> {
    let mut file = std::fs::File::open(path)?;
    let len = file.metadata()?.len() as usize;
    let mut text = Zeroizing::new(String::with_capacity(len + 1));
    file.read_to_string(&mut text)?;
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detail_names_skipped_keys() {
        let methods = MethodSet::from(&[MethodKind::PublicKey, MethodKind::Password][..]);
        let skipped = vec![
            (PathBuf::from("/k/a"), SkipReason::WrongPassphrase),
            (PathBuf::from("/k/b"), SkipReason::MissingFile),
        ];
        assert_eq!(
            failure_detail(&["password"], &skipped, &methods),
            "tried: publickey (skipped /k/a: wrong passphrase; skipped /k/b: missing file), \
             password; server offers: publickey, password"
        );
        assert_eq!(
            failure_detail(&[], &[], &methods),
            "tried: none; server offers: publickey, password"
        );
    }

    #[tokio::test]
    async fn load_key_reasons() {
        struct NoPrompt;
        #[async_trait::async_trait]
        impl Prompter for NoPrompt {
            async fn confirm_host_key(&self, _: &str, _: u16, _: &str, _: &str) -> bool {
                false
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
                _: &[crate::KbdPrompt],
            ) -> Option<Vec<SecretString>> {
                None
            }
        }
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docker/fixtures");
        let dir = tempfile::TempDir::new().unwrap();
        let garbage = dir.path().join("garbage");
        std::fs::write(&garbage, "not a key").unwrap();
        let pp = fixtures.join("id_ed25519_pp");
        let wrong = SecretString::from("wrong");

        let reason = |r: std::result::Result<PrivateKey, SkipReason>| r.err();
        assert_eq!(
            reason(load_key(&dir.path().join("none"), None, &NoPrompt).await),
            Some(SkipReason::MissingFile)
        );
        assert_eq!(
            reason(load_key(&garbage, None, &NoPrompt).await),
            Some(SkipReason::Unparseable)
        );
        assert_eq!(
            reason(load_key(&pp, None, &NoPrompt).await),
            Some(SkipReason::PassphraseCancelled)
        );
        assert_eq!(
            reason(load_key(&pp, Some(&wrong), &NoPrompt).await),
            Some(SkipReason::WrongPassphrase)
        );
        assert!(
            load_key(&fixtures.join("id_ed25519"), None, &NoPrompt)
                .await
                .is_ok()
        );
    }
}
