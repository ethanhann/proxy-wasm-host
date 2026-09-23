//! The per instance resource limits.

use std::time::Duration;

use crate::codec::pairs::{DEFAULT_MAX_DECODED_MAP_BYTES, DEFAULT_MAX_DECODED_PAIRS, PairLimits};

const DEFAULT_CPU_TIME: Duration = Duration::from_secs(1);
const DEFAULT_MEMORY_BYTES: usize = 128 * 1024 * 1024;
const DEFAULT_MAX_SHARED_NAMES: usize = 1024;
const DEFAULT_MAX_NAME_BYTES: usize = 4096;
const DEFAULT_MAX_LOG_BYTES: usize = 1024 * 1024;

/// The resources one instance may use.
///
/// The CPU time applies to each guest call, because the budget is refilled
/// before every call.
/// The memory ceiling applies to the instance's linear memory.
/// A guest that grows past it sees `memory.grow` fail.
/// The two decode limits apply to each map a guest sends to the host, which
/// a guest writes and therefore sizes.
/// The last three bound the queues and the metrics one guest holds, the
/// bytes of one name or key a guest sends, and the bytes of one line a guest
/// logs.
/// The struct is non exhaustive, so build it with [`Limits::new`] and the
/// `with_*` methods.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Limits {
    cpu_time: Duration,
    fuel: Option<u64>,
    memory_bytes: Option<usize>,
    max_decoded_pairs: Option<u32>,
    max_decoded_map_bytes: Option<usize>,
    max_shared_names: Option<usize>,
    max_name_bytes: Option<usize>,
    max_log_bytes: Option<usize>,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            cpu_time: DEFAULT_CPU_TIME,
            fuel: None,
            memory_bytes: Some(DEFAULT_MEMORY_BYTES),
            max_decoded_pairs: Some(DEFAULT_MAX_DECODED_PAIRS),
            max_decoded_map_bytes: Some(DEFAULT_MAX_DECODED_MAP_BYTES),
            max_shared_names: Some(DEFAULT_MAX_SHARED_NAMES),
            max_name_bytes: Some(DEFAULT_MAX_NAME_BYTES),
            max_log_bytes: Some(DEFAULT_MAX_LOG_BYTES),
        }
    }
}

impl Limits {
    /// One second of CPU time per call, no fuel, a 128 MiB memory ceiling,
    /// the two decode limits of the C++ host, which are 1024 pairs and 1 MiB
    /// for one map a guest sends, 1024 shared names, 4096 bytes for one name
    /// or key, and 1 MiB for one log line.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the CPU time each guest call may use.
    #[must_use]
    pub fn with_cpu_time(mut self, cpu_time: Duration) -> Self {
        self.cpu_time = cpu_time;
        self
    }

    /// Sets the fuel budget each guest call may burn.
    ///
    /// The engine must have fuel enabled, or instantiation fails.
    #[must_use]
    pub fn with_fuel(mut self, fuel: impl Into<Option<u64>>) -> Self {
        self.fuel = fuel.into();
        self
    }

    /// Sets the memory ceiling in bytes, or removes it with `None`.
    #[must_use]
    pub fn with_memory_bytes(mut self, memory_bytes: impl Into<Option<usize>>) -> Self {
        self.memory_bytes = memory_bytes.into();
        self
    }

    /// Sets the most pairs one map a guest sends may declare, or removes the
    /// limit with `None`.
    ///
    /// The default is 1024, which is the value the C++ host uses.
    /// A guest that sends a larger map receives `BAD_ARGUMENT`, and
    /// `PARSE_FAILURE` from the two functions that open a gRPC callout.
    #[must_use]
    pub fn with_max_decoded_pairs(mut self, max_decoded_pairs: impl Into<Option<u32>>) -> Self {
        self.max_decoded_pairs = max_decoded_pairs.into();
        self
    }

    /// Sets the most bytes one map a guest sends may hold, or removes the
    /// limit with `None`.
    ///
    /// The default is 1 MiB, which is the value the C++ host uses.
    /// A guest that sends a longer map receives the same status as one that
    /// declares too many pairs.
    #[must_use]
    pub fn with_max_decoded_map_bytes(
        mut self,
        max_decoded_map_bytes: impl Into<Option<usize>>,
    ) -> Self {
        self.max_decoded_map_bytes = max_decoded_map_bytes.into();
        self
    }

    /// Sets how many shared queues and metrics one guest may hold, or removes
    /// the limit with `None`.
    ///
    /// The queues and the metrics share one count, and a guest spends one of
    /// it for each identifier it obtains.
    /// A name the guest already holds costs nothing more, because the count
    /// is of identifiers rather than of calls.
    /// The default is 1024.
    ///
    /// A guest at the limit receives `INTERNAL_FAILURE` from
    /// `proxy_register_shared_queue`, `proxy_resolve_shared_queue`, and
    /// `proxy_define_metric`, and the service is not called.
    /// A guest of the Rust SDK stops on that status, which poisons the
    /// instance.
    #[must_use]
    pub fn with_max_shared_names(mut self, max_shared_names: impl Into<Option<usize>>) -> Self {
        self.max_shared_names = max_shared_names.into();
        self
    }

    /// Sets the most bytes one queue name, metric name, or shared data key a
    /// guest sends may hold, or removes the limit with `None`.
    ///
    /// The default is 4096.
    /// The three inputs share one limit, because a name and a key are the
    /// same kind of guest input.
    /// Without it a shared name count bounds nothing, because a guest puts
    /// the bytes in the names rather than in their number.
    ///
    /// A guest over the limit receives `INTERNAL_FAILURE`, and the service is
    /// not called.
    /// A guest of the Rust SDK stops on that status, which poisons the
    /// instance.
    /// A read of a shared data key over the limit answers `NOT_FOUND`, because
    /// no such key can be in the store through this crate, and the Rust SDK
    /// reads that status as a miss.
    #[must_use]
    pub fn with_max_name_bytes(mut self, max_name_bytes: impl Into<Option<usize>>) -> Self {
        self.max_name_bytes = max_name_bytes.into();
        self
    }

    /// Sets the most bytes of one message the log sink receives, or removes
    /// the limit with `None`.
    ///
    /// The default is 1 MiB.
    /// A longer message reaches the sink cut to the limit, and the call
    /// answers `OK`.
    /// A guest reports its own failures through the log, and a guest of the
    /// Rust SDK stops on a refusal, so the crate cuts the message rather
    /// than refuse it.
    /// The limit covers `proxy_log` and the WASI `fd_write` alike.
    ///
    /// With no limit, `proxy_log` gives the sink a view of guest memory, so
    /// a message is at most the size of that memory.
    /// `fd_write` joins the regions a guest lists, and a guest can list one
    /// region many times, so the crate stops the copy at the size of the
    /// guest memory.
    #[must_use]
    pub fn with_max_log_bytes(mut self, max_log_bytes: impl Into<Option<usize>>) -> Self {
        self.max_log_bytes = max_log_bytes.into();
        self
    }

    /// The CPU time each guest call may use.
    pub fn cpu_time(&self) -> Duration {
        self.cpu_time
    }

    /// The fuel budget each guest call may burn.
    pub fn fuel(&self) -> Option<u64> {
        self.fuel
    }

    /// The memory ceiling in bytes.
    pub fn memory_bytes(&self) -> Option<usize> {
        self.memory_bytes
    }

    /// The most pairs one map a guest sends may declare.
    pub fn max_decoded_pairs(&self) -> Option<u32> {
        self.max_decoded_pairs
    }

    /// The most bytes one map a guest sends may hold.
    pub fn max_decoded_map_bytes(&self) -> Option<usize> {
        self.max_decoded_map_bytes
    }

    /// How many shared queues and metrics one guest may hold.
    pub fn max_shared_names(&self) -> Option<usize> {
        self.max_shared_names
    }

    /// The most bytes one queue name, metric name, or shared data key may
    /// hold.
    pub fn max_name_bytes(&self) -> Option<usize> {
        self.max_name_bytes
    }

    /// The most bytes of one message the log sink receives.
    pub fn max_log_bytes(&self) -> Option<usize> {
        self.max_log_bytes
    }

    /// The two decode limits as one value, which
    /// [`decode_pairs`](crate::codec::pairs::decode_pairs) takes.
    ///
    /// Pass it when you decode a map of your own, so your rule and the rule
    /// the crate applies to a guest are the same one.
    pub fn pair_limits(&self) -> PairLimits {
        PairLimits::unlimited()
            .with_pairs(self.max_decoded_pairs)
            .with_bytes(self.max_decoded_map_bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_limits_are_the_values_of_the_cpp_host() {
        // Arrange
        let limits = Limits::default();

        // Act
        let observed = (limits.max_decoded_pairs(), limits.max_decoded_map_bytes());

        // Assert
        assert_eq!(observed, (Some(1024), Some(1024 * 1024)));
        assert_eq!(limits.pair_limits(), PairLimits::default());
    }

    #[test]
    fn the_three_guest_bounds_have_the_documented_defaults() {
        // Arrange
        let limits = Limits::default();

        // Act
        let observed = (
            limits.max_shared_names(),
            limits.max_name_bytes(),
            limits.max_log_bytes(),
        );

        // Assert
        assert_eq!(observed, (Some(1024), Some(4096), Some(1024 * 1024)));
    }

    #[test]
    fn each_guest_bound_reads_back_the_value_it_was_given() {
        // Arrange
        let limits = Limits::new();

        // Act
        let observed = limits
            .with_max_shared_names(4)
            .with_max_name_bytes(8)
            .with_max_log_bytes(None);

        // Assert
        assert_eq!(observed.max_shared_names(), Some(4));
        assert_eq!(observed.max_name_bytes(), Some(8));
        assert_eq!(observed.max_log_bytes(), None);
    }

    #[test]
    fn a_limit_removed_with_none_reads_back_as_none() {
        // Arrange
        let limits = Limits::new();

        // Act
        let observed = limits
            .with_max_decoded_pairs(None)
            .with_max_decoded_map_bytes(None);

        // Assert
        assert_eq!(observed.max_decoded_pairs(), None);
        assert_eq!(observed.max_decoded_map_bytes(), None);
        assert_eq!(observed.pair_limits(), PairLimits::unlimited());
    }

    #[test]
    fn a_raised_limit_reaches_the_pair_limits() {
        // Arrange
        let limits = Limits::new();

        // Act
        let observed = limits.with_max_decoded_pairs(4096);

        // Assert
        assert_eq!(observed.max_decoded_pairs(), Some(4096));
        assert_eq!(observed.pair_limits().pairs(), Some(4096));
        assert_eq!(
            observed.pair_limits().bytes(),
            Some(1024 * 1024),
            "the byte limit must not change"
        );
    }

    #[test]
    fn default_bounds_cpu_and_memory_and_not_fuel() {
        // Arrange
        let limits = Limits::default();

        // Act
        let observed = (limits.cpu_time(), limits.fuel(), limits.memory_bytes());

        // Assert
        assert_eq!(
            observed,
            (Duration::from_secs(1), None, Some(128 * 1024 * 1024))
        );
        assert_eq!(limits, Limits::new());
    }

    #[test]
    fn each_setter_changes_only_its_field() {
        // Arrange
        let base = Limits::new();

        // Act
        let changed = [
            base.clone().with_cpu_time(Duration::from_millis(5)),
            base.clone().with_fuel(10),
            base.clone().with_memory_bytes(None),
        ];

        // Assert
        assert_eq!(changed[0].cpu_time(), Duration::from_millis(5));
        assert_eq!(
            (changed[0].fuel(), changed[0].memory_bytes()),
            (base.fuel(), base.memory_bytes())
        );
        assert_eq!(changed[1].fuel(), Some(10));
        assert_eq!(
            (changed[1].cpu_time(), changed[1].memory_bytes()),
            (base.cpu_time(), base.memory_bytes())
        );
        assert_eq!(changed[2].memory_bytes(), None);
        assert_eq!(
            (changed[2].cpu_time(), changed[2].fuel()),
            (base.cpu_time(), base.fuel())
        );
    }
}
