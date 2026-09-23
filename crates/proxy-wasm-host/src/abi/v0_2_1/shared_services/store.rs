//! The shared services of one process, held in memory.

mod limits;
mod observer;

pub use limits::InMemoryStoreLimits;
pub use observer::QueueEnqueued;

use observer::Observer;

use std::collections::{BTreeMap, VecDeque};
use std::num::NonZeroU32;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use crate::abi::v0_2_1::Invocation;
use crate::abi::v0_2_1::types::{MetricType, Status};

use super::{MetricId, QueueId, SharedServices, SharedValue};

type Key = (Vec<u8>, Vec<u8>);

#[derive(Debug, Default)]
struct Queues {
    by_name: BTreeMap<Key, QueueId>,
    owners: BTreeMap<QueueId, Key>,
    items: BTreeMap<QueueId, VecDeque<Vec<u8>>>,
    next: u32,
}

#[derive(Debug)]
struct MetricEntry {
    kind: MetricType,
    value: u64,
}

#[derive(Debug, Default)]
struct Metrics {
    by_name: BTreeMap<Key, MetricId>,
    entries: BTreeMap<MetricId, MetricEntry>,
    next: u32,
}

/// The shared services of one process.
///
/// Every instance that holds the same `Arc` shares this state, which is what
/// lets one VM resolve a queue that another registered.
/// It is the default of
/// [`VmServices::with_shared`](crate::abi::v0_2_1::VmServices::with_shared),
/// so a guest works without any shared state of your own.
///
/// It serves one process.
/// A metric recorded into it reaches no metric sink of your proxy, a value
/// written into it is gone when the process ends, and a proxy that runs
/// several processes supplies an implementation that reaches across them.
///
/// Each family has its own lock, so a metric does not wait behind a queue,
/// and a panic while a lock is held does not stop the other instances.
///
/// A guest drives every one of these families directly, so the store bounds
/// what it holds.
/// [`InMemoryStoreLimits::default`] allows 4096 keys of at most 64 KiB each,
/// 1024 items on a queue, 4096 queues, and 4096 metrics, and a write past a
/// bound reports [`Status::InternalFailure`].
/// Change them with [`InMemoryStore::with_limits`].
///
/// A histogram takes an observation and keeps no value, because the ABI gives
/// a guest no way to read one back, and a read of a histogram reports
/// [`Status::BadArgument`], which is what the reference host does.
#[derive(Debug, Default)]
pub struct InMemoryStore {
    data: Mutex<BTreeMap<Key, (Vec<u8>, NonZeroU32)>>,
    queues: Mutex<Queues>,
    metrics: Mutex<Metrics>,
    limits: InMemoryStoreLimits,
    observer: Option<Observer>,
    warned_no_observer: AtomicBool,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The next compare and swap number, which is never zero, because the ABI
/// gives zero the meaning of no comparison.
fn next_cas(current: NonZeroU32) -> NonZeroU32 {
    NonZeroU32::new(current.get().wrapping_add(1)).unwrap_or(NonZeroU32::MIN)
}

fn next_id(next: &mut u32) -> Option<NonZeroU32> {
    let value = next.checked_add(1)?;
    *next = value;
    NonZeroU32::new(value)
}

impl InMemoryStore {
    /// An empty store, with no data, no queue, and no metric.
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces what the store allows a guest to hold.
    #[must_use]
    pub fn with_limits(mut self, limits: InMemoryStoreLimits) -> Self {
        self.limits = limits;
        self
    }
}

impl SharedServices for InMemoryStore {
    fn get_shared_data(
        &self,
        _: Invocation,
        vm_id: &[u8],
        key: &[u8],
    ) -> Result<SharedValue, Status> {
        lock(&self.data)
            .get(&(vm_id.to_vec(), key.to_vec()))
            .map(|(bytes, cas)| SharedValue::new(bytes.clone(), *cas))
            .ok_or(Status::NotFound)
    }

    fn set_shared_data(
        &self,
        _: Invocation,
        vm_id: &[u8],
        key: &[u8],
        value: &[u8],
        cas: Option<u32>,
    ) -> Result<(), Status> {
        if value.len() > self.limits.value_bytes() {
            return Err(Status::InternalFailure);
        }
        let mut data = lock(&self.data);
        let held = (vm_id.to_vec(), key.to_vec());
        if !data.contains_key(&held) && data.len() >= self.limits.keys() {
            return Err(Status::InternalFailure);
        }
        match data.entry(held) {
            std::collections::btree_map::Entry::Occupied(mut held) => {
                let (_, current) = held.get();
                if cas.is_some_and(|given| given != current.get()) {
                    return Err(Status::CasMismatch);
                }
                let next = next_cas(*current);
                held.insert((value.to_vec(), next));
            }
            std::collections::btree_map::Entry::Vacant(free) => {
                free.insert((value.to_vec(), NonZeroU32::MIN));
            }
        }
        Ok(())
    }

    fn register_shared_queue(
        &self,
        _: Invocation,
        vm_id: &[u8],
        name: &[u8],
    ) -> Result<QueueId, Status> {
        if self.observer.is_none() && !self.warned_no_observer.swap(true, Ordering::Relaxed) {
            tracing::warn!(
                "a guest registered a queue on a store with no enqueue observer, \
                 so no guest hears of an item"
            );
        }
        let mut queues = lock(&self.queues);
        let key = (vm_id.to_vec(), name.to_vec());
        if let Some(id) = queues.by_name.get(&key) {
            return Ok(*id);
        }
        if queues.by_name.len() >= self.limits.queues() {
            return Err(Status::InternalFailure);
        }
        let id = QueueId::from_non_zero(next_id(&mut queues.next).ok_or(Status::InternalFailure)?);
        queues.by_name.insert(key.clone(), id);
        queues.owners.insert(id, key);
        queues.items.insert(id, VecDeque::new());
        Ok(id)
    }

    fn resolve_shared_queue(
        &self,
        _: Invocation,
        vm_id: &[u8],
        name: &[u8],
    ) -> Result<QueueId, Status> {
        lock(&self.queues)
            .by_name
            .get(&(vm_id.to_vec(), name.to_vec()))
            .copied()
            .ok_or(Status::NotFound)
    }

    fn enqueue_shared_queue(
        &self,
        _: Invocation,
        queue: QueueId,
        value: &[u8],
    ) -> Result<(), Status> {
        if value.len() > self.limits.value_bytes() {
            return Err(Status::InternalFailure);
        }
        let depth = self.limits.queue_items();
        let owner = {
            let mut queues = lock(&self.queues);
            let items = queues.items.get_mut(&queue).ok_or(Status::NotFound)?;
            if items.len() >= depth {
                return Err(Status::InternalFailure);
            }
            items.push_back(value.to_vec());
            queues.owners.get(&queue).cloned()
        };
        if let (Some(observer), Some((vm_id, name))) = (&self.observer, owner) {
            (observer.0)(QueueEnqueued {
                vm_id: &vm_id,
                name: &name,
                queue,
            });
        }
        Ok(())
    }

    fn dequeue_shared_queue(&self, _: Invocation, queue: QueueId) -> Result<Vec<u8>, Status> {
        let mut queues = lock(&self.queues);
        let items = queues.items.get_mut(&queue).ok_or(Status::NotFound)?;
        items.pop_front().ok_or(Status::Empty)
    }

    fn define_metric(
        &self,
        _: Invocation,
        vm_id: &[u8],
        kind: MetricType,
        name: &[u8],
    ) -> Result<MetricId, Status> {
        let mut metrics = lock(&self.metrics);
        let key = (vm_id.to_vec(), name.to_vec());
        if let Some(id) = metrics.by_name.get(&key).copied() {
            let held = metrics.entries.get(&id).ok_or(Status::InternalFailure)?;
            if held.kind == kind {
                return Ok(id);
            }
            return Err(Status::BadArgument);
        }
        if metrics.by_name.len() >= self.limits.metrics() {
            return Err(Status::InternalFailure);
        }
        let id =
            MetricId::from_non_zero(next_id(&mut metrics.next).ok_or(Status::InternalFailure)?);
        metrics.by_name.insert(key, id);
        metrics.entries.insert(id, MetricEntry { kind, value: 0 });
        Ok(id)
    }

    fn record_metric(&self, _: Invocation, metric: MetricId, value: u64) -> Result<(), Status> {
        let mut metrics = lock(&self.metrics);
        let entry = metrics.entries.get_mut(&metric).ok_or(Status::NotFound)?;
        match entry.kind {
            MetricType::Counter | MetricType::Gauge => entry.value = value,
            MetricType::Histogram => {}
        }
        Ok(())
    }

    fn increment_metric(&self, _: Invocation, metric: MetricId, delta: i64) -> Result<(), Status> {
        let mut metrics = lock(&self.metrics);
        let entry = metrics.entries.get_mut(&metric).ok_or(Status::NotFound)?;
        match entry.kind {
            MetricType::Counter if delta < 0 => return Err(Status::BadArgument),
            MetricType::Histogram => return Err(Status::BadArgument),
            _ => {}
        }
        entry.value = entry
            .value
            .checked_add_signed(delta)
            .ok_or(Status::BadArgument)?;
        Ok(())
    }

    fn get_metric(&self, _: Invocation, metric: MetricId) -> Result<u64, Status> {
        let metrics = lock(&self.metrics);
        let entry = metrics.entries.get(&metric).ok_or(Status::NotFound)?;
        match entry.kind {
            MetricType::Histogram => Err(Status::BadArgument),
            _ => Ok(entry.value),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::abi::v0_2_1::test_support::call;

    const VM: &[u8] = b"vm-1";
    const OTHER_VM: &[u8] = b"vm-2";

    fn queue(services: &InMemoryStore, vm: &[u8]) -> QueueId {
        services.register_shared_queue(call(), vm, b"q").unwrap()
    }

    fn counter(services: &InMemoryStore) -> MetricId {
        services
            .define_metric(call(), VM, MetricType::Counter, b"requests")
            .unwrap()
    }

    #[test]
    fn a_value_reads_back_with_a_number_that_is_not_zero() {
        // Arrange
        let services = InMemoryStore::new();
        services
            .set_shared_data(call(), VM, b"k", b"v", None)
            .unwrap();

        // Act
        let found = services.get_shared_data(call(), VM, b"k");

        // Assert
        let value = found.unwrap();
        assert_eq!(value.bytes, b"v");
        assert_eq!(value.cas.get(), 1);
    }

    #[test]
    fn a_key_of_one_vm_is_not_visible_to_another() {
        // Arrange
        let services = InMemoryStore::new();
        services
            .set_shared_data(call(), VM, b"k", b"secret", None)
            .unwrap();

        // Act
        let found = services.get_shared_data(call(), OTHER_VM, b"k");

        // Assert
        assert_eq!(found, Err(Status::NotFound));
    }

    #[test]
    fn an_absent_key_is_not_found() {
        // Arrange
        let services = InMemoryStore::new();

        // Act
        let found = services.get_shared_data(call(), VM, b"missing");

        // Assert
        assert_eq!(found, Err(Status::NotFound));
    }

    #[test]
    fn a_stale_number_is_refused_and_the_value_stays() {
        // Arrange
        let services = InMemoryStore::new();
        services
            .set_shared_data(call(), VM, b"k", b"first", None)
            .unwrap();
        let cas = services
            .get_shared_data(call(), VM, b"k")
            .unwrap()
            .cas
            .get();

        // Act
        let refused = services.set_shared_data(call(), VM, b"k", b"second", Some(cas + 1));

        // Assert
        assert_eq!(refused, Err(Status::CasMismatch));
        assert_eq!(
            services.get_shared_data(call(), VM, b"k").unwrap().bytes,
            b"first"
        );
    }

    #[test]
    fn the_number_the_store_reports_writes_the_value() {
        // Arrange
        let services = InMemoryStore::new();
        services
            .set_shared_data(call(), VM, b"k", b"first", None)
            .unwrap();
        let cas = services
            .get_shared_data(call(), VM, b"k")
            .unwrap()
            .cas
            .get();

        // Act
        let written = services.set_shared_data(call(), VM, b"k", b"second", Some(cas));

        // Assert
        assert_eq!(written, Ok(()));
        assert_eq!(
            services.get_shared_data(call(), VM, b"k").unwrap().bytes,
            b"second"
        );
    }

    #[test]
    fn no_number_writes_over_any_value() {
        // Arrange
        let services = InMemoryStore::new();
        services
            .set_shared_data(call(), VM, b"k", b"first", None)
            .unwrap();

        // Act
        let written = services.set_shared_data(call(), VM, b"k", b"second", None);

        // Assert
        assert_eq!(written, Ok(()));
        assert_eq!(
            services.get_shared_data(call(), VM, b"k").unwrap().bytes,
            b"second"
        );
    }

    #[test]
    fn a_number_on_a_key_that_is_not_there_writes_it() {
        // Arrange
        let services = InMemoryStore::new();

        // Act
        let written = services.set_shared_data(call(), VM, b"k", b"v", Some(7));

        // Assert
        assert_eq!(written, Ok(()));
        assert_eq!(
            services.get_shared_data(call(), VM, b"k").unwrap().bytes,
            b"v"
        );
    }

    #[test]
    fn a_value_past_the_limit_is_refused() {
        // Arrange
        let services =
            InMemoryStore::new().with_limits(InMemoryStoreLimits::new().with_value_bytes(4));

        // Act
        let refused = services.set_shared_data(call(), VM, b"k", b"12345", None);

        // Assert
        assert_eq!(refused, Err(Status::InternalFailure));
    }

    #[test]
    fn a_key_past_the_limit_is_refused() {
        // Arrange
        let services = InMemoryStore::new().with_limits(InMemoryStoreLimits::new().with_keys(1));
        services
            .set_shared_data(call(), VM, b"first", b"v", None)
            .unwrap();

        // Act
        let refused = services.set_shared_data(call(), VM, b"second", b"v", None);

        // Assert
        assert_eq!(refused, Err(Status::InternalFailure));
        assert_eq!(
            services
                .get_shared_data(call(), VM, b"first")
                .unwrap()
                .bytes,
            b"v"
        );
    }

    #[test]
    fn a_queue_past_the_limit_is_refused_and_not_created() {
        // Arrange
        let services = InMemoryStore::new().with_limits(InMemoryStoreLimits::new().with_queues(1));
        services
            .register_shared_queue(call(), VM, b"first")
            .unwrap();

        // Act
        let refused = services.register_shared_queue(call(), VM, b"second");

        // Assert
        assert_eq!(refused, Err(Status::InternalFailure));
        assert_eq!(
            services.resolve_shared_queue(call(), VM, b"second"),
            Err(Status::NotFound)
        );
    }

    #[test]
    fn a_queue_the_store_holds_opens_at_the_limit() {
        // Arrange
        let services = InMemoryStore::new().with_limits(InMemoryStoreLimits::new().with_queues(1));
        let first = services
            .register_shared_queue(call(), VM, b"first")
            .unwrap();

        // Act
        let again = services.register_shared_queue(call(), VM, b"first");

        // Assert
        assert_eq!(again, Ok(first));
    }

    #[test]
    fn a_metric_past_the_limit_is_refused() {
        // Arrange
        let services = InMemoryStore::new().with_limits(InMemoryStoreLimits::new().with_metrics(1));
        let first = services
            .define_metric(call(), VM, MetricType::Counter, b"first")
            .unwrap();

        // Act
        let refused = services.define_metric(call(), VM, MetricType::Counter, b"second");

        // Assert
        assert_eq!(refused, Err(Status::InternalFailure));
        assert_eq!(
            services.define_metric(call(), VM, MetricType::Counter, b"first"),
            Ok(first),
            "a metric the store holds still opens"
        );
    }

    #[test]
    fn a_queue_past_its_depth_refuses_the_item() {
        // Arrange
        let services =
            InMemoryStore::new().with_limits(InMemoryStoreLimits::new().with_queue_items(1));
        let id = queue(&services, VM);
        services.enqueue_shared_queue(call(), id, b"first").unwrap();

        // Act
        let refused = services.enqueue_shared_queue(call(), id, b"second");

        // Assert
        assert_eq!(refused, Err(Status::InternalFailure));
    }

    #[test]
    fn the_number_never_reaches_zero_when_it_wraps() {
        // Arrange
        let highest = NonZeroU32::new(u32::MAX).unwrap();

        // Act
        let next = next_cas(highest);

        // Assert
        assert_eq!(next.get(), 1);
    }

    #[test]
    fn a_registration_that_repeats_a_name_opens_the_same_queue() {
        // Arrange
        let services = InMemoryStore::new();
        let first = queue(&services, VM);

        // Act
        let second = services.register_shared_queue(call(), VM, b"q");

        // Assert
        assert_eq!(second.unwrap(), first);
    }

    #[test]
    fn a_queue_resolves_through_a_second_holder_of_the_same_value() {
        // Arrange
        let services = Arc::new(InMemoryStore::new());
        let registered = queue(&services, VM);
        let second_instance = Arc::clone(&services);

        // Act
        let resolved = second_instance.resolve_shared_queue(call(), VM, b"q");

        // Assert
        assert_eq!(resolved.unwrap(), registered);
    }

    #[test]
    fn a_queue_no_vm_registered_is_not_found() {
        // Arrange
        let services = InMemoryStore::new();

        // Act
        let resolved = services.resolve_shared_queue(call(), VM, b"q");

        // Assert
        assert_eq!(resolved, Err(Status::NotFound));
    }

    #[test]
    fn an_item_moves_through_the_queue_in_order() {
        // Arrange
        let services = InMemoryStore::new();
        let id = queue(&services, VM);
        services.enqueue_shared_queue(call(), id, b"first").unwrap();
        services
            .enqueue_shared_queue(call(), id, b"second")
            .unwrap();

        // Act
        let taken = [
            services.dequeue_shared_queue(call(), id),
            services.dequeue_shared_queue(call(), id),
            services.dequeue_shared_queue(call(), id),
        ];

        // Assert
        assert_eq!(taken[0].as_deref(), Ok(b"first".as_slice()));
        assert_eq!(taken[1].as_deref(), Ok(b"second".as_slice()));
        assert_eq!(taken[2], Err(Status::Empty));
    }

    #[test]
    fn a_queue_that_is_not_there_is_not_found() {
        // Arrange
        let services = InMemoryStore::new();
        let absent = QueueId::try_from(9u32).unwrap();

        // Act
        let results = (
            services.enqueue_shared_queue(call(), absent, b"v"),
            services.dequeue_shared_queue(call(), absent),
        );

        // Assert
        assert_eq!(results.0, Err(Status::NotFound));
        assert_eq!(results.1, Err(Status::NotFound));
    }

    #[test]
    fn a_metric_carries_a_value_above_the_thirty_two_bit_limit() {
        // Arrange
        let services = InMemoryStore::new();
        let metric = services
            .define_metric(call(), VM, MetricType::Gauge, b"bytes")
            .unwrap();
        let large = u64::from(u32::MAX) + 7;

        // Act
        let recorded = services.record_metric(call(), metric, large);

        // Assert
        assert_eq!(recorded, Ok(()));
        assert_eq!(services.get_metric(call(), metric), Ok(large));
    }

    #[test]
    fn a_metric_of_one_vm_is_not_the_metric_of_another() {
        // Arrange
        let services = InMemoryStore::new();
        let mine = counter(&services);

        // Act
        let theirs = services.define_metric(call(), OTHER_VM, MetricType::Counter, b"requests");

        // Assert
        assert_ne!(theirs.unwrap(), mine);
    }

    #[test]
    fn a_name_defined_twice_with_one_kind_is_one_metric() {
        // Arrange
        let services = InMemoryStore::new();
        let first = counter(&services);

        // Act
        let second = services.define_metric(call(), VM, MetricType::Counter, b"requests");

        // Assert
        assert_eq!(second.unwrap(), first);
    }

    #[test]
    fn a_name_defined_with_another_kind_is_a_bad_argument() {
        // Arrange
        let services = InMemoryStore::new();
        counter(&services);

        // Act
        let again = services.define_metric(call(), VM, MetricType::Gauge, b"requests");

        // Assert
        assert_eq!(again, Err(Status::BadArgument));
    }

    #[test]
    fn a_counter_refuses_a_negative_delta() {
        // Arrange
        let services = InMemoryStore::new();
        let metric = counter(&services);
        services.increment_metric(call(), metric, 5).unwrap();

        // Act
        let refused = services.increment_metric(call(), metric, -1);

        // Assert
        assert_eq!(refused, Err(Status::BadArgument));
        assert_eq!(services.get_metric(call(), metric), Ok(5));
    }

    #[test]
    fn a_gauge_that_would_go_below_zero_is_refused() {
        // Arrange
        let services = InMemoryStore::new();
        let metric = services
            .define_metric(call(), VM, MetricType::Gauge, b"live")
            .unwrap();
        services.record_metric(call(), metric, 1).unwrap();

        // Act
        let refused = services.increment_metric(call(), metric, -2);

        // Assert
        assert_eq!(refused, Err(Status::BadArgument));
        assert_eq!(services.get_metric(call(), metric), Ok(1));
    }

    #[test]
    fn a_gauge_accepts_a_negative_delta() {
        // Arrange
        let services = InMemoryStore::new();
        let metric = services
            .define_metric(call(), VM, MetricType::Gauge, b"live")
            .unwrap();
        services.record_metric(call(), metric, 5).unwrap();

        // Act
        let changed = services.increment_metric(call(), metric, -2);

        // Assert
        assert_eq!(changed, Ok(()));
        assert_eq!(services.get_metric(call(), metric), Ok(3));
    }

    #[test]
    fn a_histogram_takes_an_observation_and_refuses_the_other_two() {
        // Arrange
        let services = InMemoryStore::new();
        let metric = services
            .define_metric(call(), VM, MetricType::Histogram, b"latency")
            .unwrap();
        services.record_metric(call(), metric, 12).unwrap();

        // Act
        let results = (
            services.increment_metric(call(), metric, 1),
            services.get_metric(call(), metric),
        );

        // Assert
        assert_eq!(results.0, Err(Status::BadArgument));
        assert_eq!(results.1, Err(Status::BadArgument));
    }

    #[test]
    fn a_metric_that_is_not_there_is_not_found() {
        // Arrange
        let services = InMemoryStore::new();
        let absent = MetricId::try_from(9u32).unwrap();

        // Act
        let results = [
            services.record_metric(call(), absent, 1).err(),
            services.increment_metric(call(), absent, 1).err(),
            services.get_metric(call(), absent).err(),
        ];

        // Assert
        assert_eq!(results, [Some(Status::NotFound); 3]);
    }
}
