//! The backend the effects run against. A stub for now: the vault,
//! sessions, transfer engine and prompt broker arrive in Task 4.

use async_trait::async_trait;

/// What `effects::run` may call. Grows one method per effect.
#[async_trait]
pub trait BackendApi: Send + Sync {}

/// The real backend.
pub struct Backend;

#[async_trait]
impl BackendApi for Backend {}
