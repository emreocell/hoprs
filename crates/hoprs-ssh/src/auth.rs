//! Client authentication sequence: agent, key, password, keyboard-interactive.
//!
//! Each step runs only if the server's last reply still lists its method.
//!
//! Servers drop the connection after `MaxAuthTries` failures (OpenSSH default 6), counting
//! every public key offered. So a key file whose key the agent already offered is not
//! offered again, and when a password or keyboard-interactive step can follow, at most
//! [`MAX_PUBLICKEY_ATTEMPTS`] public keys are offered in all.

use std::fmt;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use hoprs_core::{Error, ErrorKind, Result};
use russh::client::{self, AuthResult, KeyboardInteractiveAuthResponse};
use russh::keys::{self, PrivateKey, PrivateKeyWithHashAlg};
use russh::{MethodKind, MethodSet};
use secrecy::{ExposeSecret, SecretString};
use zeroize::Zeroizing;

use crate::config::{KeySource, SshConfig};
use crate::prompt::{AcceptedSecret, KbdPrompt, Prompter};
use crate::session::map_russh_error;

/// Default key files tried when the profile has no key, in this order.
const DEFAULT_KEY_FILES: [&str; 3] = ["id_ed25519", "id_ecdsa", "id_rsa"];

/// Most agent identities offered before moving on to the key files when no password or
/// keyboard-interactive step can follow.
#[cfg(unix)]
const MAX_AGENT_IDENTITIES: usize = 5;

/// Most public keys offered in all (agent identities and key files) when a password or
/// keyboard-interactive step can follow, so that it still fits in OpenSSH's default
/// `MaxAuthTries 6`.
const MAX_PUBLICKEY_ATTEMPTS: usize = 4;

/// Most keyboard-interactive rounds answered before giving up.
const MAX_KBD_ROUNDS: usize = 10;

/// A password or key passphrase the user typed at a prompt (never a configured one), kept
/// in memory only, for the reconnections of the same session. A remembered secret that the
/// server or the key file refuses is forgotten, so the next attempt asks again.
#[derive(Default)]
pub(crate) struct Remembered {
    password: Mutex<Option<SecretString>>,
    passphrases: Mutex<Vec<(PathBuf, SecretString)>>,
}

impl Remembered {
    fn password(&self) -> Option<SecretString> {
        self.password
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn set_password(&self, password: Option<SecretString>) {
        *self.password.lock().unwrap_or_else(PoisonError::into_inner) = password;
    }

    fn passphrase(&self, path: &Path) -> Option<SecretString> {
        self.passphrases
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .find(|(p, _)| p == path)
            .map(|(_, secret)| secret.clone())
    }

    fn set_passphrase(&self, path: &Path, passphrase: Option<SecretString>) {
        let mut all = self
            .passphrases
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        all.retain(|(p, _)| p != path);
        if let Some(passphrase) = passphrase {
            all.push((path.to_owned(), passphrase));
        }
    }
}

/// Where a secret came from.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Source {
    Configured,
    Remembered,
    Prompted,
}

/// Authenticates `cfg.user` on an established connection. Secrets prompted for are kept in
/// `remembered` once they proved right (see [`Remembered`]).
pub(crate) async fn authenticate<H: client::Handler>(
    handle: &mut client::Handle<H>,
    cfg: &SshConfig,
    prompter: &dyn Prompter,
    remembered: &Remembered,
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
    let mut agent_skipped: Vec<(String, String)> = Vec::new();
    let fallback = methods.contains(&MethodKind::Password)
        || (cfg.auth.allow_keyboard_interactive
            && methods.contains(&MethodKind::KeyboardInteractive));
    let mut offers = PublicKeyOffers {
        budget: if fallback {
            MAX_PUBLICKEY_ATTEMPTS
        } else {
            usize::MAX
        },
        offered: Vec::new(),
    };

    if cfg.auth.use_agent && methods.contains(&MethodKind::PublicKey) {
        let done = try_agent(
            handle,
            cfg,
            &mut methods,
            &mut tried,
            &mut agent_skipped,
            &mut offers,
        )
        .await?;
        if done {
            return Ok(());
        }
    }

    if methods.contains(&MethodKind::PublicKey) {
        for path in key_files(&cfg.auth.key) {
            let configured = match &cfg.auth.key {
                KeySource::File { passphrase, .. } => passphrase.as_ref(),
                _ => None,
            };
            // Checked before loading, so an encrypted key never asks for its passphrase
            // in vain.
            if public_key_of(&path).is_some_and(|k| offers.was_offered(&k)) {
                skipped.push((path, SkipReason::OfferedByAgent));
                continue;
            }
            if offers.budget == 0 {
                skipped.push((path, SkipReason::AttemptLimit));
                continue;
            }
            let key = match load_key(&path, configured, prompter, remembered).await {
                Ok(key) => key,
                Err(reason) => {
                    skipped.push((path, reason));
                    continue;
                }
            };
            if offers.was_offered(key.public_key()) {
                skipped.push((path, SkipReason::OfferedByAgent));
                continue;
            }
            offers.offer(key.public_key().clone());
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

    let mut password_sent = false;
    if methods.contains(&MethodKind::Password) {
        let (password, source) = match (&cfg.auth.password, remembered.password()) {
            (Some(p), _) => (p.clone(), Source::Configured),
            (None, Some(p)) => (p, Source::Remembered),
            (None, None) => (
                prompter
                    .password(user, &cfg.host)
                    .await
                    .ok_or_else(cancelled)?,
                Source::Prompted,
            ),
        };
        note(&mut tried, MethodKind::Password);
        password_sent = cfg.auth.password.is_some();
        let result = handle
            .authenticate_password(user, password.expose_secret())
            .await
            .map_err(map_russh_error)?;
        if update(result, &mut methods) {
            if source == Source::Prompted {
                remembered.set_password(Some(password));
                prompter.secret_accepted(AcceptedSecret::Password);
            }
            return Ok(());
        }
        if source == Source::Remembered {
            remembered.set_password(None);
        }
    }

    if cfg.auth.allow_keyboard_interactive && methods.contains(&MethodKind::KeyboardInteractive) {
        note(&mut tried, MethodKind::KeyboardInteractive);
        let auto_password = if password_sent {
            None
        } else {
            cfg.auth.password.as_ref()
        };
        if keyboard_interactive(handle, user, auto_password, prompter, &mut methods).await? {
            return Ok(());
        }
    }

    if handle.is_closed() {
        // See `session::attempt`, which tells a server disconnect apart from a lost link.
        return Err(Error::new(ErrorKind::Network, "Connection lost"));
    }
    Err(
        Error::new(ErrorKind::Auth, "Authentication failed").with_detail(failure_detail(
            &tried,
            &skipped,
            &agent_skipped,
            &methods,
        )),
    )
}

/// Public keys offered so far, and how many more may be offered.
struct PublicKeyOffers {
    budget: usize,
    offered: Vec<keys::PublicKey>,
}

impl PublicKeyOffers {
    fn was_offered(&self, key: &keys::PublicKey) -> bool {
        self.offered.iter().any(|k| k.key_data() == key.key_data())
    }

    fn offer(&mut self, key: keys::PublicKey) {
        self.budget = self.budget.saturating_sub(1);
        self.offered.push(key);
    }
}

/// Offers the agent's identities by public key, at most `MAX_AGENT_IDENTITIES` and within
/// the attempt budget. A missing socket, an unreachable agent or an identity the agent
/// cannot sign with is skipped, never fatal. `true` means authenticated.
#[cfg(unix)]
async fn try_agent<H: client::Handler>(
    handle: &mut client::Handle<H>,
    cfg: &SshConfig,
    methods: &mut MethodSet,
    tried: &mut Vec<&'static str>,
    agent_skipped: &mut Vec<(String, String)>,
    offers: &mut PublicKeyOffers,
) -> Result<bool> {
    use russh::keys::agent::AgentIdentity;
    use russh::keys::agent::client::AgentClient;

    let socket = match &cfg.auth.agent_socket {
        Some(path) => path.clone(),
        None => match std::env::var_os("SSH_AUTH_SOCK") {
            Some(path) if !path.is_empty() => PathBuf::from(path),
            _ => return Ok(false),
        },
    };
    let Ok(mut agent) = AgentClient::connect_uds(&socket).await else {
        return Ok(false);
    };
    let Ok(identities) = agent.request_identities().await else {
        return Ok(false);
    };
    let user = cfg.user.as_str();
    let mut offered = 0;
    for identity in identities {
        // Certificates need the certificate method, which the profile cannot ask for yet.
        let AgentIdentity::PublicKey { key, .. } = identity else {
            continue;
        };
        if offered == MAX_AGENT_IDENTITIES
            || offers.budget == 0
            || !methods.contains(&MethodKind::PublicKey)
        {
            break;
        }
        offered += 1;
        offers.offer(key.clone());
        note_name(tried, "agent");
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
        let fingerprint = keys::PublicKey::fingerprint(&key, keys::HashAlg::Sha256).to_string();
        match handle
            .authenticate_publickey_with(user, key, hash_alg, &mut agent)
            .await
        {
            Ok(result) => {
                if update(result, methods) {
                    return Ok(true);
                }
            }
            Err(russh::AgentAuthError::Send(_)) => {
                return Err(Error::new(ErrorKind::Network, "Connection lost"));
            }
            Err(russh::AgentAuthError::Key(e)) => {
                agent_skipped.push((fingerprint, e.to_string()));
            }
        }
    }
    Ok(false)
}

#[cfg(not(unix))]
async fn try_agent<H: client::Handler>(
    _: &mut client::Handle<H>,
    _: &SshConfig,
    _: &mut MethodSet,
    _: &mut Vec<&'static str>,
    _: &mut Vec<(String, String)>,
    _: &mut PublicKeyOffers,
) -> Result<bool> {
    Ok(false)
}

/// Runs the keyboard-interactive exchange. `auto_password` answers a single no-echo prompt
/// once (PAM asks for the password this way); every other prompt goes to the prompter.
/// `true` means authenticated.
async fn keyboard_interactive<H: client::Handler>(
    handle: &mut client::Handle<H>,
    user: &str,
    auto_password: Option<&SecretString>,
    prompter: &dyn Prompter,
    methods: &mut MethodSet,
) -> Result<bool> {
    let mut auto_password = auto_password;
    let mut reply = handle
        .authenticate_keyboard_interactive_start(user, None)
        .await
        .map_err(map_russh_error)?;
    for _ in 0..MAX_KBD_ROUNDS {
        let (name, instructions, prompts) = match reply {
            KeyboardInteractiveAuthResponse::Success => return Ok(true),
            KeyboardInteractiveAuthResponse::Failure {
                remaining_methods, ..
            } => {
                *methods = remaining_methods;
                return Ok(false);
            }
            KeyboardInteractiveAuthResponse::InfoRequest {
                name,
                instructions,
                prompts,
            } => (name, instructions, prompts),
        };
        let answers: Vec<String> = if prompts.is_empty() {
            Vec::new()
        } else if let ([only], Some(password)) = (prompts.as_slice(), auto_password)
            && !only.echo
        {
            auto_password = None;
            vec![password.expose_secret().to_owned()]
        } else {
            let shown: Vec<KbdPrompt> = prompts
                .iter()
                .map(|p| KbdPrompt {
                    text: p.prompt.clone(),
                    echo: p.echo,
                })
                .collect();
            let answers = prompter
                .keyboard_interactive(&name, &instructions, &shown)
                .await
                .ok_or_else(cancelled)?;
            if answers.len() != prompts.len() {
                return Err(Error::new(
                    ErrorKind::Auth,
                    "Wrong number of keyboard-interactive answers",
                ));
            }
            answers
                .iter()
                .map(|a| a.expose_secret().to_owned())
                .collect()
        };
        reply = handle
            .authenticate_keyboard_interactive_respond(answers)
            .await
            .map_err(map_russh_error)?;
    }
    match reply {
        KeyboardInteractiveAuthResponse::Success => Ok(true),
        KeyboardInteractiveAuthResponse::Failure {
            remaining_methods, ..
        } => {
            *methods = remaining_methods;
            Ok(false)
        }
        KeyboardInteractiveAuthResponse::InfoRequest { .. } => Err(Error::new(
            ErrorKind::Auth,
            "Too many keyboard-interactive rounds",
        )),
    }
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
    note_name(tried, (&method).into());
}

fn note_name(tried: &mut Vec<&'static str>, name: &'static str) {
    if !tried.contains(&name) {
        tried.push(name);
    }
}

/// `tried: agent (skipped SHA256:x: reason), publickey (skipped /k: wrong passphrase),
/// password; server offers: ...`. Method names, fingerprints and key paths only; never secrets.
fn failure_detail(
    tried: &[&'static str],
    skipped: &[(PathBuf, SkipReason)],
    agent_skipped: &[(String, String)],
    methods: &MethodSet,
) -> String {
    let publickey: &'static str = (&MethodKind::PublicKey).into();
    let mut names = tried.to_vec();
    if !agent_skipped.is_empty() && !names.contains(&"agent") {
        names.insert(0, "agent");
    }
    if !skipped.is_empty() && !names.contains(&publickey) {
        let at = usize::from(names.first() == Some(&"agent"));
        names.insert(at, publickey);
    }
    let parts: Vec<String> = names
        .iter()
        .map(|&name| {
            if name == "agent" && !agent_skipped.is_empty() {
                let why: Vec<String> = agent_skipped
                    .iter()
                    .map(|(fingerprint, reason)| format!("skipped {fingerprint}: {reason}"))
                    .collect();
                format!("{name} ({})", why.join("; "))
            } else if name == publickey && !skipped.is_empty() {
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
    OfferedByAgent,
    AttemptLimit,
}

impl fmt::Display for SkipReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::MissingFile => "missing file",
            Self::Unreadable => "unreadable",
            Self::Unparseable => "unparseable",
            Self::PassphraseCancelled => "passphrase cancelled",
            Self::WrongPassphrase => "wrong passphrase",
            Self::OfferedByAgent => "already offered by the agent",
            Self::AttemptLimit => "public key attempt limit reached",
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

/// Loads a private key; encrypted keys use `configured`, the passphrase `remembered` for
/// this path, or ask once (and remember it if it decrypts the key). `Err` skips the key.
async fn load_key(
    path: &Path,
    configured: Option<&SecretString>,
    prompter: &dyn Prompter,
    remembered: &Remembered,
) -> std::result::Result<PrivateKey, SkipReason> {
    let text = read_key_text(path).map_err(|e| match e.kind() {
        io::ErrorKind::NotFound => SkipReason::MissingFile,
        _ => SkipReason::Unreadable,
    })?;
    match keys::decode_secret_key(&text, None) {
        Ok(key) => Ok(key),
        Err(keys::Error::KeyIsEncrypted) => {
            let (passphrase, source) = match (configured, remembered.passphrase(path)) {
                (Some(p), _) => (p.clone(), Source::Configured),
                (None, Some(p)) => (p, Source::Remembered),
                (None, None) => (
                    prompter
                        .key_passphrase(path)
                        .await
                        .ok_or(SkipReason::PassphraseCancelled)?,
                    Source::Prompted,
                ),
            };
            match keys::decode_secret_key(&text, Some(passphrase.expose_secret())) {
                Ok(key) => {
                    if source == Source::Prompted {
                        remembered.set_passphrase(path, Some(passphrase));
                        prompter.secret_accepted(AcceptedSecret::Passphrase);
                    }
                    Ok(key)
                }
                Err(_) => {
                    if source == Source::Remembered {
                        remembered.set_passphrase(path, None);
                    }
                    Err(SkipReason::WrongPassphrase)
                }
            }
        }
        Err(_) => Err(SkipReason::Unparseable),
    }
}

/// The public key of an OpenSSH private key file, read without decrypting it. `None` for
/// other formats or unreadable files.
fn public_key_of(path: &Path) -> Option<keys::PublicKey> {
    let text = read_key_text(path).ok()?;
    let key = PrivateKey::from_openssh(text.as_bytes()).ok()?;
    Some(key.public_key().clone())
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
            failure_detail(&["password"], &skipped, &[], &methods),
            "tried: publickey (skipped /k/a: wrong passphrase; skipped /k/b: missing file), \
             password; server offers: publickey, password"
        );
        assert_eq!(
            failure_detail(&[], &[], &[], &methods),
            "tried: none; server offers: publickey, password"
        );
    }

    #[test]
    fn detail_names_agent_and_kbd() {
        let methods = MethodSet::from(&[MethodKind::KeyboardInteractive][..]);
        let agent = vec![("SHA256:abc".to_owned(), "agent refused".to_owned())];
        assert_eq!(
            failure_detail(&["agent", "keyboard-interactive"], &[], &agent, &methods),
            "tried: agent (skipped SHA256:abc: agent refused), keyboard-interactive; \
             server offers: keyboard-interactive"
        );
        let skipped = vec![(PathBuf::from("/k/a"), SkipReason::MissingFile)];
        assert_eq!(
            failure_detail(&["agent"], &skipped, &agent, &methods),
            "tried: agent (skipped SHA256:abc: agent refused), \
             publickey (skipped /k/a: missing file); server offers: keyboard-interactive"
        );
    }

    #[cfg(feature = "testing")]
    #[tokio::test]
    async fn prompted_passphrase_is_remembered_until_refused() {
        use crate::prompt::ScriptedPrompter;
        let pp = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docker/fixtures/id_ed25519_pp");
        let remembered = Remembered::default();

        // A wrong prompted passphrase is not remembered.
        let wrong = ScriptedPrompter::new().with_passphrase(Some("wrong"));
        assert_eq!(
            load_key(&pp, None, &wrong, &remembered).await.err(),
            Some(SkipReason::WrongPassphrase)
        );
        assert!(remembered.passphrase(&pp).is_none());
        assert!(
            wrong.accepted().is_empty(),
            "a wrong passphrase is never accepted"
        );

        // A right one is, and is used without asking again.
        let right = ScriptedPrompter::new().with_passphrase(Some("fixture-pp"));
        load_key(&pp, None, &right, &remembered).await.unwrap();
        load_key(&pp, None, &right, &remembered).await.unwrap();
        assert_eq!(right.calls().passphrase, 1);
        assert_eq!(
            right.accepted(),
            [AcceptedSecret::Passphrase],
            "reported once, for the prompted passphrase only"
        );

        // A remembered passphrase that no longer decrypts the key is forgotten.
        remembered.set_passphrase(&pp, Some(SecretString::from("stale")));
        assert_eq!(
            load_key(&pp, None, &right, &remembered).await.err(),
            Some(SkipReason::WrongPassphrase)
        );
        assert!(remembered.passphrase(&pp).is_none());
        assert_eq!(right.calls().passphrase, 1);
    }

    #[tokio::test]
    async fn load_key_reasons() {
        struct NoPrompt;
        #[async_trait::async_trait]
        impl Prompter for NoPrompt {
            async fn confirm_host_key(
                &self,
                _: &str,
                _: u16,
                _: &str,
                _: &str,
                _: &[String],
            ) -> bool {
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
        let none = Remembered::default();

        let reason = |r: std::result::Result<PrivateKey, SkipReason>| r.err();
        assert_eq!(
            reason(load_key(&dir.path().join("none"), None, &NoPrompt, &none).await),
            Some(SkipReason::MissingFile)
        );
        assert_eq!(
            reason(load_key(&garbage, None, &NoPrompt, &none).await),
            Some(SkipReason::Unparseable)
        );
        assert_eq!(
            reason(load_key(&pp, None, &NoPrompt, &none).await),
            Some(SkipReason::PassphraseCancelled)
        );
        assert_eq!(
            reason(load_key(&pp, Some(&wrong), &NoPrompt, &none).await),
            Some(SkipReason::WrongPassphrase)
        );
        assert!(
            load_key(&fixtures.join("id_ed25519"), None, &NoPrompt, &none)
                .await
                .is_ok()
        );
    }
}
