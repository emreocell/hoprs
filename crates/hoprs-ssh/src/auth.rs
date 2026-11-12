//! Client authentication sequence: key, then password.
//!
//! Each step runs only if the server's last reply still lists its method.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use hoprs_core::{Error, ErrorKind, Result};
use russh::client::{self, AuthResult};
use russh::keys::{self, PrivateKey, PrivateKeyWithHashAlg};
use russh::{MethodKind, MethodSet};
use secrecy::{ExposeSecret, SecretString};

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

    if methods.contains(&MethodKind::PublicKey) {
        for path in key_files(&cfg.auth.key) {
            let configured = match &cfg.auth.key {
                KeySource::File { passphrase, .. } => passphrase.as_ref(),
                _ => None,
            };
            let Some(key) = load_key(&path, configured, prompter).await else {
                continue;
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
        .with_detail(failure_detail(&tried, &methods)))
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

fn failure_detail(tried: &[&str], methods: &MethodSet) -> String {
    let offered: Vec<&'static str> = methods.iter().map(<&'static str>::from).collect();
    let tried = if tried.is_empty() {
        "none".to_owned()
    } else {
        tried.join(", ")
    };
    format!("tried: {tried}; server offers: {}", offered.join(", "))
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

/// Loads a private key; encrypted keys use `configured` or ask once. `None` skips the key.
async fn load_key(
    path: &Path,
    configured: Option<&SecretString>,
    prompter: &dyn Prompter,
) -> Option<PrivateKey> {
    let text = std::fs::read_to_string(path).ok()?;
    match keys::decode_secret_key(&text, None) {
        Ok(key) => Some(key),
        Err(keys::Error::KeyIsEncrypted) => {
            let passphrase = match configured {
                Some(p) => p.clone(),
                None => prompter.key_passphrase(path).await?,
            };
            keys::decode_secret_key(&text, Some(passphrase.expose_secret())).ok()
        }
        Err(_) => None,
    }
}
