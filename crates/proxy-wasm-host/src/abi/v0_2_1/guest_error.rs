//! Why a guest could not be built or a callback could not run.

use crate::Error;
use crate::abi::UnsupportedAbi;
use crate::abi::v0_2_1::{Callback, ContextId, ContextProblem};

/// Why [`Guest::new`](crate::abi::v0_2_1::Guest::new) or a callback of a
/// [`CallScope`](crate::abi::v0_2_1::CallScope) failed.
///
/// The first five variants are refusals that this ABI version defines, and
/// the guest is still usable after each of them.
/// [`GuestError::Runtime`] holds every failure of the runtime.
/// Some of those poison the guest and some do not, so after any error ask
/// [`Guest::is_poisoned`](crate::abi::v0_2_1::Guest::is_poisoned) before you
/// use the guest again.
///
/// For example, you can tell a rejected configuration from a trap:
///
/// ```
/// use proxy_wasm_host::Error;
/// use proxy_wasm_host::abi::v0_2_1::{Callback, GuestError};
///
/// fn describe(error: &GuestError) -> &'static str {
///     match error {
///         GuestError::GuestRejected { callback: Callback::Configure, .. } => "bad configuration",
///         GuestError::Runtime(Error::Trap { .. }) => "the guest crashed",
///         _ => "another failure",
///     }
/// }
/// # let error = GuestError::from(Error::Poisoned);
/// # assert_eq!(describe(&error), "another failure");
/// ```
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum GuestError {
    /// The module advertises no ABI version that the crate accepts.
    #[error(transparent)]
    UnsupportedAbi(#[from] UnsupportedAbi),
    /// An earlier `proxy_on_vm_start` or `proxy_on_configure` returned
    /// false, so this callback and every later one on the same root is
    /// refused.
    #[error("the guest rejected root context {root} in {callback}")]
    GuestRejected {
        /// The callback that returned false.
        callback: Callback,
        /// The root context that callback served.
        root: ContextId,
    },
    /// A context argument does not satisfy the callback's precondition.
    #[error("context {id} {problem}")]
    Context {
        /// The context you passed.
        id: ContextId,
        /// What is wrong with it.
        problem: ContextProblem,
    },
    /// The guest has allocated every context identifier.
    #[error("no context identifier is left")]
    ContextIdsExhausted,
    /// A callback returned a value outside its enumeration.
    #[error("{callback} returned {value}, which is not a valid result")]
    UnexpectedReturn {
        /// The callback that returned the value.
        callback: Callback,
        /// The value it returned.
        value: i32,
    },
    /// The runtime failed.
    #[error(transparent)]
    Runtime(#[from] Error),
}

#[cfg(test)]
mod tests {
    use std::error::Error as _;

    use super::*;

    #[test]
    fn the_context_errors_display_their_subject() {
        // Arrange
        let id = ContextId::try_from(4).unwrap();
        let errors = [
            GuestError::GuestRejected {
                callback: Callback::Configure,
                root: id,
            },
            GuestError::Context {
                id,
                problem: ContextProblem::NotDone,
            },
            GuestError::ContextIdsExhausted,
            GuestError::UnexpectedReturn {
                callback: Callback::Done,
                value: 7,
            },
        ];

        // Act
        let texts: Vec<String> = errors.iter().map(ToString::to_string).collect();

        // Assert
        assert_eq!(
            texts,
            [
                "the guest rejected root context 4 in proxy_on_configure",
                "context 4 is not done",
                "no context identifier is left",
                "proxy_on_done returned 7, which is not a valid result",
            ]
        );
    }

    #[test]
    fn an_unsupported_abi_prints_the_names_it_found() {
        // Arrange
        let unsupported = UnsupportedAbi {
            found: vec!["proxy_abi_version_0_1_0".to_owned()],
        };

        // Act
        let error = GuestError::from(unsupported);

        // Assert
        assert_eq!(
            error.to_string(),
            "no supported proxy_abi_version export, found [\"proxy_abi_version_0_1_0\"]"
        );
        assert!(matches!(error, GuestError::UnsupportedAbi(_)));
        assert!(
            error.source().is_none(),
            "a transparent variant has the source of the value inside, which has none"
        );
    }

    #[test]
    fn a_runtime_error_converts_and_keeps_its_text_and_its_source() {
        // Arrange
        let inner = Error::Compile {
            source: "the bytes are not a module".into(),
        };
        let expected_text = inner.to_string();
        let expected_source = inner.source().map(ToString::to_string);

        // Act
        let error = GuestError::from(inner);

        // Assert
        assert!(matches!(error, GuestError::Runtime(Error::Compile { .. })));
        assert_eq!(error.to_string(), expected_text);
        assert_eq!(
            expected_source.as_deref(),
            Some("the bytes are not a module")
        );
        assert_eq!(error.source().map(ToString::to_string), expected_source);
    }
}
