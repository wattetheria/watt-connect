//! Per agent-session record of successful preview calls.

use std::collections::HashMap;

/// A preview older than this no longer authorizes an order.
pub const PREVIEW_TTL_SECS: u64 = 30 * 60;

#[derive(Debug, Default, Clone)]
pub struct PolicySession {
    /// (service, preview tool) -> unix seconds of the last successful call.
    previews: HashMap<(String, String), u64>,
}

impl PolicySession {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_preview(&mut self, service: &str, tool: &str, now: u64) {
        self.previews
            .insert((service.to_owned(), tool.to_owned()), now);
    }

    #[must_use]
    pub fn has_fresh_preview(&self, service: &str, tool: &str, now: u64) -> bool {
        self.previews
            .get(&(service.to_owned(), tool.to_owned()))
            .is_some_and(|at| now.saturating_sub(*at) <= PREVIEW_TTL_SECS)
    }

    /// An order consumes its preview: the next order needs a new one.
    pub fn consume_preview(&mut self, service: &str, tool: &str) {
        self.previews.remove(&(service.to_owned(), tool.to_owned()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previews_expire_and_are_consumed() {
        let mut session = PolicySession::new();
        assert!(!session.has_fresh_preview("s", "preview", 0));
        session.record_preview("s", "preview", 100);
        assert!(session.has_fresh_preview("s", "preview", 100 + PREVIEW_TTL_SECS));
        assert!(!session.has_fresh_preview("s", "preview", 101 + PREVIEW_TTL_SECS));
        assert!(!session.has_fresh_preview("other", "preview", 100));
        session.consume_preview("s", "preview");
        assert!(!session.has_fresh_preview("s", "preview", 100));
    }
}
