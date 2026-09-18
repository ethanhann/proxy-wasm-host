//! The steps every host function that reaches the embedder shares.
//!
//! A host function resolves the effective context, refuses one whose root the
//! guest rejected, and reaches the stream host the scope installed or the
//! shared services the embedder supplied.
//! When any of those three fails, the guest receives the status its own ABI
//! section lists for a resource that is not available, which the caller
//! passes as `absent`.

use std::sync::Arc;

use crate::abi::v0_2_1::SharedServices;
use crate::abi::v0_2_1::host_functions::Failure;
use crate::abi::v0_2_1::types::Status;
use crate::abi::v0_2_1::{ContextId, HostCall, StreamHost};
use crate::runtime::HostState;

/// The effective context, refused when the guest rejected its root or the
/// whole instance.
pub(super) fn context(state: &HostState, absent: Status) -> Result<ContextId, Failure> {
    let contexts = state.abi().contexts();
    match contexts.effective() {
        Some(context) if contexts.rejection_of(context).is_none() => Ok(context),
        _ => Err(absent.into()),
    }
}

/// The call to report and the stream host to ask.
pub(super) fn with_stream(
    state: &mut HostState,
    absent: Status,
) -> Result<(HostCall, &mut dyn StreamHost), Failure> {
    let context = context(state, absent)?;
    let call = HostCall::new(context, state.abi().current_callback());
    let stream = state.abi_mut().stream_host().ok_or(absent)?;
    Ok((call, stream))
}

/// Drops the grants when the embedder has replaced the shared services.
///
/// A body that consults a grant calls this first, because a queue or metric
/// identifier means one thing inside one store and something else inside
/// another.
pub(super) fn settle(state: &mut HostState) {
    let shared = std::sync::Arc::clone(state.services().shared());
    state.abi_mut().settle_grants(&shared);
}

/// The call to report and the shared services to ask.
///
/// The grants are settled first, so a guest never names an identifier that a
/// store the embedder has since replaced handed out.
pub(super) fn with_shared(
    state: &mut HostState,
    absent: Status,
) -> Result<(HostCall, Arc<dyn SharedServices>), Failure> {
    let shared = Arc::clone(state.services().shared());
    state.abi_mut().settle_grants(&shared);
    let context = context(state, absent)?;
    let call = HostCall::new(context, state.abi().current_callback());
    Ok((call, shared))
}

/// The embedder's answer, with a success for a resource it never touched
/// turned into a failure.
pub(super) fn from_embedder<T>(
    method: &'static str,
    result: Result<T, Status>,
) -> Result<T, Failure> {
    match result {
        Ok(value) => Ok(value),
        Err(Status::Ok) => {
            tracing::warn!(method, "the embedder refused with Status::Ok");
            Err(Status::InternalFailure.into())
        }
        Err(status) => Err(Failure::Status(status)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::v0_2_1::Callback;
    use crate::abi::v0_2_1::test_support::{RecordingStream, bare, hosted, unhosted};
    use crate::abi::v0_2_1::types::MapType;
    use crate::runtime::test_support::{MINIMAL_GUEST, engine};

    #[test]
    fn the_effective_context_is_reported_when_one_is_set() {
        // Arrange
        let engine = engine();
        let (instance, root) = unhosted(&engine, MINIMAL_GUEST);

        // Act
        let found = context(instance.state(), Status::BadArgument);

        // Assert
        assert_eq!(found.ok(), Some(root));
    }

    #[test]
    fn no_effective_context_reports_the_status_the_caller_gave() {
        // Arrange
        let engine = engine();
        let instance = bare(&engine, MINIMAL_GUEST);

        // Act
        let found = context(instance.state(), Status::NotFound);

        // Assert
        assert!(matches!(found, Err(Failure::Status(Status::NotFound))));
    }

    #[test]
    fn a_refused_root_reports_the_status_the_caller_gave() {
        // Arrange
        let engine = engine();
        let (mut instance, root) = unhosted(&engine, MINIMAL_GUEST);
        instance.state_mut().abi_mut().contexts_mut().reject(root);

        // Act
        let found = context(instance.state(), Status::Unimplemented);

        // Assert
        assert!(matches!(found, Err(Failure::Status(Status::Unimplemented))));
    }

    #[test]
    fn a_refused_instance_reports_the_status_the_caller_gave() {
        // Arrange
        let engine = engine();
        let (mut instance, root) = unhosted(&engine, MINIMAL_GUEST);
        instance
            .state_mut()
            .abi_mut()
            .contexts_mut()
            .reject_vm(root);

        // Act
        let found = context(instance.state(), Status::NotFound);

        // Assert
        assert!(matches!(found, Err(Failure::Status(Status::NotFound))));
    }

    #[test]
    fn with_stream_reports_the_call_the_embedder_sees() {
        // Arrange
        let engine = engine();
        let (mut instance, root) = hosted(&engine, MINIMAL_GUEST, RecordingStream::new());

        // Act
        let found = with_stream(instance.state_mut(), Status::NotFound);

        // Assert
        let (call, _) = found.expect("a stream host is installed");
        assert_eq!(call.context, root);
        assert_eq!(call.callback, Some(Callback::RequestHeaders));
    }

    #[test]
    fn with_stream_reports_the_status_the_caller_gave_when_none_is_installed() {
        // Arrange
        let engine = engine();
        let (mut instance, _) = unhosted(&engine, MINIMAL_GUEST);

        // Act
        let found = with_stream(instance.state_mut(), Status::Unimplemented);

        // Assert
        assert!(matches!(
            found.err(),
            Some(Failure::Status(Status::Unimplemented))
        ));
    }

    #[test]
    fn an_answer_passes_through_and_a_success_for_nothing_is_a_failure() {
        // Arrange
        let refusals = [
            Ok(MapType::HttpRequestHeaders),
            Err(Status::Ok),
            Err(Status::NotFound),
        ];

        // Act
        let results = refusals.map(|result| from_embedder("header_map", result));

        // Assert
        assert!(matches!(results[0], Ok(MapType::HttpRequestHeaders)));
        assert!(matches!(
            results[1],
            Err(Failure::Status(Status::InternalFailure))
        ));
        assert!(matches!(results[2], Err(Failure::Status(Status::NotFound))));
    }

    #[test]
    fn with_shared_reports_the_call_the_embedder_sees() {
        // Arrange
        let engine = engine();
        let (mut instance, root) = unhosted(&engine, MINIMAL_GUEST);

        // Act
        let found = with_shared(instance.state_mut(), Status::NotFound);

        // Assert
        let (call, _) = found.expect("the services are always installed");
        assert_eq!(call.context, root);
        assert_eq!(call.callback, Some(Callback::RequestHeaders));
    }

    #[test]
    fn with_shared_reports_the_status_the_caller_gave_with_no_context() {
        // Arrange
        let engine = engine();
        let mut instance = bare(&engine, MINIMAL_GUEST);

        // Act
        let found = with_shared(instance.state_mut(), Status::NotFound);

        // Assert
        assert!(matches!(
            found.err(),
            Some(Failure::Status(Status::NotFound))
        ));
    }
}
