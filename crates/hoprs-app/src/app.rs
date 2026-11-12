//! The application state and its pure update: no I/O here, only state
//! changes and the [`Effect`]s for `effects::run` to perform.

/// The whole UI state.
#[derive(Debug)]
pub struct App {
    /// Whether the transfer queue is expanded.
    pub queue_open: bool,
    /// The sidebar's site filter, as typed.
    pub site_filter: String,
}

#[derive(Debug, Clone)]
pub enum Message {
    /// Collapse or expand the transfer queue.
    ToggleQueue,
    /// The sidebar filter changed.
    FilterSites(String),
}

/// Work for the backend. None yet: the first ones come with Task 4.
#[derive(Debug)]
pub enum Effect {}

impl App {
    pub fn new() -> Self {
        Self {
            queue_open: true,
            site_filter: String::new(),
        }
    }

    /// Applies `msg`; returns the effects to run. Never blocks.
    pub fn update(&mut self, msg: Message) -> Vec<Effect> {
        match msg {
            Message::ToggleQueue => self.queue_open = !self.queue_open,
            Message::FilterSites(filter) => self.site_filter = filter,
        }
        Vec::new()
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggle_queue_flips_and_has_no_effects() {
        let mut app = App::new();
        assert!(app.queue_open);
        assert!(app.update(Message::ToggleQueue).is_empty());
        assert!(!app.queue_open);
        assert!(app.update(Message::ToggleQueue).is_empty());
        assert!(app.queue_open);
    }
}
