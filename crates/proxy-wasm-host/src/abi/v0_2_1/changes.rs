//! What a guest changed that the embedder acts on.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use crate::abi::v0_2_1::{ContextId, QueueId};

/// What a guest changed since you last asked.
///
/// A guest may set its tick period or register a queue in any callback, and
/// the ABI has no signal for either.
/// Read [`Guest::take_changes`](crate::abi::v0_2_1::Guest::take_changes)
/// after a group of callbacks.
/// It saves you a question to each root.
///
/// The value holds one entry for each root and one for each registration, so
/// its size does not depend on how many times the guest made the same
/// change.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Changes {
    /// The last tick period each root set, with `None` for a period of zero,
    /// which stops the timer.
    pub tick_periods: BTreeMap<ContextId, Option<Duration>>,
    /// Each queue a root registered.
    pub queues: BTreeSet<QueueRegistration>,
}

impl Changes {
    /// Whether the guest changed nothing.
    pub fn is_empty(&self) -> bool {
        self.tick_periods.is_empty() && self.queues.is_empty()
    }
}

/// One queue that a root of a guest registered.
///
/// The VM id of the registration is the one you gave to
/// [`VmServices::with_vm_id`](crate::abi::v0_2_1::VmServices::with_vm_id).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub struct QueueRegistration {
    /// The identifier the shared services gave.
    pub queue: QueueId,
    /// The root whose context registered the queue.
    pub root: ContextId,
    /// The name the guest registered the queue with.
    pub name: Vec<u8>,
}

impl QueueRegistration {
    pub(crate) fn new(queue: QueueId, root: ContextId, name: &[u8]) -> Self {
        Self {
            queue,
            root,
            name: name.to_vec(),
        }
    }
}
