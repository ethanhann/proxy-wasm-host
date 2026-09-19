//! How a host function body ends.
//!
//! A body either answers the guest with a status or ends the guest call.
//! Every error a body can meet converts into one of the two, so a body
//! reads as a sequence of `?` operators.

use crate::abi::v0_2_1::InvalidContextId;
use crate::abi::v0_2_1::types::{Status, UnknownValue};
use crate::codec::pairs::{DecodeError, EncodeError};
use crate::error::MemoryError;
use crate::{Error, NotAllowed};

/// Why a host function body did not answer `OK`.
#[derive(Debug)]
pub(crate) enum Failure {
    /// The guest receives this status.
    Status(Status),
    /// The guest call unwinds with this error, and the instance is poisoned.
    Unwind(Error),
}

impl From<Status> for Failure {
    fn from(status: Status) -> Self {
        Self::Status(status)
    }
}

impl From<MemoryError> for Failure {
    fn from(error: MemoryError) -> Self {
        Self::Status(Status::from(error))
    }
}

impl From<UnknownValue> for Failure {
    fn from(error: UnknownValue) -> Self {
        Self::Status(Status::from(error))
    }
}

impl From<InvalidContextId> for Failure {
    fn from(error: InvalidContextId) -> Self {
        Self::Status(Status::from(error))
    }
}

impl From<DecodeError> for Failure {
    fn from(error: DecodeError) -> Self {
        Self::Status(Status::from(error))
    }
}

impl From<EncodeError> for Failure {
    fn from(error: EncodeError) -> Self {
        Self::Status(Status::from(error))
    }
}

impl From<NotAllowed> for Failure {
    fn from(error: NotAllowed) -> Self {
        Self::Status(Status::from(error))
    }
}

impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        match error {
            Error::Memory(memory) => Self::Status(Status::from(memory)),
            Error::ValueTooLarge { .. } | Error::AllocationFailed { .. } => {
                Self::Status(Status::InternalFailure)
            }
            Error::Trap { .. }
            | Error::LimitExceeded { .. }
            | Error::GuestExit { .. }
            | Error::Poisoned
            | Error::MissingAllocator
            | Error::MissingMemory
            | Error::Compile { .. }
            | Error::Instantiate { .. }
            | Error::Config { .. }
            | Error::MissingExport { .. }
            | Error::ExportTypeMismatch { .. } => Self::Unwind(error),
        }
    }
}

/// Answers a host function this crate does not serve yet.
///
/// A guest built with the Rust SDK aborts on this status, so the log line is
/// the embedder's warning.
pub(crate) fn stub(name: &'static str) -> i32 {
    tracing::debug!(function = name, "unimplemented host function called");
    i32::from(Status::Unimplemented)
}

/// Turns a body's result into what wasmtime returns to the guest.
pub(crate) fn complete(
    name: &'static str,
    result: Result<(), Failure>,
) -> Result<i32, wasmtime::Error> {
    match result {
        Ok(()) => Ok(i32::from(Status::Ok)),
        Err(Failure::Status(status)) => {
            tracing::debug!(function = name, ?status, "host function reported a status");
            Ok(i32::from(status))
        }
        Err(Failure::Unwind(error)) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::map_guest_error;

    /// What the conversion answers, without the payload.
    fn answer(error: Error) -> Option<Status> {
        match Failure::from(error) {
            Failure::Status(status) => Some(status),
            Failure::Unwind(_) => None,
        }
    }

    #[test]
    fn every_runtime_error_maps_to_a_status_or_an_unwind() {
        // Arrange
        let source = || -> Box<dyn std::error::Error + Send + Sync> { "refused".into() };
        let errors = [
            Error::Memory(MemoryError::NegativePointer { ptr: -1 }),
            Error::ValueTooLarge { size: 1 },
            Error::AllocationFailed { size: 1 },
            Error::Compile { source: source() },
            Error::Instantiate { source: source() },
            Error::Config {
                message: String::new(),
            },
            Error::MissingMemory,
            Error::MissingAllocator,
            Error::MissingExport {
                name: String::new(),
            },
            Error::ExportTypeMismatch {
                name: String::new(),
            },
            Error::Trap {
                message: String::new(),
                backtrace: None,
            },
            Error::LimitExceeded {
                limit: crate::Limit::Fuel,
            },
            Error::GuestExit { code: 0 },
            Error::Poisoned,
        ];

        // Act
        let answers: Vec<Option<Status>> = errors.into_iter().map(answer).collect();

        // Assert
        assert_eq!(
            answers[..3],
            [
                Some(Status::InvalidMemoryAccess),
                Some(Status::InternalFailure),
                Some(Status::InternalFailure)
            ]
        );
        assert_eq!(answers[3..], [None; 11]);
    }

    #[test]
    fn complete_maps_ok_a_status_and_an_unwind() {
        // Arrange
        let results = [
            Ok(()),
            Err(Failure::Status(Status::NotFound)),
            Err(Failure::Unwind(Error::Poisoned)),
        ];

        // Act
        let completed: Vec<Result<i32, wasmtime::Error>> = results
            .into_iter()
            .map(|result| complete("proxy_test", result))
            .collect();

        // Assert
        assert!(matches!(completed[0], Ok(0)));
        assert!(matches!(completed[1], Ok(1)));
        let unwound = completed.into_iter().nth(2).unwrap().unwrap_err();
        assert!(matches!(map_guest_error(unwound), Error::Poisoned));
    }

    #[test]
    fn a_stub_answers_unimplemented() {
        // Arrange
        let name = "proxy_get_log_level";

        // Act
        let answer = stub(name);

        // Assert
        assert_eq!(answer, i32::from(Status::Unimplemented));
    }

    #[test]
    fn errors_split_into_statuses_and_unwinds() {
        // Arrange
        let errors = [
            Error::Memory(MemoryError::NegativePointer { ptr: -1 }),
            Error::ValueTooLarge { size: 5 },
            Error::AllocationFailed { size: 5 },
            Error::Poisoned,
        ];

        // Act
        let failures: Vec<Failure> = errors.into_iter().map(Failure::from).collect();

        // Assert
        assert!(matches!(
            failures[0],
            Failure::Status(Status::InvalidMemoryAccess)
        ));
        assert!(matches!(
            failures[1],
            Failure::Status(Status::InternalFailure)
        ));
        assert!(matches!(
            failures[2],
            Failure::Status(Status::InternalFailure)
        ));
        assert!(matches!(failures[3], Failure::Unwind(Error::Poisoned)));
    }
}
