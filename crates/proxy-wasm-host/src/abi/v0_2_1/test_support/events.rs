//! A count of the warnings that one closure reports through `tracing`.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Level, Metadata, Subscriber};

struct WarningCount(Arc<AtomicUsize>);

impl Subscriber for WarningCount {
    fn enabled(&self, _: &Metadata<'_>) -> bool {
        true
    }

    fn new_span(&self, _: &Attributes<'_>) -> Id {
        Id::from_u64(1)
    }

    fn record(&self, _: &Id, _: &Record<'_>) {}

    fn record_follows_from(&self, _: &Id, _: &Id) {}

    fn event(&self, event: &Event<'_>) {
        if *event.metadata().level() == Level::WARN {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn enter(&self, _: &Id) {}

    fn exit(&self, _: &Id) {}
}

/// Runs `run` and answers how many warnings it reported on this thread.
pub(crate) fn warnings(run: impl FnOnce()) -> usize {
    let count = Arc::new(AtomicUsize::new(0));
    tracing::subscriber::with_default(WarningCount(Arc::clone(&count)), run);
    count.load(Ordering::Relaxed)
}
