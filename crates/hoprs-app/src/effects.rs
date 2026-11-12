//! Turns each [`Effect`] into an [`iced::Task`]: the only place that calls
//! the backend.

use std::sync::Arc;

use crate::app::{Effect, Message};
use crate::backend::BackendApi;

/// Runs one effect against the backend.
pub fn run(backend: Arc<dyn BackendApi>, effect: Effect) -> iced::Task<Message> {
    let _ = backend;
    match effect {}
}
