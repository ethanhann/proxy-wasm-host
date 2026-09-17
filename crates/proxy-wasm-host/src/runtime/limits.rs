//! The per instance resource limits.

use std::time::Duration;

const DEFAULT_CPU_TIME: Duration = Duration::from_secs(1);
const DEFAULT_MEMORY_BYTES: usize = 128 * 1024 * 1024;

/// The resources one instance may use.
///
/// The CPU time applies to each guest call, because the budget is refilled
/// before every call.
/// The memory ceiling applies to the instance's linear memory.
/// A guest that grows past it sees `memory.grow` fail.
/// The struct is non exhaustive, so build it with [`Limits::new`] and the
/// `with_*` methods.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Limits {
    cpu_time: Duration,
    fuel: Option<u64>,
    memory_bytes: Option<usize>,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            cpu_time: DEFAULT_CPU_TIME,
            fuel: None,
            memory_bytes: Some(DEFAULT_MEMORY_BYTES),
        }
    }
}

impl Limits {
    /// One second of CPU time per call, no fuel, and a 128 MiB memory ceiling.
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
