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

    /// The identifier as the ABI carries it.
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
}

impl fmt::Display for ContextProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Unknown => "is unknown",
            Self::NotRoot => "is not a root context",
            Self::NotStream => "is not a stream context",
            Self::NotDone => "is not done",
            Self::HasChildren => "still has stream contexts",
        })
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
                "still has stream contexts"
            ]
        );
    }
}
