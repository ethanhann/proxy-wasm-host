//! What the in memory shared services allow a guest to store.

/// What [`InMemoryStore`] allows a guest to store.
///
/// The default allows 4096 keys of at most 64 KiB each, 1024 items on a
/// queue, 4096 queues, and 4096 metrics, which bounds a guest that writes and
/// does not read.
///
/// The store outlives every guest that uses it, so these limits hold across
/// guests.
/// A guest that registers new queue names and then traps leaves those
/// queues here, and the guest built after it cannot add more than the limit
/// allows.
/// A name the store already holds still opens at the limit.
///
/// [`InMemoryStore`]: super::InMemoryStore
///
/// ```
/// use proxy_wasm_host::abi::v0_2_1::{InMemoryStore, InMemoryStoreLimits};
///
/// let limits = InMemoryStoreLimits::new().with_value_bytes(4 * 1024);
/// let services = InMemoryStore::new().with_limits(limits);
/// assert_eq!(limits.value_bytes(), 4 * 1024);
/// # let _ = services;
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct InMemoryStoreLimits {
    value_bytes: usize,
    keys: usize,
    queue_items: usize,
    queues: usize,
    metrics: usize,
}

impl Default for InMemoryStoreLimits {
    fn default() -> Self {
        Self {
            value_bytes: 64 * 1024,
            keys: 4096,
            queue_items: 1024,
            queues: 4096,
            metrics: 4096,
        }
    }
}

impl InMemoryStoreLimits {
    /// The limits described on the type.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the largest value or item, in bytes.
    #[must_use]
    pub fn with_value_bytes(mut self, value_bytes: usize) -> Self {
        self.value_bytes = value_bytes;
        self
    }

    /// Sets the number of keys the shared data holds.
    #[must_use]
    pub fn with_keys(mut self, keys: usize) -> Self {
        self.keys = keys;
        self
    }

    /// Sets the number of items one queue holds.
    #[must_use]
    pub fn with_queue_items(mut self, queue_items: usize) -> Self {
        self.queue_items = queue_items;
        self
    }

    /// Sets the number of queues the store holds, for every VM together.
    #[must_use]
    pub fn with_queues(mut self, queues: usize) -> Self {
        self.queues = queues;
        self
    }

    /// Sets the number of metrics the store holds, for every VM together.
    #[must_use]
    pub fn with_metrics(mut self, metrics: usize) -> Self {
        self.metrics = metrics;
        self
    }

    /// The largest value or item, in bytes.
    pub fn value_bytes(&self) -> usize {
        self.value_bytes
    }

    /// The number of keys the shared data holds.
    pub fn keys(&self) -> usize {
        self.keys
    }

    /// The number of items one queue holds.
    pub fn queue_items(&self) -> usize {
        self.queue_items
    }

    /// The number of queues the store holds.
    pub fn queues(&self) -> usize {
        self.queues
    }

    /// The number of metrics the store holds.
    pub fn metrics(&self) -> usize {
        self.metrics
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_returns_the_default_limits() {
        // Arrange
        let default = InMemoryStoreLimits::default();

        // Act
        let built = InMemoryStoreLimits::new();

        // Assert
        assert_eq!(built, default);
        assert_eq!(built.value_bytes(), 64 * 1024);
        assert_eq!(built.keys(), 4096);
        assert_eq!(built.queue_items(), 1024);
        assert_eq!(built.queues(), 4096);
        assert_eq!(built.metrics(), 4096);
    }

    #[test]
    fn each_builder_method_is_reported_by_its_getter() {
        // Arrange
        let base = InMemoryStoreLimits::new();

        // Act
        let built = base
            .with_value_bytes(4)
            .with_keys(5)
            .with_queue_items(6)
            .with_queues(7)
            .with_metrics(8);

        // Assert
        assert_eq!(built.value_bytes(), 4);
        assert_eq!(built.keys(), 5);
        assert_eq!(built.queue_items(), 6);
        assert_eq!(built.queues(), 7);
        assert_eq!(built.metrics(), 8);
    }
}
