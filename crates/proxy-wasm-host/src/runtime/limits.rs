//! The per instance resource limits.

use std::time::Duration;

use crate::codec::pairs::{DEFAULT_MAX_DECODED_MAP_BYTES, DEFAULT_MAX_DECODED_PAIRS, PairLimits};

const DEFAULT_CPU_TIME: Duration = Duration::from_secs(1);
const DEFAULT_MEMORY_BYTES: usize = 128 * 1024 * 1024;

/// The resources one instance may use.
///
/// The CPU time applies to each guest call, because the budget is refilled
/// before every call.
/// The memory ceiling applies to the instance's linear memory.
/// A guest that grows past it sees `memory.grow` fail.
/// The two decode limits apply to each map a guest sends to the host, which
/// a guest writes and therefore sizes.
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
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            cpu_time: DEFAULT_CPU_TIME,
            fuel: None,
            memory_bytes: Some(DEFAULT_MEMORY_BYTES),
            max_decoded_pairs: Some(DEFAULT_MAX_DECODED_PAIRS),
            max_decoded_map_bytes: Some(DEFAULT_MAX_DECODED_MAP_BYTES),
        }
    }
}

impl Limits {
    /// One second of CPU time per call, no fuel, a 128 MiB memory ceiling,
    /// and the two decode limits of the C++ host, which are 1024 pairs and
    /// 1 MiB for one map a guest sends.
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
