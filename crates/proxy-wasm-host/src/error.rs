//! The errors that you observe.
//!
//! A guest observes only [`Status`] values, which never unwind.
//! Everything that unwinds a guest call, fails to build an instance, or
//! rejects a guest address is one of the types here.
//! The refusals that an ABI version defines are on that version's module,
//! for example [`GuestError`].
//!
//! [`Status`]: crate::abi::v0_2_1::types::Status
//! [`GuestError`]: crate::abi::v0_2_1::GuestError

use std::fmt;

/// Why the runtime could not do what the embedder asked.
///
/// The variant tells you whether the guest is still usable.
/// After [`Error::Trap`], [`Error::LimitExceeded`], [`Error::GuestExit`],
/// or any error that unwound a guest call, the instance is poisoned and every
/// later call returns [`Error::Poisoned`].
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The bytes are not a valid module for this engine.
    #[error("the module did not compile")]
    Compile {
        /// The compiler's own error.
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    /// The module could not be instantiated, for example because it imports
    /// a function the crate does not provide.
    #[error("the module did not instantiate")]
    Instantiate {
        /// The runtime's own error.
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    /// The engine or the limits are configured in a way the runtime rejects.
    #[error("invalid configuration: {message}")]
    Config {
        /// What is wrong.
        message: String,
    },
    /// The module exports no memory named `memory`.
    #[error("the module exports no memory named \"memory\"")]
    MissingMemory,
    /// The module exports neither `proxy_on_memory_allocate` nor `malloc`.
    #[error("the module exports neither proxy_on_memory_allocate nor malloc")]
    MissingAllocator,
    /// A named export is absent.
    #[error("the module does not export {name}")]
    MissingExport {
        /// The export name.
        name: String,
    },
    /// A named export exists with another type than the caller expected.
    #[error("the export {name} does not have the expected type")]
    ExportTypeMismatch {
        /// The export name.
        name: String,
    },
    /// The guest trapped.
    #[error("the guest trapped: {message}")]
    Trap {
        /// The trap reason.
        message: String,
        /// The guest stack at the trap, when the runtime has one.
        backtrace: Option<String>,
    },
    /// The guest ran past one of its limits.
    #[error("the guest exceeded its {limit} limit")]
    LimitExceeded {
        /// Which limit.
        limit: Limit,
    },
    /// The guest called `proc_exit`.
    #[error("the guest called proc_exit with code {code}")]
    GuestExit {
        /// The exit code the guest passed.
        code: i32,
    },
    /// An earlier failure unwound a guest call, so the guest heap is not
    /// trusted.
    #[error("the instance is poisoned by an earlier failure")]
    Poisoned,
    /// A guest address or length was rejected.
    #[error(transparent)]
    Memory(#[from] MemoryError),
    /// The guest allocator returned null.
    #[error("the guest allocator returned null for {size} bytes")]
    AllocationFailed {
        /// The requested size.
        size: u32,
    },
    /// A host value is larger than the ABI can carry to the guest.
    #[error("{size} bytes is more than the ABI can pass to the guest")]
    ValueTooLarge {
        /// The value size.
        size: usize,
    },
}

/// A resource limit that a guest can exceed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Limit {
    /// The CPU time limit, enforced through epoch interruption.
    Epoch,
    /// The fuel budget.
    Fuel,
    /// The wasm stack.
    Stack,
}

impl fmt::Display for Limit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Epoch => f.write_str("epoch"),
            Self::Fuel => f.write_str("fuel"),
            Self::Stack => f.write_str("stack"),
        }
    }
}

/// Why a guest address or range was rejected.
///
/// A host function maps every variant to [`Status::InvalidMemoryAccess`].
///
/// [`Status::InvalidMemoryAccess`]: crate::abi::v0_2_1::types::Status::InvalidMemoryAccess
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum MemoryError {
    /// The guest passed a negative address.
    #[error("guest address {ptr} is negative")]
    NegativePointer {
        /// The value the guest passed.
        ptr: i32,
    },
    /// The guest passed a negative length.
    #[error("guest length {len} is negative")]
    NegativeLength {
        /// The value the guest passed.
        len: i32,
    },
    /// The address plus the length does not fit in 32 bits.
    #[error("guest range {ptr}+{len} does not fit in 32 bits")]
    RangeOverflow {
        /// The address.
        ptr: u32,
        /// The length.
        len: u32,
    },
    /// The range ends past the end of guest memory.
    #[error("guest range {ptr}+{len} is outside the {memory_size} byte memory")]
    OutOfBounds {
        /// The address.
        ptr: u32,
        /// The length.
        len: u32,
        /// The memory size in bytes.
        memory_size: usize,
    },
    /// A write was given a byte slice of another length than the range.
    #[error("expected {expected} bytes for the guest range, got {actual}")]
    LengthMismatch {
        /// The range length.
        expected: u32,
        /// The byte slice length.
        actual: usize,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limit_displays_its_name() {
        // Arrange
        let limits = [Limit::Epoch, Limit::Fuel, Limit::Stack];

        // Act
        let texts: Vec<String> = limits.iter().map(ToString::to_string).collect();

        // Assert
        assert_eq!(texts, vec!["epoch", "fuel", "stack"]);
    }

    #[test]
    fn errors_display_their_messages() {
        // Arrange
        let errors: [&dyn fmt::Display; 3] = [
            &Error::Trap {
                message: "unreachable".into(),
                backtrace: None,
            },
            &Error::GuestExit { code: 3 },
            &MemoryError::OutOfBounds {
                ptr: 65_530,
                len: 8,
                memory_size: 65_536,
            },
        ];

        // Act
        let texts: Vec<String> = errors.iter().map(ToString::to_string).collect();

        // Assert
        assert_eq!(
            texts,
            vec![
                "the guest trapped: unreachable",
                "the guest called proc_exit with code 3",
                "guest range 65530+8 is outside the 65536 byte memory",
            ]
        );
    }

    #[test]
    fn memory_error_converts_into_error() {
        // Arrange
        let memory_error = MemoryError::NegativePointer { ptr: -1 };

        // Act
        let error = Error::from(memory_error);

        // Assert
        assert!(matches!(
            error,
            Error::Memory(MemoryError::NegativePointer { ptr: -1 })
        ));
    }
}
