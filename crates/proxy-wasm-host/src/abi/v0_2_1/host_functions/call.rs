//! The steps every host function that reaches the embedder shares.
//!
//! A host function resolves the effective context, refuses one whose root the
//! guest rejected, and reaches the stream state the scope installed or the
//! shared services the embedder supplied.
//! When any of those three fails, the guest receives the status its own ABI
//! section lists for a resource that is not available, which the caller
//! passes as `absent`.

use std::sync::Arc;

use crate::abi::v0_2_1::AbiAccess;
use crate::abi::v0_2_1::SharedServices;
use crate::abi::v0_2_1::host_functions::Failure;
use crate::abi::v0_2_1::payload::Delivery;
use crate::abi::v0_2_1::types::Status;
use crate::abi::v0_2_1::{ContextId, Invocation, StreamState};
use crate::codec::pairs::{DecodeError, Pairs, decode_pairs};
use crate::runtime::HostState;

/// The call an embedder is told about, from a context, whatever callback is
/// running, and the callout that callback delivers.
pub(super) fn invocation(state: &HostState, context: ContextId) -> Invocation {
    let abi = state.abi();
    let mut call = Invocation::new(abi.guest(), context);
    if let Some(callback) = abi.current_callback() {
        call = call.with_callback(callback);
    }
    if let Some(callout) = abi.delivery().and_then(Delivery::callout) {
        call = call.with_callout(callout);
    }
    call
}

/// Decodes a map the guest wrote, under the limits the embedder set.
///
/// Every host function that reads a map from guest memory goes through this,
/// so a new one cannot decode without a bound.
pub(super) fn guest_pairs<'a>(
    state: &HostState,
    bytes: &'a [u8],
) -> Result<Pairs<'a>, DecodeError> {
    decode_pairs(bytes, state.pair_limits())
}

/// The effective context, refused when the guest rejected its root or the
/// whole instance.
pub(super) fn context(state: &HostState, absent: Status) -> Result<ContextId, Failure> {
    let contexts = state.abi().contexts();
    match contexts.effective() {
        Some(context) if contexts.rejection_of(context).is_none() => Ok(context),
        _ => Err(absent.into()),
    }
}

/// The call to report and the stream state to ask.
pub(super) fn with_stream(
    state: &mut HostState,
    absent: Status,
) -> Result<(Invocation, &mut dyn StreamState), Failure> {
    let context = context(state, absent)?;
    let call = invocation(state, context);
    let stream = state.abi_mut().stream_state().ok_or(absent)?;
    Ok((call, stream))
}

/// Drops the grants when the embedder has replaced the shared services.
///
/// A body that consults a grant calls this first, because a queue or metric
/// identifier means one thing inside one store and something else inside
/// another.
pub(super) fn settle(state: &mut HostState) {
    state.abi_mut().settle();
}

/// The call to report and the shared services to ask.
///
/// The grants are settled first, so a guest never passes an identifier that a
/// store the embedder has since replaced handed out.
pub(super) fn with_shared(
    state: &mut HostState,
    absent: Status,
) -> Result<(Invocation, Arc<dyn SharedServices>), Failure> {
    let shared = Arc::clone(state.abi().services().shared());
    state.abi_mut().settle_grants(&shared);
    let context = context(state, absent)?;
    let call = invocation(state, context);
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
    use crate::abi::v0_2_1::payload::Delivery;
    use crate::abi::v0_2_1::test_support::{
        MINIMAL_GUEST, RecordingStream, bare, engine, hosted, unhosted,
    };
    use crate::abi::v0_2_1::types::MapType;
    use crate::abi::v0_2_1::{Callback, CalloutId, HttpCallResponse};

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
        let (call, _) = found.expect("a stream state is installed");
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

    #[test]
    fn a_call_in_a_delivery_names_the_callout() {
        // Arrange
        let engine = engine();
        let (mut instance, _) = hosted(&engine, MINIMAL_GUEST, RecordingStream::new());
        let callout = CalloutId::try_from(7_u32).unwrap();
        let delivery = Delivery::http_call_response(callout, HttpCallResponse::failed());
        instance.state_mut().abi_mut().set_delivery(Some(delivery));

        // Act
        let call = with_stream(instance.state_mut(), Status::NotFound).map(|(call, _)| call);

        // Assert
        assert_eq!(call.ok().and_then(|call| call.callout), Some(callout));
    }
}
