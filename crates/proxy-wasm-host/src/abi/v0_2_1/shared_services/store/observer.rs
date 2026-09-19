//! How the store tells an embedder that a queue got an item.

use std::fmt;
use std::sync::Arc;

use crate::abi::v0_2_1::QueueId;

/// One item that a queue of an [`InMemoryStore`](super::InMemoryStore) got.
///
/// The name and the VM id are the pair that a guest registered the queue
/// with, so you can find the guests that wait for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct QueueEnqueued<'a> {
    /// The VM id of the guest that registered the queue.
    pub vm_id: &'a [u8],
    /// The name the queue was registered with.
    pub name: &'a [u8],
    /// The identifier of the queue.
    pub queue: QueueId,
}

/// The function an embedder gave to hear of every item.
#[derive(Clone)]
pub(super) struct Observer(pub(super) Arc<dyn Fn(QueueEnqueued<'_>) + Send + Sync>);

impl fmt::Debug for Observer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Observer")
    }
}

impl super::InMemoryStore {
    /// Sets a function the store calls one time for each item a queue gets.
    ///
    /// Sometimes a guest registers a queue and waits for `proxy_on_queue_ready`.
    /// An item can come from another guest on another thread, and that thread
    /// cannot call the guest that waits, so the store tells you, and you call
    /// [`CallScope::on_queue_ready`](crate::abi::v0_2_1::CallScope::on_queue_ready)
    /// on the thread that owns the guest.
    ///
    /// The function runs on the thread that enqueued, after the item is
    /// visible and after the store released its lock, so it may read the
    /// store.
    /// It must not block and must not call a guest.
    /// A store with no function reports nothing, and it warns through
    /// `tracing` the first time a guest registers a queue.
    #[must_use]
    pub fn with_enqueue_observer(
        mut self,
        observer: Arc<dyn Fn(QueueEnqueued<'_>) + Send + Sync>,
    ) -> Self {
        self.observer = Some(Observer(observer));
        self
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use crate::abi::v0_2_1::test_support::call;
    use crate::abi::v0_2_1::test_support::events::warnings;
    use crate::abi::v0_2_1::types::Status;
    use crate::abi::v0_2_1::{InMemoryStore, QueueId, SharedServices};

    #[test]
    fn the_store_reports_each_item_with_the_owner_and_the_name() {
        // Arrange
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&seen);
        let store = Arc::new(
            InMemoryStore::new().with_enqueue_observer(Arc::new(move |item| {
                sink.lock()
                    .unwrap()
                    .push((item.vm_id.to_vec(), item.name.to_vec(), item.queue));
            })),
        );
        let call = call();
        let queue = store.register_shared_queue(call, b"vm", b"q").unwrap();
        let absent = QueueId::try_from(99_u32).unwrap();

        // Act
        let answers = [
            store.enqueue_shared_queue(call, queue, b"one"),
            store.enqueue_shared_queue(call, queue, b"two"),
            store.enqueue_shared_queue(call, absent, b"lost"),
        ];

        // Assert
        assert_eq!(answers, [Ok(()), Ok(()), Err(Status::NotFound)]);
        let expected = (b"vm".to_vec(), b"q".to_vec(), queue);
        assert_eq!(*seen.lock().unwrap(), [expected.clone(), expected]);
    }

    #[test]
    fn an_observer_that_reads_the_store_returns() {
        // Arrange
        let call = call();
        let taken = Arc::new(Mutex::new(Vec::new()));
        let store: Arc<Mutex<Option<Arc<InMemoryStore>>>> = Arc::new(Mutex::new(None));
        let (inner, sink) = (Arc::clone(&store), Arc::clone(&taken));
        let built = Arc::new(
            InMemoryStore::new().with_enqueue_observer(Arc::new(move |item| {
                let store = inner.lock().unwrap().clone();
                if let Some(store) = store {
                    sink.lock()
                        .unwrap()
                        .push(store.dequeue_shared_queue(call, item.queue));
                }
            })),
        );
        *store.lock().unwrap() = Some(Arc::clone(&built));
        let queue = built.register_shared_queue(call, b"vm", b"q").unwrap();

        // Act
        let answer = built.enqueue_shared_queue(call, queue, b"item");

        // Assert
        assert_eq!(answer, Ok(()));
        assert_eq!(*taken.lock().unwrap(), [Ok(b"item".to_vec())]);
    }

    #[test]
    fn a_store_with_no_observer_warns_at_the_first_registration_only() {
        // Arrange
        let silent = InMemoryStore::new();
        let heard = InMemoryStore::new().with_enqueue_observer(Arc::new(|_| {}));
        let register = |store: &InMemoryStore| {
            for name in [b"a", b"b", b"a"] {
                store.register_shared_queue(call(), b"vm", name).unwrap();
            }
        };

        // Act
        let counts = [
            warnings(|| register(&silent)),
            warnings(|| register(&heard)),
        ];

        // Assert
        assert_eq!(counts, [1, 0]);
    }
}
