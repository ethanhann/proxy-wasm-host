//! The data stored in every instance's wasmtime store.

use std::sync::{Arc, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use wasmtime::{Memory, StoreLimits, TypedFunc};

use crate::abi::v0_2_1::types::LogLevel;

/// Where guest log output goes.
///
/// The WASI `fd_write` function and the `proxy_log` host function both call
/// this.
/// One sink usually serves a whole process, so the runtime holds it in an
/// `Arc` and calls it through a shared reference.
pub trait LogSink: Send + Sync {
    /// Records one message at one level.
    fn log(&self, level: LogLevel, message: &[u8]);
}

/// The time source for the WASI `clock_time_get` function.
///
/// A host may return approximate or frozen time.
/// A test can therefore install a clock with fixed values.
pub trait Clock: Send + Sync {
    /// Nanoseconds since the Unix epoch.
    fn realtime_nanos(&self) -> u64;
    /// Nanoseconds since an origin that never moves while the process runs.
    fn monotonic_nanos(&self) -> u64;
}

/// The clock that reads the operating system.
///
/// The monotonic origin is one `Instant` for the whole process.
/// Every instance therefore reports comparable monotonic values.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

fn monotonic_origin() -> Instant {
    static ORIGIN: OnceLock<Instant> = OnceLock::new();
    *ORIGIN.get_or_init(Instant::now)
}

fn nanos(duration: std::time::Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

impl Clock for SystemClock {
    fn realtime_nanos(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, nanos)
    }

    fn monotonic_nanos(&self) -> u64 {
        nanos(monotonic_origin().elapsed())
    }
}

/// The services you supply to a guest: a log sink, a clock, and environment
/// variables.
///
/// Build one per instance and pass it to [`crate::runtime::Instance::new`].
/// You can change it between calls through
/// [`crate::runtime::Instance::services_mut`].
pub struct HostServices {
    log: Arc<dyn LogSink>,
    clock: Arc<dyn Clock>,
    environment: Vec<(Vec<u8>, Vec<u8>)>,
}

impl HostServices {
    /// Services that log to `log`, read [`SystemClock`], and have no
    /// environment variables.
    pub fn new(log: Arc<dyn LogSink>) -> Self {
        Self {
            log,
            clock: Arc::new(SystemClock),
            environment: Vec::new(),
        }
    }

    /// Replaces the clock.
    #[must_use]
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    /// Sets the environment variables that the guest sees through WASI.
    ///
    /// The ABI document says these must be configured per guest.
    /// They are never read from the process environment.
    #[must_use]
    pub fn with_environment(mut self, variables: Vec<(Vec<u8>, Vec<u8>)>) -> Self {
        self.environment = variables;
        self
    }

    /// The log sink.
    pub fn log(&self) -> &dyn LogSink {
        self.log.as_ref()
    }

    /// The clock.
    pub fn clock(&self) -> &dyn Clock {
        self.clock.as_ref()
    }

    /// The environment variables, in the order given.
    pub fn environment(&self) -> &[(Vec<u8>, Vec<u8>)] {
        &self.environment
    }
}

/// The store data of one instance.
///
/// The runtime keeps the cached memory handle, the guest allocator, the
/// store limits, and the poison flag here, next to the services the embedder
/// supplied.
/// The type is crate private, so nothing outside the crate can clear the
/// poison flag or replace the cached handles.
pub(crate) struct HostState {
    services: HostServices,
    store_limits: StoreLimits,
    memory: Option<Memory>,
    allocator: Option<TypedFunc<i32, i32>>,
    poisoned: bool,
}

impl HostState {
    pub(crate) fn new(services: HostServices) -> Self {
        Self {
            services,
            store_limits: StoreLimits::default(),
            memory: None,
            allocator: None,
            poisoned: false,
        }
    }

    pub(crate) fn services(&self) -> &HostServices {
        &self.services
    }

    pub(crate) fn services_mut(&mut self) -> &mut HostServices {
        &mut self.services
    }

    pub(crate) fn is_poisoned(&self) -> bool {
        self.poisoned
    }

    pub(crate) fn memory(&self) -> Option<Memory> {
        self.memory
    }

    pub(crate) fn allocator(&self) -> Option<TypedFunc<i32, i32>> {
        self.allocator.clone()
    }

    pub(crate) fn store_limits(&mut self) -> &mut StoreLimits {
        &mut self.store_limits
    }

    pub(crate) fn set_memory(&mut self, memory: Memory) {
        self.memory = Some(memory);
    }

    pub(crate) fn set_allocator(&mut self, allocator: TypedFunc<i32, i32>) {
        self.allocator = Some(allocator);
    }

    pub(crate) fn set_store_limits(&mut self, limits: StoreLimits) {
        self.store_limits = limits;
    }

    pub(crate) fn poison(&mut self) {
        self.poisoned = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::test_support::RecordingSink;

    #[test]
    fn system_clock_is_monotonic_and_shares_its_origin() {
        // Arrange
        let first = SystemClock;
        let second = SystemClock;

        // Act
        let readings = [
            first.monotonic_nanos(),
            second.monotonic_nanos(),
            first.monotonic_nanos(),
        ];

        // Assert
        assert!(readings[0] <= readings[1]);
        assert!(readings[1] <= readings[2]);
        assert!(readings[2] - readings[0] < 1_000_000_000);
    }

    #[test]
    fn system_clock_real_time_is_after_2020() {
        // Arrange
        let clock = SystemClock;
        let year_2020_nanos = 1_577_836_800_u64 * 1_000_000_000;

        // Act
        let now = clock.realtime_nanos();

        // Assert
        assert!(now > year_2020_nanos);
    }

    #[test]
    fn with_environment_keeps_the_order() {
        // Arrange
        let variables = vec![
            (b"B".to_vec(), b"2".to_vec()),
            (b"A".to_vec(), b"1".to_vec()),
        ];

        // Act
        let services = HostServices::new(Arc::new(RecordingSink::default()))
            .with_environment(variables.clone());

        // Assert
        assert_eq!(services.environment(), variables.as_slice());
    }

    #[test]
    fn poison_is_observable() {
        // Arrange
        let mut state = HostState::new(HostServices::new(Arc::new(RecordingSink::default())));

        // Act
        state.poison();

        // Assert
        assert!(state.is_poisoned());
    }
}
