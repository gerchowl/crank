//! Minimal subscriber so `crank-ble`'s TRACE-gated capture records (no extra dependency).
use tracing::{Event, Metadata, span};

/// Enables every callsite and discards the output.
pub struct TraceAll;
impl tracing::Subscriber for TraceAll {
    fn enabled(&self, _: &Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, _: &span::Attributes<'_>) -> span::Id {
        span::Id::from_u64(1)
    }
    fn record(&self, _: &span::Id, _: &span::Record<'_>) {}
    fn record_follows_from(&self, _: &span::Id, _: &span::Id) {}
    fn event(&self, _: &Event<'_>) {}
    fn enter(&self, _: &span::Id) {}
    fn exit(&self, _: &span::Id) {}
}
