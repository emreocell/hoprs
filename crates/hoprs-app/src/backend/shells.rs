//! Terminal shells of the sessions, behind [`ShellId`]s.
//!
//! A shell belongs to the session it was opened on ([`BackendApi::open_shell`]):
//! [`BackendApi::close`] of the session closes all of its shells. A shell whose connection
//! drops ends (its output yields `Closed`, then stops); reconnecting the session does
//! **not** reopen it: the terminal tab decides (Plan 6 Task 4) and opens a new shell, with
//! a new id. The registry keeps an ended shell until the tab closes it
//! ([`BackendApi::close_shell`]) or its session closes; input to it fails with `Network`,
//! "Shell closed", like input to an unknown id.
//!
//! The output stream is handed out once ([`BackendApi::shell_output`]), for the tab's
//! subscription. Terminal bytes are never logged.
//!
//! [`BackendApi::open_shell`]: super::BackendApi::open_shell
//! [`BackendApi::close`]: super::BackendApi::close
//! [`BackendApi::close_shell`]: super::BackendApi::close_shell
//! [`BackendApi::shell_output`]: super::BackendApi::shell_output

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use futures_util::stream::BoxStream;
use hoprs_core::{Error, ErrorKind, Result};
use hoprs_ssh::{ShellHandle, ShellOutput};

use super::sessions::SessionId;

/// Identifies one shell. Monotonic: never reused within a run.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ShellId(pub u64);

/// Drives one shell: the SSH [`ShellHandle`], or a test double.
#[async_trait]
pub trait ShellControl: Send + Sync {
    async fn input(&self, data: Vec<u8>) -> Result<()>;
    async fn resize(&self, cols: u16, rows: u16) -> Result<()>;
    /// Ends the shell without waiting for the server; its output then yields `Closed`.
    fn close(&self);
}

#[async_trait]
impl ShellControl for ShellHandle {
    async fn input(&self, data: Vec<u8>) -> Result<()> {
        self.write(&data).await
    }

    async fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        ShellHandle::resize(self, cols.into(), rows.into()).await
    }

    fn close(&self) {
        ShellHandle::close(self);
    }
}

/// The error for input to a shell that ended or is unknown.
pub fn shell_closed() -> Error {
    Error::new(ErrorKind::Network, "Shell closed")
}

struct Shell {
    session: SessionId,
    control: Arc<dyn ShellControl>,
    /// Until the tab's subscription takes it.
    output: Option<BoxStream<'static, ShellOutput>>,
}

/// The open shells, by id (see the module docs).
pub struct Shells {
    next_id: AtomicU64,
    shells: Mutex<HashMap<ShellId, Shell>>,
}

impl Default for Shells {
    fn default() -> Self {
        Self {
            next_id: AtomicU64::new(1),
            shells: Mutex::new(HashMap::new()),
        }
    }
}

impl Shells {
    fn shells(&self) -> MutexGuard<'_, HashMap<ShellId, Shell>> {
        self.shells.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Registers an opened shell of `session` under a new id.
    pub fn insert(
        &self,
        session: SessionId,
        control: Arc<dyn ShellControl>,
        output: BoxStream<'static, ShellOutput>,
    ) -> ShellId {
        let id = ShellId(self.next_id.fetch_add(1, Ordering::SeqCst));
        let shell = Shell {
            session,
            control,
            output: Some(output),
        };
        self.shells().insert(id, shell);
        id
    }

    /// The shell's output, the first time it is asked for; `None` after that, and for an
    /// unknown or closed shell.
    pub fn output(&self, id: ShellId) -> Option<BoxStream<'static, ShellOutput>> {
        self.shells().get_mut(&id)?.output.take()
    }

    fn control(&self, id: ShellId) -> Result<Arc<dyn ShellControl>> {
        self.shells()
            .get(&id)
            .map(|shell| shell.control.clone())
            .ok_or_else(shell_closed)
    }

    /// The session the shell was opened on, while it is registered.
    pub fn session_of(&self, id: ShellId) -> Option<SessionId> {
        self.shells().get(&id).map(|shell| shell.session)
    }

    pub async fn input(&self, id: ShellId, data: Vec<u8>) -> Result<()> {
        self.control(id)?.input(data).await
    }

    pub async fn resize(&self, id: ShellId, cols: u16, rows: u16) -> Result<()> {
        self.control(id)?.resize(cols, rows).await
    }

    /// Closes the shell and forgets it. Unknown ids are ignored.
    pub fn close(&self, id: ShellId) {
        let shell = self.shells().remove(&id);
        if let Some(shell) = shell {
            shell.control.close();
        }
    }

    /// Closes and forgets every shell of `session`.
    pub fn close_session(&self, session: SessionId) {
        let closed: Vec<Shell> = {
            let mut shells = self.shells();
            let ids: Vec<ShellId> = shells
                .iter()
                .filter(|(_, shell)| shell.session == session)
                .map(|(id, _)| *id)
                .collect();
            ids.into_iter()
                .filter_map(|id| shells.remove(&id))
                .collect()
        };
        for shell in closed {
            shell.control.close();
        }
    }

    /// The shells of `session`, in opening order. For tests.
    #[cfg(test)]
    pub fn of_session(&self, session: SessionId) -> Vec<ShellId> {
        let mut ids: Vec<ShellId> = self
            .shells()
            .iter()
            .filter(|(_, shell)| shell.session == session)
            .map(|(id, _)| *id)
            .collect();
        ids.sort();
        ids
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;

    use futures_util::StreamExt;

    use super::*;

    #[derive(Default)]
    struct Counted {
        closes: AtomicUsize,
        inputs: Mutex<Vec<Vec<u8>>>,
    }

    #[async_trait]
    impl ShellControl for Counted {
        async fn input(&self, data: Vec<u8>) -> Result<()> {
            self.inputs.lock().unwrap().push(data);
            Ok(())
        }

        async fn resize(&self, _cols: u16, _rows: u16) -> Result<()> {
            Ok(())
        }

        fn close(&self) {
            self.closes.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn empty() -> BoxStream<'static, ShellOutput> {
        futures_util::stream::empty().boxed()
    }

    #[tokio::test]
    async fn registry_closes_only_the_sessions_shells() {
        let shells = Shells::default();
        let (one, two) = (SessionId(1), SessionId(2));
        let a = Arc::new(Counted::default());
        let b = Arc::new(Counted::default());
        let c = Arc::new(Counted::default());
        let ia = shells.insert(one, a.clone(), empty());
        let ib = shells.insert(one, b.clone(), empty());
        let ic = shells.insert(two, c.clone(), empty());
        assert!(ia < ib && ib < ic);
        assert_eq!(shells.of_session(one), vec![ia, ib]);
        assert!(shells.output(ia).is_some());
        assert!(shells.output(ia).is_none(), "handed out once");

        shells.input(ic, b"x".to_vec()).await.unwrap();
        assert_eq!(*c.inputs.lock().unwrap(), vec![b"x".to_vec()]);

        shells.close_session(one);
        assert_eq!(a.closes.load(Ordering::SeqCst), 1);
        assert_eq!(b.closes.load(Ordering::SeqCst), 1);
        assert_eq!(c.closes.load(Ordering::SeqCst), 0);
        assert!(shells.of_session(one).is_empty());
        let e = shells.input(ia, b"x".to_vec()).await.unwrap_err();
        assert_eq!(e, shell_closed());
        assert!(shells.output(ib).is_none());

        shells.close(ic);
        shells.close(ic);
        assert_eq!(c.closes.load(Ordering::SeqCst), 1);
        assert!(shells.resize(ic, 80, 24).await.is_err());
    }

    /// Plan 6 Task 6: input to a shell (a typed line, a paste, a control byte) and
    /// input to a closed shell never reach the log, at trace, on any thread.
    #[test]
    fn shell_input_never_reaches_the_log() {
        const TYPED: &[u8] = b"typed-Shell-Wx72\r";
        const PASTED: &[u8] = b"\x1b[200~pasted-Shell-Gb19\x1b[201~";
        const LATE: &[u8] = b"late-Shell-Mn64";
        let _lock = crate::subscriber_test_lock();
        let (buffer, capture) = crate::capture_log(&crate::log_filter(Some("trace")));
        capture.runtime().block_on(async {
            let shells = Arc::new(Shells::default());
            let shell = Arc::new(Counted::default());
            let id = shells.insert(SessionId(1), shell.clone(), empty());
            // From another task, as the effects run.
            tokio::spawn({
                let shells = shells.clone();
                async move {
                    shells.input(id, TYPED.to_vec()).await.unwrap();
                    shells.input(id, PASTED.to_vec()).await.unwrap();
                    shells.input(id, vec![0x03]).await.unwrap();
                }
            })
            .await
            .unwrap();
            assert_eq!(shell.inputs.lock().unwrap().len(), 3, "the shell got them");
            shells.close(id);
            assert!(shells.input(id, LATE.to_vec()).await.is_err());
            crate::log_canaries().await;
        });
        drop(capture);
        crate::assert_no_secrets(
            &buffer,
            &["typed-Shell-Wx72", "pasted-Shell-Gb19", "late-Shell-Mn64"],
        );
    }
}
