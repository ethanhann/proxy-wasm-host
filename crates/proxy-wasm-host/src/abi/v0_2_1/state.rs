//! The state the ABI layer keeps in the wasmtime store.

use std::any::Any;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use crate::abi::v0_2_1::callout::CalloutTable;
use crate::abi::v0_2_1::changes::{Changes, QueueRegistration};
use crate::abi::v0_2_1::payload::Delivery;
use crate::abi::v0_2_1::types::BufferType;
use crate::abi::v0_2_1::{
    Callback, ContextId, ContextTable, GuestId, MetricId, QueueId, SharedServices, StreamState,
    VmServices,
};
use crate::runtime::HostState;

/// Reaches the ABI state the store data holds.
///
/// The store data keeps it as an opaque value, so the layer that owns the
/// state is the layer that reads it.
///
/// The accessors do not fail.
/// An instance is built only inside this crate, by `Guest::new` and by the
/// test aids of this module, and each of them fills the slot through
/// [`state`](crate::abi::v0_2_1::state).
/// The runtime tests build an instance with another value in its slot, and
/// they link no host function of this version, so nothing reads that slot.
pub(crate) trait AbiAccess {
    fn abi(&self) -> &AbiState;
    fn abi_mut(&mut self) -> &mut AbiState;
}

impl AbiAccess for HostState {
    fn abi(&self) -> &AbiState {
        self.abi_slot()
            .downcast_ref()
            .unwrap_or_else(|| unreachable!("the ABI layer fills the slot of every instance"))
    }

    fn abi_mut(&mut self) -> &mut AbiState {
        self.abi_slot_mut()
            .downcast_mut()
            .unwrap_or_else(|| unreachable!("the ABI layer fills the slot of every instance"))
    }
}

/// Everything ABI v0.2.1 keeps for one instance.
///
/// The runtime holds one of these in its store data and never reads inside
/// it, so the ABI layer adds state without a change under `runtime/`.
pub(crate) struct AbiState {
    guest: GuestId,
    services: VmServices,
    stream_state: Option<Box<dyn StreamState>>,
    contexts: ContextTable,
    current_callback: Option<Callback>,
    queues: BTreeSet<QueueId>,
    metrics: BTreeSet<MetricId>,
    granted_against: Option<Arc<dyn SharedServices>>,
    registrants: BTreeMap<QueueId, BTreeSet<ContextId>>,
    callouts: CalloutTable,
    announced: Option<(BufferType, u32)>,
    deleting: Option<ContextId>,
    delivery: Option<Delivery>,
    changes: Changes,
}

impl AbiState {
    pub(crate) fn new(services: VmServices) -> Self {
        Self {
            guest: GuestId::next(),
            services,
            stream_state: None,
            contexts: ContextTable::new(),
            current_callback: None,
            queues: BTreeSet::new(),
            metrics: BTreeSet::new(),
            granted_against: None,
            registrants: BTreeMap::new(),
            callouts: CalloutTable::new(),
            announced: None,
            deleting: None,
            delivery: None,
            changes: Changes::default(),
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
        self.registrants.clear();
        self.changes.queues.clear();
        self.granted_against = Some(Arc::clone(shared));
    }

    /// Drops the grants when the embedder has replaced the shared services.
    pub(crate) fn settle(&mut self) {
        let shared = Arc::clone(self.services.shared());
        self.settle_grants(&shared);
    }

    /// The identity of the guest this state belongs to.
    pub(crate) fn guest(&self) -> GuestId {
        self.guest
    }

    pub(crate) fn services(&self) -> &VmServices {
        &self.services
    }

    pub(crate) fn services_mut(&mut self) -> &mut VmServices {
        &mut self.services
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

    /// Records that a context of `root` registered `queue`, and reports it as
    /// a change the first time.
    pub(crate) fn register_queue(&mut self, queue: QueueId, root: ContextId, name: &[u8]) {
        if self.registrants.entry(queue).or_default().insert(root) {
            let registration = QueueRegistration::new(queue, root, name);
            self.changes.queues.insert(registration);
        }
    }

    /// Whether the shared services are the ones that issued the grants.
    fn grants_are_current(&self) -> bool {
        matches!(&self.granted_against, Some(seen) if Arc::ptr_eq(seen, self.services.shared()))
    }

    /// The roots whose contexts registered `queue` with the shared services
    /// the guest has.
    pub(crate) fn registrants(&self, queue: QueueId) -> Vec<ContextId> {
        if !self.grants_are_current() {
            return Vec::new();
        }
        self.registrants
            .get(&queue)
            .map(|roots| roots.iter().copied().collect())
            .unwrap_or_default()
    }

    /// Removes a deleted root from every queue it registered.
    pub(crate) fn forget_registrant(&mut self, root: ContextId) {
        self.registrants.retain(|_, roots| {
            roots.remove(&root);
            !roots.is_empty()
        });
    }

    /// Records a tick period a guest set on `root`.
    pub(crate) fn note_tick_period(&mut self, root: ContextId, period: Option<Duration>) {
        self.changes.tick_periods.insert(root, period);
    }

    /// The changes since the last call, without the queues of shared
    /// services that the guest does not have.
    pub(crate) fn take_changes(&mut self) -> Changes {
        if !self.grants_are_current() {
            self.changes.queues.clear();
        }
        std::mem::take(&mut self.changes)
    }

    /// The callouts the guest has open.
    pub(crate) fn callouts(&self) -> &CalloutTable {
        &self.callouts
    }

    /// The open callouts, to enter or remove one.
    pub(crate) fn callouts_mut(&mut self) -> &mut CalloutTable {
        &mut self.callouts
    }

    /// Records the buffer and the size that the running data callback
    /// announced, or clears the record.
    ///
    /// A guest that reads that buffer inside the callback gets what the
    /// stream state holds, and the crate reports a length that differs.
    pub(crate) fn set_announced(&mut self, announced: Option<(BufferType, u32)>) {
        self.announced = announced;
    }

    /// The size the running data callback announced for `buffer`.
    pub(crate) fn announced(&self, buffer: BufferType) -> Option<u32> {
        self.announced
            .and_then(|(kind, size)| (kind == buffer).then_some(size))
    }

    /// Marks the context whose deletion is running, or clears the mark.
    ///
    /// A guest runs plugin code in the failure deliveries of a deletion, and
    /// a callout it opens for that context would end with no signal to the
    /// embedder, so the callout functions refuse one.
    pub(crate) fn set_deleting(&mut self, context: Option<ContextId>) {
        self.deleting = context;
    }

    /// Whether the deletion of `context` is running.
    pub(crate) fn is_deleting(&self, context: ContextId) -> bool {
        self.deleting == Some(context)
    }

    /// The result the running callback delivers, which is `None` outside a
    /// delivery.
    pub(crate) fn delivery(&self) -> Option<&Delivery> {
        self.delivery.as_ref()
    }

    /// The delivered result as a header map read needs it.
    pub(crate) fn delivery_mut(&mut self) -> Option<&mut Delivery> {
        self.delivery.as_mut()
    }

    /// Installs the result of a delivery, or clears it with `None`.
    pub(crate) fn set_delivery(&mut self, delivery: Option<Delivery>) {
        self.delivery = delivery;
    }

    /// Records that this guest defined a metric.
    pub(crate) fn grant_metric(&mut self, metric: MetricId) {
        self.metrics.insert(metric);
    }

    /// Whether this guest defined the metric.
    pub(crate) fn holds_metric(&self, metric: MetricId) -> bool {
        self.metrics.contains(&metric)
    }

    pub(crate) fn stream_state(&mut self) -> Option<&mut dyn StreamState> {
        self.stream_state.as_deref_mut()
    }

    pub(crate) fn set_stream_state(&mut self, stream: Box<dyn StreamState>) {
        self.stream_state = Some(stream);
    }

    pub(crate) fn take_stream_state(&mut self) -> Option<Box<dyn StreamState>> {
        self.stream_state.take()
    }

    /// The installed stream state as the concrete type `enter` stored, for a
    /// change.
    pub(crate) fn stream_state_as<H: StreamState>(&mut self) -> Option<&mut H> {
        let stream: &mut dyn StreamState = self.stream_state.as_deref_mut()?;
        let any: &mut dyn Any = stream;
        any.downcast_mut::<H>()
    }

    /// The installed stream state as the concrete type `enter` stored, for a
    /// read.
    pub(crate) fn stream_state_as_ref<H: StreamState>(&self) -> Option<&H> {
        let stream: &dyn StreamState = self.stream_state.as_deref()?;
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
    use crate::abi::v0_2_1::test_support::{RecordingStream, services};

    fn state() -> AbiState {
        AbiState::new(services())
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
        assert!(state.stream_state().is_none());
    }

    #[test]
    fn the_stream_state_downcasts_for_a_read_and_for_a_change() {
        // Arrange
        let mut state = state();
        state.set_stream_state(Box::new(RecordingStream::new()));

        // Act
        let found = (
            state.stream_state_as::<RecordingStream>().is_some(),
            state.stream_state_as::<NoStream>().is_some(),
        );

        // Assert
        assert_eq!(found, (true, false));
        assert!(state.stream_state_as_ref::<RecordingStream>().is_some());
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
        let mut mine = AbiState::new(services());
        let theirs = AbiState::new(services());
        let queue = QueueId::try_from(1u32).unwrap();

        // Act
        mine.grant_queue(queue);

        // Assert
        assert!(mine.holds_queue(queue));
        assert!(!theirs.holds_queue(queue));
    }

    #[test]
    fn the_trait_reaches_the_state_the_abi_root_built() {
        // Arrange
        let services = services();
        let state = HostState::new(crate::abi::v0_2_1::state(services));

        // Act
        let found = state.abi().current_callback();

        // Assert
        assert!(found.is_none());
    }

    #[test]
    fn a_write_through_the_trait_is_read_back_through_it() {
        // Arrange
        let services = services();
        let mut state = HostState::new(crate::abi::v0_2_1::state(services));

        // Act
        state
            .abi_mut()
            .set_current_callback(Some(Callback::RequestHeaders));

        // Assert
        assert_eq!(
            state.abi().current_callback(),
            Some(Callback::RequestHeaders)
        );
    }

    #[test]
    #[should_panic(expected = "the ABI layer fills the slot")]
    fn a_slot_of_another_type_is_reported_as_an_unreachable_state() {
        // Arrange
        let state = HostState::new(Box::new(0_u8));

        // Act
        let _ = state.abi();

        // Assert
        // The panic is the assertion, which the attribute above states.
    }
}
