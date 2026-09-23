//! Contexts, their identifiers, and the table that tracks them.
//!
//! The ABI gives every root context and every stream context a `u32`
//! identifier.
//! This crate allocates those identifiers and records, for each live context,
//! its type, its parent, and how far through the done, log, delete sequence
//! it is.

pub(crate) mod table;

use std::fmt;
use std::num::NonZeroU32;

/// The identifier of a root context or a stream context.
///
/// Zero is never a context, because the ABI uses it for the absent parent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContextId(NonZeroU32);

impl ContextId {
    pub(crate) fn new(value: NonZeroU32) -> Self {
        Self(value)
    }

    /// The identifier in the form the ABI uses.
    pub fn get(self) -> u32 {
        self.0.get()
    }

    /// The identifier as a wasm `i32` parameter.
    pub(crate) fn wire(self) -> i32 {
        self.get().cast_signed()
    }
}

impl TryFrom<u32> for ContextId {
    type Error = InvalidContextId;

    fn try_from(value: u32) -> Result<Self, InvalidContextId> {
        NonZeroU32::new(value).map(Self).ok_or(InvalidContextId {
            value: i64::from(value),
        })
    }
}

impl TryFrom<i32> for ContextId {
    type Error = InvalidContextId;

    fn try_from(value: i32) -> Result<Self, InvalidContextId> {
        u32::try_from(value)
            .ok()
            .and_then(NonZeroU32::new)
            .map(Self)
            .ok_or(InvalidContextId {
                value: i64::from(value),
            })
    }
}

impl fmt::Display for ContextId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A value that cannot be a context identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("{value} is not a valid context identifier")]
pub struct InvalidContextId {
    /// The value that was rejected.
    pub value: i64,
}

/// Which kind of context an identifier names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ContextType {
    /// A context with no parent, which the ABI calls the plugin context.
    Root,
    /// A context created under a root context, one per stream.
    Stream,
}

/// Which family of stream a stream context serves.
///
/// A guest decides for itself whether a stream context is a TCP stream or an
/// HTTP stream, and it decides inside its own SDK, so the crate cannot read
/// the choice.
/// Both guest SDKs stop with a panic when a callback of one family names a
/// context of the other family, and a panic poisons the guest.
///
/// The crate records the family of a stream context at its first stream
/// callback and refuses a callback of the other family from then on.
/// A wrong first callback still reaches the guest, so tell the crate which
/// family you serve with
/// [`Guest::expect_stream_kind`](crate::abi::v0_2_1::Guest::expect_stream_kind)
/// before the first callback of a context.
/// The ABI names two families and no more, so this enumeration is closed
/// and you match it with two arms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StreamKind {
    /// A TCP stream, which takes the connection callbacks and the two data
    /// callbacks.
    Tcp,
    /// An HTTP stream, which takes the header, body, and trailer callbacks
    /// of a request and of its response.
    Http,
}

impl fmt::Display for StreamKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Tcp => "a TCP stream",
            Self::Http => "an HTTP stream",
        })
    }
}

/// How far a context is through its finalization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ContextState {
    /// The context is in use.
    Active,
    /// `proxy_on_done` returned false, and the guest will call `proxy_done`.
    Pending,
    /// The guest is done with the context, so `proxy_on_log` and
    /// `proxy_on_delete` may run.
    Done,
}

/// What is wrong with a context argument.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ContextProblem {
    /// No context with the identifier exists in this instance.
    Unknown,
    /// The context is a stream context where a root context is required.
    NotRoot,
    /// The context is a root context where a stream context is required.
    NotStream,
    /// The context is not done, so `proxy_on_log` and `proxy_on_delete`
    /// cannot run.
    NotDone,
    /// The root context still has stream contexts under it.
    HasChildren,
    /// The scope serves another stream context, and its stream state
    /// belongs to that one.
    OtherStream {
        /// The stream context that the scope serves.
        lent: ContextId,
    },
    /// The context serves one family of stream, and the callback belongs to
    /// the other family.
    WrongStreamKind {
        /// The family the context took at its first stream callback, or the
        /// family you declared for it.
        recorded: StreamKind,
        /// The family of the callback you called.
        attempted: StreamKind,
    },
}

impl fmt::Display for ContextProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown => f.write_str("is unknown"),
            Self::NotRoot => f.write_str("is not a root context"),
            Self::NotStream => f.write_str("is not a stream context"),
            Self::NotDone => f.write_str("is not done"),
            Self::HasChildren => f.write_str("still has stream contexts"),
            Self::OtherStream { lent } => {
                write!(f, "is not the stream context {lent} that this scope serves")
            }
            Self::WrongStreamKind {
                recorded,
                attempted,
            } => write!(f, "is {recorded} and took a callback of {attempted}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_and_negative_values_are_not_identifiers() {
        // Arrange
        let values = (0u32, 0i32, -1i32);

        // Act
        let results = (
            ContextId::try_from(values.0),
            ContextId::try_from(values.1),
            ContextId::try_from(values.2),
        );

        // Assert
        assert_eq!(results.0, Err(InvalidContextId { value: 0 }));
        assert_eq!(results.1, Err(InvalidContextId { value: 0 }));
        assert_eq!(results.2, Err(InvalidContextId { value: -1 }));
        assert_eq!(
            results.2.unwrap_err().to_string(),
            "-1 is not a valid context identifier"
        );
    }

    #[test]
    fn one_is_an_identifier_that_displays_itself() {
        // Arrange
        let value = 1u32;

        // Act
        let id = ContextId::try_from(value).unwrap();

        // Assert
        assert_eq!(id.get(), 1);
        assert_eq!(id.wire(), 1);
        assert_eq!(id.to_string(), "1");
    }

    #[test]
    fn problems_display_as_a_predicate() {
        // Arrange
        let problems = [
            ContextProblem::Unknown,
            ContextProblem::NotRoot,
            ContextProblem::NotStream,
            ContextProblem::NotDone,
            ContextProblem::HasChildren,
            ContextProblem::WrongStreamKind {
                recorded: StreamKind::Tcp,
                attempted: StreamKind::Http,
            },
        ];

        // Act
        let texts: Vec<String> = problems.iter().map(ToString::to_string).collect();

        // Assert
        assert_eq!(
            texts,
            [
                "is unknown",
                "is not a root context",
                "is not a stream context",
                "is not done",
                "still has stream contexts",
                "is a TCP stream and took a callback of an HTTP stream",
            ]
        );
    }
}
