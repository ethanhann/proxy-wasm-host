//! The identifiers the shared services hand a guest.

use std::fmt;
use std::num::NonZeroU32;

macro_rules! shared_id {
    ($name:ident, $invalid:ident, $what:literal, $message:literal) => {
        #[doc = concat!("The identifier of one ", $what, ".")]
        ///
        /// The crate's implementation allocates these, and an implementation
        /// of your own allocates its own.
        /// Zero is never one, because the ABI uses it for an absent value.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(NonZeroU32);

        impl $name {
            /// The identifier as the ABI carries it.
            pub fn get(self) -> u32 {
                self.0.get()
            }

            pub(crate) fn from_non_zero(value: NonZeroU32) -> Self {
                Self(value)
            }
        }

        impl TryFrom<u32> for $name {
            type Error = $invalid;

            fn try_from(value: u32) -> Result<Self, $invalid> {
                NonZeroU32::new(value).map(Self).ok_or($invalid { value })
            }
        }

        impl TryFrom<i32> for $name {
            type Error = $invalid;

            /// Reads the raw bits as unsigned, because the ABI types the
            /// argument as unsigned, and rejects zero.
            fn try_from(value: i32) -> Result<Self, $invalid> {
                Self::try_from(value.cast_unsigned())
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        #[doc = concat!("A value that cannot be a ", $what, " identifier.")]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
        #[error($message)]
        pub struct $invalid {
            /// The value that was rejected.
            pub value: u32,
        }
    };
}

shared_id!(
    QueueId,
    InvalidQueueId,
    "queue",
    "{value} is not a valid queue identifier"
);
shared_id!(
    MetricId,
    InvalidMetricId,
    "metric",
    "{value} is not a valid metric identifier"
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_identifier_reads_its_bits_as_unsigned_and_rejects_zero() {
        // Arrange
        let values = [1, -1, 0];

        // Act
        let results = values.map(QueueId::try_from);

        // Assert
        assert_eq!(results[0].unwrap().get(), 1);
        assert_eq!(results[1].unwrap().get(), u32::MAX);
        assert_eq!(results[2], Err(InvalidQueueId { value: 0 }));
    }

    #[test]
    fn an_identifier_displays_itself_and_reports_its_value() {
        // Arrange
        let id = MetricId::try_from(7u32).unwrap();

        // Act
        let shown = format!("{id}");

        // Assert
        assert_eq!(shown, "7");
        assert_eq!(id.get(), 7);
    }

    #[test]
    fn the_invalid_identifier_errors_name_their_kind() {
        // Arrange
        let queue = InvalidQueueId { value: 0 };
        let metric = InvalidMetricId { value: 0 };

        // Act
        let messages = (queue.to_string(), metric.to_string());

        // Assert
        assert_eq!(messages.0, "0 is not a valid queue identifier");
        assert_eq!(messages.1, "0 is not a valid metric identifier");
    }
}
