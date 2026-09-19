//! The enumerations of ABI v0.2.1 and their `i32` conversions.
//!
//! Each enum carries the exact values from the Types section of the ABI
//! document.
//! A value crosses the guest boundary as an `i32`, so every enum converts in
//! both directions.
//! `TryFrom<i32>` rejects a value that the ABI document does not list.
//! A rejection converts into [`Status::BadArgument`] for a host function.

use crate::NotAllowed;
use crate::abi::v0_2_1::InvalidContextId;
use crate::codec::pairs::{DecodeError, EncodeError};
use crate::error::MemoryError;

/// An `i32` that is not a listed value of an ABI enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("{value} is not a known {type_name} value")]
pub struct UnknownValue {
    /// The Rust name of the enum.
    pub type_name: &'static str,
    /// The value that was offered.
    pub value: i32,
}

/// Defines one ABI enum with its `i32` conversions and an `ALL` list.
///
/// The definition must stay above the `mod` declarations below.
/// A `macro_rules!` macro is visible only to code that follows it in the file
/// and in the child modules declared after it.
macro_rules! abi_enum {
    (
        $(#[$meta:meta])*
        $name:ident {
            $( $(#[$variant_meta:meta])* $variant:ident = $value:literal ),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[repr(i32)]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $name {
            $( $(#[$variant_meta])* $variant = $value, )+
        }

        impl $name {
            /// Every variant, in the order of the ABI document.
            pub const ALL: &[Self] = &[$(Self::$variant),+];
        }

        impl From<$name> for i32 {
            fn from(value: $name) -> Self {
                value as i32
            }
        }

        impl TryFrom<i32> for $name {
            type Error = $crate::abi::v0_2_1::types::UnknownValue;

            fn try_from(value: i32) -> Result<Self, $crate::abi::v0_2_1::types::UnknownValue> {
                match value {
                    $( $value => Ok(Self::$variant), )+
                    _ => Err($crate::abi::v0_2_1::types::UnknownValue {
                        type_name: stringify!($name),
                        value,
                    }),
                }
            }
        }
    };
}

mod proxy;
mod wasi;

pub use proxy::{Action, BufferType, LogLevel, MapType, MetricType, PeerType, Status, StreamType};
pub use wasi::{WasiClockId, WasiErrno, WasiFdId};

impl From<UnknownValue> for Status {
    fn from(_: UnknownValue) -> Self {
        Self::BadArgument
    }
}

impl From<DecodeError> for Status {
    fn from(_: DecodeError) -> Self {
        Self::BadArgument
    }
}

/// A guest sees every rejected address or range as one status.
impl From<MemoryError> for Status {
    fn from(_: MemoryError) -> Self {
        Self::InvalidMemoryAccess
    }
}

impl From<EncodeError> for Status {
    fn from(_: EncodeError) -> Self {
        Self::SerializationFailure
    }
}

impl From<NotAllowed> for Status {
    fn from(_: NotAllowed) -> Self {
        Self::BadArgument
    }
}

impl From<InvalidContextId> for Status {
    fn from(_: InvalidContextId) -> Self {
        Self::BadArgument
    }
}

#[cfg(test)]
pub(super) mod test_support {
    use super::UnknownValue;

    /// Converts each table row in both directions.
    pub(crate) fn round_trip<E>(table: &[(E, i32)]) -> Vec<(i32, Result<E, UnknownValue>)>
    where
        E: Copy + Into<i32> + TryFrom<i32, Error = UnknownValue>,
    {
        table
            .iter()
            .map(|&(variant, value)| (variant.into(), E::try_from(value)))
            .collect()
    }

    /// The result that a correct table produces from `round_trip`.
    pub(crate) fn expected<E: Copy>(table: &[(E, i32)]) -> Vec<(i32, Result<E, UnknownValue>)> {
        table
            .iter()
            .map(|&(variant, value)| (value, Ok(variant)))
            .collect()
    }

    /// The errors that `values` must produce for the enum named `type_name`.
    pub(crate) fn unknown<E>(
        type_name: &'static str,
        values: &[i32],
    ) -> Vec<Result<E, UnknownValue>> {
        values
            .iter()
            .map(|&value| Err(UnknownValue { type_name, value }))
            .collect()
    }

    /// Whether a hand written table names every variant in `all` exactly once.
    pub(crate) fn covers<E: Copy + PartialEq>(table: &[(E, i32)], all: &[E]) -> bool {
        table.len() == all.len() && all.iter().all(|v| table.iter().any(|(t, _)| t == v))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_value_displays_the_value_and_the_type_name() {
        // Arrange
        let error = UnknownValue {
            type_name: "Action",
            value: 7,
        };

        // Act
        let text = error.to_string();

        // Assert
        assert_eq!(text, "7 is not a known Action value");
    }

    #[test]
    fn unknown_value_converts_to_bad_argument() {
        // Arrange
        let error = UnknownValue {
            type_name: "MapType",
            value: 9,
        };

        // Act
        let status = Status::from(error);

        // Assert
        assert_eq!(status, Status::BadArgument);
    }

    #[test]
    fn decode_error_converts_to_bad_argument() {
        // Arrange
        let error = DecodeError::TruncatedCount;

        // Act
        let status = Status::from(error);

        // Assert
        assert_eq!(status, Status::BadArgument);
    }

    #[test]
    fn every_failure_converts_to_its_status() {
        // Arrange
        let memory = MemoryError::NegativePointer { ptr: -1 };
        let encode = EncodeError::TooManyPairs { count: 5 };
        let context = InvalidContextId { value: 0 };

        // Act
        let statuses = (
            Status::from(memory),
            Status::from(encode),
            Status::from(NotAllowed),
            Status::from(context),
        );

        // Assert
        assert_eq!(
            statuses,
            (
                Status::InvalidMemoryAccess,
                Status::SerializationFailure,
                Status::BadArgument,
                Status::BadArgument
            )
        );
    }

    #[test]
    fn log_level_orders_from_trace_to_critical() {
        // Arrange
        let levels = LogLevel::ALL;

        // Act
        let sorted = levels.windows(2).all(|pair| pair[0] < pair[1]);

        // Assert
        assert!(sorted);
        assert!(LogLevel::Error >= LogLevel::Warn);
    }
}
