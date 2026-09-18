//! The state the ABI layer keeps in the wasmtime store.

use std::any::Any;
use std::collections::BTreeSet;
use std::sync::Arc;

use crate::abi::v0_2_1::{Callback, ContextTable, MetricId, QueueId, SharedServices, StreamHost};

/// Everything ABI v0.2.1 keeps for one instance.
///
/// The runtime holds one of these in its store data and never reads inside
/// it, so the ABI layer adds state without a change under `runtime/`.
pub(crate) struct AbiState {
    stream_host: Option<Box<dyn StreamHost>>,
    contexts: ContextTable,
    current_callback: Option<Callback>,
    queues: BTreeSet<QueueId>,
    metrics: BTreeSet<MetricId>,
    granted_against: Option<Arc<dyn SharedServices>>,
}

impl AbiState {
    pub(crate) fn new() -> Self {
        Self {
            stream_host: None,
            contexts: ContextTable::new(),
            current_callback: None,
            queues: BTreeSet::new(),
            metrics: BTreeSet::new(),
            granted_against: None,
        }
    }

    /// Drops the grants when the shared services are not the ones that issued
    /// them.
    ///
    /// A queue or metric identifier is a small number that means one thing
    /// inside one store and something else inside another, so a grant cannot
    /// outlive the store it came from.
    /// An embedder that replaces the services between calls therefore starts
    /// with no grants, and the guest registers or resolves again.
    /// The store that issued the grants is held until then, so its address
    /// cannot be reused by a different store while the comparison still
    /// matters.
    pub(crate) fn settle_grants(&mut self, shared: &Arc<dyn SharedServices>) {
        if matches!(&self.granted_against, Some(seen) if Arc::ptr_eq(seen, shared)) {
            return;
        }
        self.queues.clear();
        self.metrics.clear();
        self.granted_against = Some(Arc::clone(shared));
    }

    /// Records that this guest obtained a queue identifier.
    ///
    /// A guest that names one it never obtained is refused, so it cannot
    /// reach a queue of another VM by guessing a number.
    pub(crate) fn grant_queue(&mut self, queue: QueueId) {
        self.queues.insert(queue);
    }

    /// Whether this guest obtained the queue identifier.
    pub(crate) fn holds_queue(&self, queue: QueueId) -> bool {
        self.queues.contains(&queue)
    }

    /// Records that this guest defined a metric.
    pub(crate) fn grant_metric(&mut self, metric: MetricId) {
        self.metrics.insert(metric);
    }

    /// Whether this guest defined the metric.
    pub(crate) fn holds_metric(&self, metric: MetricId) -> bool {
        self.metrics.contains(&metric)
    }

    pub(crate) fn stream_host(&mut self) -> Option<&mut dyn StreamHost> {
        self.stream_host.as_deref_mut()
    }

    pub(crate) fn set_stream_host(&mut self, stream: Box<dyn StreamHost>) {
        self.stream_host = Some(stream);
    }

    pub(crate) fn take_stream_host(&mut self) -> Option<Box<dyn StreamHost>> {
        self.stream_host.take()
    }

    /// The installed stream host as the concrete type `enter` stored, for a
    /// change.
    pub(crate) fn stream_host_as<H: StreamHost>(&mut self) -> Option<&mut H> {
        let stream: &mut dyn StreamHost = self.stream_host.as_deref_mut()?;
        let any: &mut dyn Any = stream;
        any.downcast_mut::<H>()
    }

    /// The installed stream host as the concrete type `enter` stored, for a
    /// read.
    pub(crate) fn stream_host_as_ref<H: StreamHost>(&self) -> Option<&H> {
        let stream: &dyn StreamHost = self.stream_host.as_deref()?;
        let any: &dyn Any = stream;
        any.downcast_ref::<H>()
    }

    pub(crate) fn contexts(&self) -> &ContextTable {
        &self.contexts
    }

    pub(crate) fn contexts_mut(&mut self) -> &mut ContextTable {
        &mut self.contexts
    }

    pub(crate) fn current_callback(&self) -> Option<Callback> {
        self.current_callback
    }

    pub(crate) fn set_current_callback(&mut self, callback: Option<Callback>) {
        self.current_callback = callback;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::v0_2_1::NoStream;
    use crate::abi::v0_2_1::test_support::RecordingStream;

    fn state() -> AbiState {
        AbiState::new()
    }

    #[test]
    fn a_new_state_is_empty_and_its_table_starts_at_one() {
        // Arrange
        let mut state = state();

        // Act
        let first = state.contexts_mut().create(None).unwrap();

        // Assert
        assert_eq!(first.get(), 1);
        assert!(state.current_callback().is_none());
        assert!(state.stream_host().is_none());
    }

    #[test]
    fn the_stream_host_downcasts_for_a_read_and_for_a_change() {
        // Arrange
        let mut state = state();
        state.set_stream_host(Box::new(RecordingStream::new()));

        // Act
        let found = (
            state.stream_host_as::<RecordingStream>().is_some(),
            state.stream_host_as::<NoStream>().is_some(),
        );

        // Assert
        assert_eq!(found, (true, false));
        assert!(state.stream_host_as_ref::<RecordingStream>().is_some());
    }

    #[test]
    fn a_queue_grant_is_not_a_metric_grant() {
        // Arrange
        let mut state = state();
        let queue = QueueId::try_from(1u32).unwrap();
        let metric = MetricId::try_from(1u32).unwrap();

        // Act
        state.grant_queue(queue);

        // Assert
        assert!(state.holds_queue(queue));
        assert!(!state.holds_metric(metric));
    }

    #[test]
    fn a_grant_of_one_state_is_not_a_grant_of_another() {
        // Arrange
        let mut mine = AbiState::new();
        let theirs = AbiState::new();
        let queue = QueueId::try_from(1u32).unwrap();

        // Act
        mine.grant_queue(queue);

        // Assert
        assert!(mine.holds_queue(queue));
        assert!(!theirs.holds_queue(queue));
    }
}
