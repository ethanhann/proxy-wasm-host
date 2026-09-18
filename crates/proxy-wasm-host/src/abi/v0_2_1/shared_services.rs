//! The state a VM shares with every other VM that holds the same value.

mod ids;
mod store;

use std::num::NonZeroU32;

use crate::abi::v0_2_1::Invocation;
use crate::abi::v0_2_1::types::{MetricType, Status};
use crate::abi::v0_2_1::unserved::unserved;

pub use ids::{InvalidMetricId, InvalidQueueId, MetricId, QueueId};
pub use store::{InMemoryStore, InMemoryStoreLimits};

/// One value of the shared data, with the number that guards a write to it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SharedValue {
    /// The bytes the guest stored.
    pub bytes: Vec<u8>,
    /// The number a write must match to succeed.
    ///
    /// It is never zero, because the ABI gives zero the meaning that the
    /// guest does not compare, and a guest built with the Rust SDK reads a
    /// zero as an absent value.
    pub cas: NonZeroU32,
}

impl SharedValue {
    /// A value with `bytes` and the number that guards it.
    pub fn new(bytes: Vec<u8>, cas: NonZeroU32) -> Self {
        Self { bytes, cas }
    }
}

/// The shared data, the shared queues, and the metrics of a VM.
///
/// One value serves every instance that holds the same `Arc`, which is what
/// lets one VM resolve a queue that another registered.
/// Every method takes `&self`, so an implementation holds its own state
/// behind a lock, and a method may run while another thread holds the same
/// value.
/// You supply one through
/// [`VmServices::with_shared`](crate::abi::v0_2_1::VmServices::with_shared),
/// which defaults to [`InMemoryStore`].
///
/// The shared data, the queues, and the metrics are separated by the VM id
/// rather than by the context, because a context identifier starts at one in
/// every instance and says nothing about which VM asked.
/// Two plugins that you give the same VM id share their keys, which is the
/// same control the ABI gives you for a queue.
/// A VM id you leave empty puts every plugin of that process in one
/// namespace, so set one per plugin through
/// [`VmServices::with_vm_id`](crate::abi::v0_2_1::VmServices::with_vm_id).
///
/// The crate refuses a queue or a metric identifier that the guest did not
/// obtain through a register, a resolve, or a define in this instance, so an
/// implementation does not have to check that itself.
///
/// The trait is not downcastable, so keep your own `Arc` if you want your
/// concrete type back.
///
/// Every method has a default body that reports [`Status::NotFound`], so you
/// implement what you serve.
/// The ABI names no status for a function the host does not implement, so
/// this is the crate's own rule rather than the ABI's, and `NOT_FOUND` is
/// chosen because a guest can act on it.
/// Every default body also logs a warning that names itself.
pub trait SharedServices: Send + Sync {
    /// The value and the compare and swap number of one key.
    ///
    /// # Errors
    ///
    /// Report [`Status::NotFound`] when the key is not in the store, which
    /// the default body does.
    fn get_shared_data(
        &self,
        call: Invocation,
        vm_id: &[u8],
        key: &[u8],
    ) -> Result<SharedValue, Status> {
        let _ = (call, vm_id, key);
        unserved("get_shared_data");
        Err(Status::NotFound)
    }

    /// Writes one key.
    ///
    /// A `cas` of `None` writes over whatever is there, and a `Some` writes
    /// only when the number matches the one the store holds.
    /// The number you report from [`SharedServices::get_shared_data`] is
    /// never zero, because the ABI gives zero the meaning of no comparison.
    ///
    /// # Errors
    ///
    /// Report [`Status::CasMismatch`] when the number does not match.
    /// The default body reports [`Status::NotFound`], which matches the read,
    /// so a guest is never told that a write landed when nothing holds it.
    /// A guest built with the Rust SDK stops on that status.
    fn set_shared_data(
        &self,
        call: Invocation,
        vm_id: &[u8],
        key: &[u8],
        value: &[u8],
        cas: Option<u32>,
    ) -> Result<(), Status> {
        let _ = (call, vm_id, key, value, cas);
        unserved("set_shared_data");
        Err(Status::NotFound)
    }

    /// Opens a queue under a name, and creates it when it is new.
    ///
    /// A guest is told about an item on a queue through `proxy_on_queue_ready`,
    /// which arrives with the rest of the callbacks.
    ///
    /// # Errors
    ///
    /// Report [`Status::NotFound`] when you serve no queues, which the
    /// default body does.
    /// A guest built with the Rust SDK stops on any status but `OK` here, so
    /// serve this if your guests register queues.
    fn register_shared_queue(
        &self,
        call: Invocation,
        vm_id: &[u8],
        name: &[u8],
    ) -> Result<QueueId, Status> {
        let _ = (call, vm_id, name);
        unserved("register_shared_queue");
        Err(Status::NotFound)
    }

    /// Opens a queue that another VM registered.
    ///
    /// # Errors
    ///
    /// Report [`Status::NotFound`] when no VM registered that name, which
    /// the default body does.
    fn resolve_shared_queue(
        &self,
        call: Invocation,
        vm_id: &[u8],
        name: &[u8],
    ) -> Result<QueueId, Status> {
        let _ = (call, vm_id, name);
        unserved("resolve_shared_queue");
        Err(Status::NotFound)
    }

    /// Adds one item to the end of a queue.
    ///
    /// # Errors
    ///
    /// Report [`Status::NotFound`] for a queue you do not hold, which the
    /// default body does.
    fn enqueue_shared_queue(
        &self,
        call: Invocation,
        queue: QueueId,
        value: &[u8],
    ) -> Result<(), Status> {
        let _ = (call, queue, value);
        unserved("enqueue_shared_queue");
        Err(Status::NotFound)
    }

    /// Takes one item from the front of a queue.
    ///
    /// The crate writes the item into the guest after you return it, and a
    /// guest whose allocator fails loses the item, because nothing puts it
    /// back.
    ///
    /// # Errors
    ///
    /// Report [`Status::Empty`] for a queue that holds nothing and
    /// [`Status::NotFound`] for one you do not hold, which the default body
    /// does.
    fn dequeue_shared_queue(&self, call: Invocation, queue: QueueId) -> Result<Vec<u8>, Status> {
        let _ = (call, queue);
        unserved("dequeue_shared_queue");
        Err(Status::NotFound)
    }

    /// Defines a metric, or reports the identifier of one that exists.
    ///
    /// # Errors
    ///
    /// Report [`Status::BadArgument`] when the name exists with another
    /// kind.
    /// The default body reports [`Status::NotFound`], because it holds no
    /// metrics.
    /// A guest built with the Rust SDK stops on any status but `OK` here, so
    /// serve this if your guests define metrics.
    fn define_metric(
        &self,
        call: Invocation,
        vm_id: &[u8],
        kind: MetricType,
        name: &[u8],
    ) -> Result<MetricId, Status> {
        let _ = (call, vm_id, kind, name);
        unserved("define_metric");
        Err(Status::NotFound)
    }

    /// Sets a metric to a value.
    ///
    /// The ABI says this sets the metric, so it can lower a counter, and the
    /// crate does not refuse that on your behalf.
    ///
    /// # Errors
    ///
    /// Report [`Status::NotFound`] for a metric you do not hold, which the
    /// default body does.
    fn record_metric(&self, call: Invocation, metric: MetricId, value: u64) -> Result<(), Status> {
        let _ = (call, metric, value);
        unserved("record_metric");
        Err(Status::NotFound)
    }

    /// Changes a metric by a delta.
    ///
    /// # Errors
    ///
    /// Report [`Status::BadArgument`] when the delta cannot be applied, such
    /// as a negative delta on a counter, and [`Status::NotFound`] for a
    /// metric you do not hold, which the default body does.
    fn increment_metric(
        &self,
        call: Invocation,
        metric: MetricId,
        delta: i64,
    ) -> Result<(), Status> {
        let _ = (call, metric, delta);
        unserved("increment_metric");
        Err(Status::NotFound)
    }

    /// The value of a metric.
    ///
    /// # Errors
    ///
    /// Report [`Status::NotFound`] for a metric you do not hold, which the
    /// default body does, and [`Status::BadArgument`] for a kind with no
    /// single value, such as a histogram.
    fn get_metric(&self, call: Invocation, metric: MetricId) -> Result<u64, Status> {
        let _ = (call, metric);
        unserved("get_metric");
        Err(Status::NotFound)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::v0_2_1::test_support::call;

    struct Empty;

    impl SharedServices for Empty {}

    #[test]
    fn every_default_body_reports_not_found() {
        // Arrange
        let empty = Empty;
        let id = QueueId::try_from(1u32).unwrap();
        let metric = MetricId::try_from(1u32).unwrap();

        // Act
        let results = [
            empty.get_shared_data(call(), b"vm", b"k").err(),
            empty.set_shared_data(call(), b"vm", b"k", b"v", None).err(),
            empty.register_shared_queue(call(), b"vm", b"q").err(),
            empty.resolve_shared_queue(call(), b"vm", b"q").err(),
            empty.enqueue_shared_queue(call(), id, b"v").err(),
            empty.dequeue_shared_queue(call(), id).err(),
            empty
                .define_metric(call(), b"vm", MetricType::Counter, b"m")
                .err(),
            empty.record_metric(call(), metric, 1).err(),
            empty.increment_metric(call(), metric, 1).err(),
            empty.get_metric(call(), metric).err(),
        ];

        // Assert
        assert_eq!(results, [Some(Status::NotFound); 10]);
    }
}
