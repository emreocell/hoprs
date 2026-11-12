//! The prompt broker: turns the connection crates' [`Prompter`] / [`FtpPrompter`] calls into
//! dialogs for the UI and the UI's answers back into return values.
//!
//! - Every call becomes a [`PromptRequest`] `{ id, session, key: "host:port", kind }`, sent
//!   as a [`BrokerEvent::Request`] on the channel the UI subscribes to
//!   ([`PromptBroker::take_events`]). The answer comes back through
//!   [`PromptBroker::answer`].
//! - **Serialized per `key`:** while a host key or certificate dialog for a `key` is open,
//!   every other host key or certificate request for that `key` waits. When the dialog is
//!   answered, waiters that ask about the same key or certificate (same fingerprint) get
//!   the same answer without a dialog of their own; the others then ask in turn. Password,
//!   passphrase and keyboard-interactive prompts are never shared (each session asks).
//! - **Cancelled is refused:** a dialog the user closes answers [`PromptAnswer::Cancel`],
//!   which is `false` / `None` (and is shared with the waiters like any answer). Closing a
//!   session ([`SessionPrompter::cancel`]) refuses its open and future prompts; its open
//!   dialogs are withdrawn ([`BrokerEvent::Withdrawn`]). A dialog withdrawn that way is not
//!   an answer: the requests waiting on it ask again, in a dialog of their own.
//! - **Pins:** `store_pin` writes the accepted certificate to the session's site
//!   ([`Vault::set_ftps_pin`](hoprs_vault::Vault::set_ftps_pin)); `pin_superseded` (a trusted
//!   chain beat a stale pin, ruling R5 of Plan 3) clears it.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use hoprs_core::SiteId;
use hoprs_remote::{CertProblem, FtpPrompter};
use hoprs_ssh::{KbdPrompt, Prompter};
use secrecy::SecretString;
use tokio::sync::{mpsc, oneshot, watch};
use tokio_util::sync::CancellationToken;

use super::sessions::SessionId;
use super::vault::VaultHandle;

/// Identifies one dialog.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PromptId(pub u64);

/// A dialog the UI must show.
#[derive(Clone, Debug, PartialEq)]
pub struct PromptRequest {
    pub id: PromptId,
    pub session: SessionId,
    /// `host:port` of the connection asking.
    pub key: String,
    pub kind: PromptKind,
}

/// What a dialog asks.
#[derive(Clone, Debug, PartialEq)]
pub enum PromptKind {
    /// Trust an unknown SSH host key? `other_key_types`: key types already known for the
    /// host (empty for a host never seen).
    HostKey {
        host: String,
        port: u16,
        key_type: String,
        fingerprint: String,
        other_key_types: Vec<String>,
    },
    /// Accept (and pin) an FTPS certificate?
    Certificate {
        host: String,
        port: u16,
        fingerprint: String,
        problem: CertProblem,
    },
    Password {
        user: String,
        host: String,
    },
    Passphrase {
        path: PathBuf,
    },
    KeyboardInteractive {
        name: String,
        instructions: String,
        prompts: Vec<KbdField>,
    },
}

/// One keyboard-interactive field: its label, and whether the answer may be shown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KbdField {
    pub text: String,
    pub echo: bool,
}

/// The UI's answer to a dialog. An answer of the wrong shape for the dialog counts as
/// [`Cancel`](Self::Cancel).
#[derive(Clone)]
pub enum PromptAnswer {
    /// Host key or certificate: trust it or not.
    Decision(bool),
    /// Password or passphrase.
    Secret(SecretString),
    /// Keyboard-interactive: one response per prompt.
    Responses(Vec<SecretString>),
    /// The dialog was closed: a refusal.
    Cancel,
}

/// Never prints secrets.
impl fmt::Debug for PromptAnswer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Decision(accept) => f.debug_tuple("Decision").field(accept).finish(),
            Self::Secret(_) => f.write_str("Secret(<redacted>)"),
            Self::Responses(r) => write!(f, "Responses(<{} redacted>)", r.len()),
            Self::Cancel => f.write_str("Cancel"),
        }
    }
}

/// What the UI receives from the broker.
#[derive(Clone, Debug, PartialEq)]
pub enum BrokerEvent {
    /// Show this dialog.
    Request(PromptRequest),
    /// Close this dialog without an answer: nobody waits on it any more (its session was
    /// closed, or its connection attempt ended).
    Withdrawn(PromptId),
}

/// Hands out per-session prompters and routes the UI's answers. Cheap to clone.
#[derive(Clone)]
pub struct PromptBroker {
    inner: Arc<Inner>,
}

struct Inner {
    vault: VaultHandle,
    next_id: AtomicU64,
    events: mpsc::UnboundedSender<BrokerEvent>,
    receiver: Mutex<Option<mpsc::UnboundedReceiver<BrokerEvent>>>,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    /// Open dialogs.
    pending: HashMap<PromptId, oneshot::Sender<PromptAnswer>>,
    /// Open host key / certificate dialogs, by `key`.
    decisions: HashMap<String, InFlight>,
}

/// An open host key or certificate dialog that others may wait on.
struct InFlight {
    /// What it asks about: requests with the same subject reuse its answer.
    subject: String,
    /// Becomes `Some(answer)` once answered; closed without a value if withdrawn.
    answer: watch::Receiver<Option<bool>>,
}

impl PromptBroker {
    pub fn new(vault: VaultHandle) -> Self {
        let (events, receiver) = mpsc::unbounded_channel();
        Self {
            inner: Arc::new(Inner {
                vault,
                next_id: AtomicU64::new(1),
                events,
                receiver: Mutex::new(Some(receiver)),
                state: Mutex::new(State::default()),
            }),
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// The channel of dialogs for the UI. Only the first call gets it.
    pub fn take_events(&self) -> Option<mpsc::UnboundedReceiver<BrokerEvent>> {
        self.inner
            .receiver
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }

    /// The prompter of one session of `site`, whose connection goes to `host:port`.
    pub fn session(
        &self,
        session: SessionId,
        site: SiteId,
        host: &str,
        port: u16,
    ) -> SessionPrompter {
        SessionPrompter {
            broker: self.clone(),
            session,
            site,
            key: endpoint_key(host, port),
            cancel: CancellationToken::new(),
        }
    }

    /// Delivers the UI's answer. Returns false if the dialog is no longer open.
    pub fn answer(&self, id: PromptId, answer: PromptAnswer) -> bool {
        let Some(reply) = self.state().pending.remove(&id) else {
            return false;
        };
        reply.send(answer).is_ok()
    }

    /// Shows one dialog and waits for its answer. `None` when it was withdrawn instead:
    /// the session was cancelled, the UI is gone, or the caller stopped waiting.
    async fn ask(
        &self,
        from: &SessionPrompter,
        key: String,
        kind: PromptKind,
    ) -> Option<PromptAnswer> {
        if from.cancel.is_cancelled() {
            return None;
        }
        let id = PromptId(self.inner.next_id.fetch_add(1, Ordering::Relaxed));
        let (reply, answer) = oneshot::channel();
        self.state().pending.insert(id, reply);
        // Withdraws the dialog however this future ends without an answer (dropped too).
        let _open = OpenDialog { broker: self, id };
        let request = PromptRequest {
            id,
            session: from.session,
            key,
            kind,
        };
        if self
            .inner
            .events
            .send(BrokerEvent::Request(request))
            .is_err()
        {
            return None;
        }
        tokio::select! {
            answer = answer => answer.ok(),
            () = from.cancel.cancelled() => None,
        }
    }

    /// A host key or certificate decision, serialized per `key` (see the module docs).
    /// `subject` names what is decided: waiters with the same subject reuse the answer.
    async fn decide(
        &self,
        from: &SessionPrompter,
        key: String,
        subject: String,
        kind: PromptKind,
    ) -> bool {
        loop {
            if from.cancel.is_cancelled() {
                return false;
            }
            let turn = {
                let mut state = self.state();
                match state.decisions.get(&key) {
                    Some(open) => Turn::Wait {
                        same: open.subject == subject,
                        answer: open.answer.clone(),
                    },
                    None => {
                        let (answer, rx) = watch::channel(None);
                        let open = InFlight {
                            subject: subject.clone(),
                            answer: rx,
                        };
                        state.decisions.insert(key.clone(), open);
                        Turn::Lead(answer)
                    }
                }
            };
            match turn {
                Turn::Lead(answer) => {
                    // This request's dialog decides for the waiters too.
                    let leader = Leader {
                        broker: self,
                        key: &key,
                        answer: Some(answer),
                    };
                    return match self.ask(from, key.clone(), kind).await {
                        Some(answer) => {
                            let accept = matches!(answer, PromptAnswer::Decision(true));
                            leader.finish(accept);
                            accept
                        }
                        // Withdrawn: not an answer. Dropping `leader` wakes the waiters,
                        // and they ask again.
                        None => false,
                    };
                }
                Turn::Wait { same, mut answer } => {
                    let decided = tokio::select! {
                        decided = answer.wait_for(Option::is_some) => {
                            decided.ok().and_then(|value| *value)
                        }
                        () = from.cancel.cancelled() => return false,
                    };
                    if let Some(accept) = decided
                        && same
                    {
                        return accept;
                    }
                    // Answered about another key or certificate, or withdrawn: look again.
                }
            }
        }
    }
}

fn endpoint_key(host: &str, port: u16) -> String {
    format!("{host}:{port}")
}

/// Whether a decision request leads (shows the dialog) or waits for the open one.
enum Turn {
    Lead(watch::Sender<Option<bool>>),
    Wait {
        same: bool,
        answer: watch::Receiver<Option<bool>>,
    },
}

/// An open dialog: withdrawn when dropped unanswered.
struct OpenDialog<'a> {
    broker: &'a PromptBroker,
    id: PromptId,
}

impl Drop for OpenDialog<'_> {
    fn drop(&mut self) {
        let unanswered = self.broker.state().pending.remove(&self.id).is_some();
        if unanswered {
            let _ = self
                .broker
                .inner
                .events
                .send(BrokerEvent::Withdrawn(self.id));
        }
    }
}

/// The leading request of a `key`: owns its [`InFlight`] entry until it is answered.
/// Dropped unanswered, it removes the entry and closes the channel, which wakes the
/// waiters with no value.
struct Leader<'a> {
    broker: &'a PromptBroker,
    key: &'a str,
    answer: Option<watch::Sender<Option<bool>>>,
}

impl Leader<'_> {
    /// Ends the turn and hands `accept` to the waiters. Both happen under the state lock:
    /// no request can see the entry gone before the waiters have the answer.
    fn finish(mut self, accept: bool) {
        let mut state = self.broker.state();
        state.decisions.remove(self.key);
        if let Some(answer) = self.answer.take() {
            answer.send_replace(Some(accept));
        }
    }
}

impl Drop for Leader<'_> {
    fn drop(&mut self) {
        if self.answer.is_some() {
            // The sender is dropped right after this, closing the waiters' channel.
            self.broker.state().decisions.remove(self.key);
        }
    }
}

/// The [`Prompter`] and [`FtpPrompter`] of one session. Cheap to clone.
#[derive(Clone)]
pub struct SessionPrompter {
    broker: PromptBroker,
    session: SessionId,
    site: SiteId,
    /// `host:port` of the session's connection.
    key: String,
    cancel: CancellationToken,
}

impl SessionPrompter {
    /// The session is closing: refuses its open prompts (their dialogs are withdrawn) and
    /// every later one.
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    async fn secret(&self, kind: PromptKind) -> Option<SecretString> {
        match self.broker.ask(self, self.key.clone(), kind).await {
            Some(PromptAnswer::Secret(secret)) => Some(secret),
            _ => None,
        }
    }

    async fn password_prompt(&self, user: &str, host: &str) -> Option<SecretString> {
        self.secret(PromptKind::Password {
            user: user.to_owned(),
            host: host.to_owned(),
        })
        .await
    }
}

#[async_trait]
impl Prompter for SessionPrompter {
    async fn confirm_host_key(
        &self,
        host: &str,
        port: u16,
        key_type: &str,
        fingerprint: &str,
        other_key_types: &[String],
    ) -> bool {
        let kind = PromptKind::HostKey {
            host: host.to_owned(),
            port,
            key_type: key_type.to_owned(),
            fingerprint: fingerprint.to_owned(),
            other_key_types: other_key_types.to_vec(),
        };
        let subject = format!("host key {key_type} {fingerprint}");
        self.broker
            .decide(self, endpoint_key(host, port), subject, kind)
            .await
    }

    async fn password(&self, user: &str, host: &str) -> Option<SecretString> {
        self.password_prompt(user, host).await
    }

    async fn key_passphrase(&self, path: &Path) -> Option<SecretString> {
        self.secret(PromptKind::Passphrase {
            path: path.to_path_buf(),
        })
        .await
    }

    async fn keyboard_interactive(
        &self,
        name: &str,
        instructions: &str,
        prompts: &[KbdPrompt],
    ) -> Option<Vec<SecretString>> {
        // Servers may send a round with nothing to ask (only instructions, or nothing).
        if prompts.is_empty() {
            return Some(Vec::new());
        }
        let kind = PromptKind::KeyboardInteractive {
            name: name.to_owned(),
            instructions: instructions.to_owned(),
            prompts: prompts
                .iter()
                .map(|p| KbdField {
                    text: p.text.clone(),
                    echo: p.echo,
                })
                .collect(),
        };
        match self.broker.ask(self, self.key.clone(), kind).await {
            Some(PromptAnswer::Responses(responses)) if responses.len() == prompts.len() => {
                Some(responses)
            }
            _ => None,
        }
    }
}

#[async_trait]
impl FtpPrompter for SessionPrompter {
    async fn password(&self, user: &str, host: &str) -> Option<SecretString> {
        self.password_prompt(user, host).await
    }

    async fn confirm_certificate(
        &self,
        host: &str,
        port: u16,
        fingerprint: &str,
        problem: &CertProblem,
    ) -> bool {
        let kind = PromptKind::Certificate {
            host: host.to_owned(),
            port,
            fingerprint: fingerprint.to_owned(),
            problem: problem.clone(),
        };
        let subject = format!("certificate {fingerprint}");
        self.broker
            .decide(self, endpoint_key(host, port), subject, kind)
            .await
    }

    async fn store_pin(&self, _host: &str, _port: u16, fingerprint: &str) {
        let vault = &self.broker.inner.vault;
        let saved = vault
            .set_ftps_pin(self.site.clone(), fingerprint.to_owned())
            .await;
        if let Err(err) = saved {
            // The session keeps the pin in memory; the next run asks again.
            tracing::warn!(site = %self.site, error = %err, "cannot save the accepted certificate");
        }
    }

    async fn pin_superseded(&self, _host: &str, _port: u16, pinned: &str) {
        let vault = &self.broker.inner.vault;
        let cleared = vault
            .clear_stale_ftps_pin(self.site.clone(), pinned.to_owned())
            .await;
        match cleared {
            Ok(true) => tracing::info!(site = %self.site, "cleared a stale certificate pin"),
            Ok(false) => {}
            Err(err) => {
                tracing::warn!(site = %self.site, error = %err, "cannot clear a stale certificate pin");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use hoprs_core::{Protocol, Site};
    use hoprs_remote::CertReason;
    use hoprs_vault::{MemoryStore, Vault};
    use secrecy::ExposeSecret;
    use tokio::sync::mpsc::UnboundedReceiver;
    use tokio::task::JoinHandle;

    use super::*;

    struct Fixture {
        broker: PromptBroker,
        events: UnboundedReceiver<BrokerEvent>,
        vault: VaultHandle,
        site: Site,
        _dir: tempfile::TempDir,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = Vault::open(dir.path(), Arc::new(MemoryStore::new())).unwrap();
        let site = Site::new("f", "ftp.example.com", "u", Protocol::FtpsExplicit);
        vault.upsert_site(site.clone()).unwrap();
        let vault = VaultHandle::new(vault);
        let broker = PromptBroker::new(vault.clone());
        let events = broker.take_events().unwrap();
        assert!(
            broker.take_events().is_none(),
            "the channel is handed out once"
        );
        Fixture {
            broker,
            events,
            vault,
            site,
            _dir: dir,
        }
    }

    impl Fixture {
        fn prompter(&self, session: u64) -> SessionPrompter {
            self.broker
                .session(SessionId(session), self.site.id.clone(), "h", 21)
        }

        /// The next event, failing after a second.
        async fn next(&mut self) -> BrokerEvent {
            tokio::time::timeout(Duration::from_secs(1), self.events.recv())
                .await
                .expect("no broker event")
                .expect("broker channel closed")
        }

        async fn next_request(&mut self) -> PromptRequest {
            match self.next().await {
                BrokerEvent::Request(r) => r,
                other => panic!("expected a request, got {other:?}"),
            }
        }

        /// Asserts no event arrives for a while.
        async fn quiet(&mut self) {
            tokio::time::sleep(Duration::from_millis(50)).await;
            if let Ok(event) = self.events.try_recv() {
                panic!("unexpected {event:?}");
            }
        }
    }

    fn problem() -> CertProblem {
        CertProblem::Untrusted {
            reason: CertReason::UnknownIssuer,
        }
    }

    fn ask_cert(p: SessionPrompter, fingerprint: &str) -> JoinHandle<bool> {
        let fingerprint = fingerprint.to_owned();
        tokio::spawn(async move {
            p.confirm_certificate("h", 21, &fingerprint, &problem())
                .await
        })
    }

    async fn join<T>(handle: JoinHandle<T>) -> T {
        tokio::time::timeout(Duration::from_secs(1), handle)
            .await
            .expect("prompt never returned")
            .unwrap()
    }

    #[tokio::test]
    async fn broker_reuses_answer() {
        let mut f = fixture();
        let asks: Vec<_> = (1..=3)
            .map(|s| ask_cert(f.prompter(s), "SHA256:AA"))
            .collect();

        let request = f.next_request().await;
        assert_eq!(request.key, "h:21");
        assert_eq!(
            request.kind,
            PromptKind::Certificate {
                host: "h".into(),
                port: 21,
                fingerprint: "SHA256:AA".into(),
                problem: problem(),
            }
        );
        f.quiet().await;
        assert!(f.broker.answer(request.id, PromptAnswer::Decision(true)));
        for ask in asks {
            assert!(join(ask).await);
        }
        f.quiet().await;
        // Answering twice does nothing.
        assert!(!f.broker.answer(request.id, PromptAnswer::Decision(false)));

        // A refusal is shared the same way.
        let asks: Vec<_> = (1..=3)
            .map(|s| ask_cert(f.prompter(s), "SHA256:BB"))
            .collect();
        let request = f.next_request().await;
        f.quiet().await;
        f.broker.answer(request.id, PromptAnswer::Decision(false));
        for ask in asks {
            assert!(!join(ask).await);
        }
        f.quiet().await;
    }

    #[tokio::test]
    async fn broker_serializes_same_host() {
        let mut f = fixture();
        let first = ask_cert(f.prompter(1), "SHA256:AA");
        let r1 = f.next_request().await;

        // Another certificate and a host key for the same `host:port` wait their turn.
        let second = ask_cert(f.prompter(2), "SHA256:BB");
        let p3 = f.prompter(3);
        let third = tokio::spawn(async move {
            p3.confirm_host_key("h", 21, "ssh-ed25519", "SHA256:CC", &[])
                .await
        });
        // A different host:port does not wait.
        let p4 = f
            .broker
            .session(SessionId(4), f.site.id.clone(), "other", 21);
        let other = tokio::spawn(async move {
            p4.confirm_certificate("other", 21, "SHA256:AA", &problem())
                .await
        });
        let r_other = f.next_request().await;
        assert_eq!(r_other.key, "other:21");
        f.quiet().await;

        f.broker.answer(r1.id, PromptAnswer::Decision(true));
        assert!(join(first).await);
        // One of the waiters asks now, the other still waits.
        let r2 = f.next_request().await;
        f.quiet().await;
        f.broker.answer(r2.id, PromptAnswer::Decision(false));
        let r3 = f.next_request().await;
        f.broker.answer(r3.id, PromptAnswer::Decision(true));
        let answers = (join(second).await, join(third).await);
        let kinds = [&r2.kind, &r3.kind];
        let cert_first = matches!(kinds[0], PromptKind::Certificate { .. });
        assert_eq!(
            answers,
            if cert_first {
                (false, true)
            } else {
                (true, false)
            }
        );
        assert!(matches!(
            kinds[usize::from(cert_first)],
            PromptKind::HostKey { .. }
        ));

        f.broker.answer(r_other.id, PromptAnswer::Decision(true));
        assert!(join(other).await);
    }

    #[tokio::test]
    async fn password_prompts_are_not_shared() {
        let mut f = fixture();
        let asks: Vec<_> = (1..=2)
            .map(|s| {
                let p = f.prompter(s);
                tokio::spawn(async move { Prompter::password(&p, "u", "h").await })
            })
            .collect();
        let a = f.next_request().await;
        let b = f.next_request().await;
        assert_ne!(a.session, b.session);
        assert_eq!(
            a.kind,
            PromptKind::Password {
                user: "u".into(),
                host: "h".into()
            }
        );
        f.broker
            .answer(a.id, PromptAnswer::Secret(SecretString::from("one")));
        f.broker
            .answer(b.id, PromptAnswer::Secret(SecretString::from("two")));
        let mut got: Vec<String> = Vec::new();
        for ask in asks {
            got.push(join(ask).await.unwrap().expose_secret().to_owned());
        }
        got.sort();
        assert_eq!(got, ["one", "two"]);
    }

    #[tokio::test]
    async fn broker_cancel_is_reject() {
        let mut f = fixture();
        let p = f.prompter(1);

        // Closing the dialog refuses, whatever it asked.
        let ask = ask_cert(p.clone(), "SHA256:AA");
        let r = f.next_request().await;
        f.broker.answer(r.id, PromptAnswer::Cancel);
        assert!(!join(ask).await);

        let q = p.clone();
        let ask = tokio::spawn(async move { FtpPrompter::password(&q, "u", "h").await });
        let r = f.next_request().await;
        f.broker.answer(r.id, PromptAnswer::Cancel);
        assert!(join(ask).await.is_none());

        let q = p.clone();
        let ask = tokio::spawn(async move { q.key_passphrase(Path::new("/k")).await });
        let r = f.next_request().await;
        assert_eq!(r.kind, PromptKind::Passphrase { path: "/k".into() });
        f.broker.answer(r.id, PromptAnswer::Cancel);
        assert!(join(ask).await.is_none());

        let q = p.clone();
        let ask = tokio::spawn(async move {
            let prompts = [KbdPrompt {
                text: "Code: ".into(),
                echo: false,
            }];
            q.keyboard_interactive("n", "i", &prompts).await
        });
        let r = f.next_request().await;
        f.broker.answer(r.id, PromptAnswer::Cancel);
        assert!(join(ask).await.is_none());

        // An answer of the wrong shape is a refusal too.
        let ask = ask_cert(p.clone(), "SHA256:AA");
        let r = f.next_request().await;
        f.broker
            .answer(r.id, PromptAnswer::Secret(SecretString::from("x")));
        assert!(!join(ask).await);

        // Closing the session refuses its open dialog and withdraws it...
        let ask = ask_cert(p.clone(), "SHA256:AA");
        let r = f.next_request().await;
        p.cancel();
        assert!(!join(ask).await);
        assert_eq!(f.next().await, BrokerEvent::Withdrawn(r.id));
        assert!(!f.broker.answer(r.id, PromptAnswer::Decision(true)));
        // ...and every later prompt, without a dialog.
        assert!(Prompter::password(&p, "u", "h").await.is_none());
        assert!(!ask_cert(p.clone(), "SHA256:AA").await.unwrap());
        f.quiet().await;
    }

    #[tokio::test]
    async fn withdrawn_leader_hands_over_to_waiters() {
        let mut f = fixture();
        let leader = f.prompter(1);
        let first = ask_cert(leader.clone(), "SHA256:AA");
        let r1 = f.next_request().await;
        let waiter = ask_cert(f.prompter(2), "SHA256:AA");
        f.quiet().await;

        // The leader's session closes: that is not an answer for the waiter.
        leader.cancel();
        assert!(!join(first).await);
        assert_eq!(f.next().await, BrokerEvent::Withdrawn(r1.id));
        let r2 = f.next_request().await;
        assert_eq!(r2.session, SessionId(2));
        f.broker.answer(r2.id, PromptAnswer::Decision(true));
        assert!(join(waiter).await);

        // A waiter whose own session closes stops waiting.
        let first = ask_cert(f.prompter(1), "SHA256:BB");
        let r = f.next_request().await;
        let p2 = f.prompter(2);
        let waiting = ask_cert(p2.clone(), "SHA256:BB");
        f.quiet().await;
        p2.cancel();
        assert!(!join(waiting).await);
        f.quiet().await;
        f.broker.answer(r.id, PromptAnswer::Decision(true));
        assert!(join(first).await);
    }

    #[tokio::test]
    async fn keyboard_interactive_answers_and_skips_empty_rounds() {
        let mut f = fixture();
        let p = f.prompter(1);
        // A round with no prompts needs no dialog.
        assert_eq!(
            p.keyboard_interactive("", "", &[]).await.map(|r| r.len()),
            Some(0)
        );
        f.quiet().await;

        let ask = tokio::spawn(async move {
            let prompts = [KbdPrompt {
                text: "Code: ".into(),
                echo: false,
            }];
            p.keyboard_interactive("n", "i", &prompts).await
        });
        let r = f.next_request().await;
        assert!(
            matches!(&r.kind, PromptKind::KeyboardInteractive { prompts, .. } if prompts.len() == 1)
        );
        f.broker.answer(
            r.id,
            PromptAnswer::Responses(vec![SecretString::from("123456")]),
        );
        let responses = join(ask).await.unwrap();
        assert_eq!(responses[0].expose_secret(), "123456");
        // Debug of an answer never shows it.
        let debug = format!(
            "{:?}",
            PromptAnswer::Responses(vec![SecretString::from("123456")])
        );
        assert!(!debug.contains("123456"), "{debug}");
    }

    #[tokio::test]
    async fn store_pin_writes_vault() {
        let f = fixture();
        let p = f.prompter(1);
        p.store_pin("h", 21, "SHA256:AA").await;
        let site = f.vault.site(f.site.id.clone()).await.unwrap().unwrap();
        assert_eq!(site.ftp.pinned_fingerprint.as_deref(), Some("SHA256:AA"));

        // A stale pin other than the current one is left alone; the current one is cleared.
        p.pin_superseded("h", 21, "SHA256:BB").await;
        let site = f.vault.site(f.site.id.clone()).await.unwrap().unwrap();
        assert_eq!(site.ftp.pinned_fingerprint.as_deref(), Some("SHA256:AA"));
        p.pin_superseded("h", 21, "SHA256:AA").await;
        let site = f.vault.site(f.site.id.clone()).await.unwrap().unwrap();
        assert!(site.ftp.pinned_fingerprint.is_none());
    }
}
