//! The identity of one guest.

use std::fmt;
use std::num::NonZeroU64;
use std::sync::atomic::{AtomicU64, Ordering};

/// The identity of one guest in this process.
///
/// The callout identifiers of every guest start at one, so a service that
/// serves two guests sees the same [`CalloutId`](crate::abi::v0_2_1::CalloutId) twice.
/// Every method of your
/// [`Callouts`](crate::abi::v0_2_1::Callouts) service receives this on its
/// [`Invocation`](crate::abi::v0_2_1::Invocation), and
/// [`Guest::id`](crate::abi::v0_2_1::Guest::id) answers it, so you key your own record by
/// the guest and the callout.
/// The identity is never reused in one process, so a guest you build after a
/// trap has a new one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GuestId(NonZeroU64);

impl GuestId {
    /// The identity as a number, where the first guest of the process is one.
    pub fn get(self) -> u64 {
        self.0.get()
    }

    /// A fresh identity, which no guest of this process has.
    ///
    /// You use it when you build an
    /// [`Invocation`](crate::abi::v0_2_1::Invocation) to test your own
    /// service or stream state.
    /// The counter is a `u64`, so it does not wrap in the life of a process.
    pub fn next() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let value = NEXT.fetch_add(1, Ordering::Relaxed);
        Self(NonZeroU64::new(value).unwrap_or(NonZeroU64::MIN))
    }
}

impl fmt::Display for GuestId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_identity_of_a_process_differs_from_the_one_before_it() {
        // Arrange
        let first = GuestId::next();

        // Act
        let second = GuestId::next();

        // Assert
        assert_ne!(first, second);
        assert!(second.get() > first.get());
        assert_eq!(second.to_string(), second.get().to_string());
    }
}
