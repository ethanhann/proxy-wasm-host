//! Which worker a queue item wakes.

use std::collections::HashMap;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, PoisonError};

use proxy_wasm_host::abi::v0_2_1::{ContextId, QueueEnqueued, QueueId};

use crate::worker::Job;

/// The workers that registered each queue, in the order they registered.
///
/// An item wakes the worker that registered last, as the C++ host does.
/// The earlier workers stay in the list, so an item still reaches a guest when
/// the last worker stops.
#[derive(Clone, Default)]
pub struct QueueRoutes(Arc<Mutex<Registrants>>);

/// The workers and the roots that registered each queue.
type Registrants = HashMap<QueueId, Vec<(usize, ContextId)>>;

impl QueueRoutes {
    /// Records that `worker` serves `queue` on `root`.
    ///
    /// A worker that registers again moves to the end of the list, so it
    /// receives the next item.
    pub fn register(&self, queue: QueueId, worker: usize, root: ContextId) {
        let mut table = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let list = table.entry(queue).or_default();
        list.retain(|(index, _)| *index != worker);
        list.push((worker, root));
        tracing::info!("queue {queue:?} now wakes worker {worker}");
    }

    /// Drops every queue of `worker`, which stopped.
    pub fn forget(&self, worker: usize) {
        let mut table = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        for (queue, list) in table.iter_mut() {
            let before = list.len();
            list.retain(|(index, _)| *index != worker);
            if list.len() != before {
                tracing::info!("queue {queue:?} no longer wakes worker {worker}");
            }
        }
    }

    /// The worker and the root that take the next item of `queue`.
    ///
    /// The lock is held for this lookup alone, and never across a call of a
    /// guest.
    pub fn route(&self, queue: QueueId) -> Option<(usize, ContextId)> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&queue)
            .and_then(|list| list.last())
            .copied()
    }
}

/// The closure the store calls for each item.
///
/// The store calls it on the thread that enqueued, with the guest still on the
/// stack, so it must not block and must not call a guest.
/// The channel is therefore unbounded, and a real proxy bounds the work where
/// the requests arrive.
pub fn observer(
    routes: QueueRoutes,
    senders: Vec<Sender<Job>>,
) -> Arc<dyn Fn(QueueEnqueued<'_>) + Send + Sync> {
    Arc::new(move |item: QueueEnqueued<'_>| {
        let Some((index, root)) = routes.route(item.queue) else {
            tracing::warn!("queue {:?} has no route, and the item waits", item.queue);
            return;
        };
        let job = Job::QueueReady {
            queue: item.queue,
            root,
        };
        if senders[index].send(job).is_err() {
            tracing::warn!("worker {index} has stopped, and the item waits");
        }
    })
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::channel;

    use proxy_wasm_host::abi::v0_2_1::{GuestId, InMemoryStore, Invocation, SharedServices};

    use super::*;

    fn context(id: u32) -> ContextId {
        ContextId::try_from(id).unwrap()
    }

    fn queue(id: u32) -> QueueId {
        QueueId::try_from(id).unwrap()
    }

    #[test]
    fn a_route_is_found_by_its_queue() {
        // Arrange
        let routes = QueueRoutes::default();
        routes.register(queue(1), 2, context(1));

        // Act
        let found = routes.route(queue(1));

        // Assert
        assert_eq!(found, Some((2, context(1))));
    }

    #[test]
    fn a_later_registration_takes_the_next_item() {
        // Arrange
        let routes = QueueRoutes::default();
        routes.register(queue(1), 0, context(1));

        // Act
        routes.register(queue(1), 3, context(7));

        // Assert
        assert_eq!(routes.route(queue(1)), Some((3, context(7))));
    }

    #[test]
    fn a_worker_that_registers_again_takes_the_next_item() {
        // Arrange
        let routes = QueueRoutes::default();
        routes.register(queue(1), 0, context(1));
        routes.register(queue(1), 1, context(1));

        // Act
        routes.register(queue(1), 0, context(9));

        // Assert
        assert_eq!(routes.route(queue(1)), Some((0, context(9))));
    }

    #[test]
    fn a_worker_that_stops_gives_the_queue_back_to_an_earlier_worker() {
        // Arrange
        let routes = QueueRoutes::default();
        routes.register(queue(1), 0, context(1));
        routes.register(queue(1), 3, context(1));

        // Act
        routes.forget(3);

        // Assert
        assert_eq!(routes.route(queue(1)), Some((0, context(1))));
    }

    #[test]
    fn a_queue_with_no_worker_left_has_no_route() {
        // Arrange
        let routes = QueueRoutes::default();
        routes.register(queue(1), 0, context(1));

        // Act
        routes.forget(0);

        // Assert
        assert_eq!(routes.route(queue(1)), None);
    }

    #[test]
    fn an_unknown_queue_has_no_route() {
        // Arrange
        let routes = QueueRoutes::default();
        routes.register(queue(1), 0, context(1));

        // Act
        let found = routes.route(queue(2));

        // Assert
        assert_eq!(found, None);
    }

    #[test]
    fn the_observer_sends_one_job_to_the_worker_that_registered_last() {
        // Arrange
        let routes = QueueRoutes::default();
        let (first, unused) = channel();
        let (second, wanted) = channel();
        let store = InMemoryStore::new()
            .with_enqueue_observer(observer(routes.clone(), vec![first, second]));
        let call = Invocation::new(GuestId::next(), context(1));
        let queue = store
            .register_shared_queue(call, b"example", b"paths")
            .unwrap();
        routes.register(queue, 0, context(1));
        routes.register(queue, 1, context(4));

        // Act
        let enqueued = store.enqueue_shared_queue(call, queue, b"/one");

        // Assert
        enqueued.unwrap();
        assert!(unused.try_recv().is_err());
        let job = wanted.try_recv();
        assert!(
            matches!(job, Ok(Job::QueueReady { queue: woken, root }) if woken == queue && root == context(4))
        );
    }
}
