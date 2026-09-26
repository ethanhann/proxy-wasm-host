//! The time a guest reads, and the clock the crate uses when you set none.

use std::sync::OnceLock;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// The time source for the WASI `clock_time_get` function and for
/// `proxy_get_current_time_nanoseconds`.
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn the_system_clock_moves_between_two_reads() {
        // Arrange
        let clock = SystemClock;
        let first = clock.monotonic_nanos();
        std::thread::sleep(Duration::from_millis(10));

        // Act
        let second = clock.monotonic_nanos();

        // Assert
        assert!(second - first >= 10_000_000, "{first} then {second}");
        assert!(clock.realtime_nanos() > 1_600_000_000_000_000_000);
    }
}
