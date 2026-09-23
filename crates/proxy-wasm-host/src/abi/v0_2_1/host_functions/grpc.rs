//! The five gRPC callout functions.
//!
//! A guest opens a gRPC call or a gRPC stream, and the `Callouts` service of
//! the embedder sends it.
//! The crate keeps the record of what is open, and the embedder answers
//! through the four gRPC callbacks.
//!
//! The three functions that take an open callout answer `OK` for a callout
//! that this guest opened and that has ended.
//! A guest of the Rust SDK stops with a panic on any other status from them,
//! and a plugin that closes its own stream inside `proxy_on_grpc_close` is
//! ordinary code.
//! They answer `NOT_FOUND` for an identifier that the crate never gave out
//! and for a callout of another context, which are a defect of the guest.
//! They never answer `BAD_ARGUMENT`, which the ABI text lists and the SDK
//! does not accept.
//! The ABI text gives `NOT_FOUND` for a callout that ended, and this crate
//! departs from it for the reason above.

use std::borrow::Cow;
use std::sync::Arc;
use std::time::Duration;

use wasmtime::AsContextMut;

use crate::abi::v0_2_1::AbiAccess;
use crate::abi::v0_2_1::callout::Callout;
use crate::abi::v0_2_1::host_functions::Failure;
use crate::abi::v0_2_1::host_functions::call::{context, guest_pairs, invocation};
use crate::abi::v0_2_1::host_functions::callout::open_callout;
use crate::abi::v0_2_1::types::Status;
use crate::abi::v0_2_1::{CalloutId, CalloutKind, GrpcCall, GrpcStream, HeaderPairs};
use crate::runtime::{GuestPtr, GuestSlice, HostState, split};

/// The metadata of a gRPC callout, which answers `PARSE_FAILURE` when it does
/// not decode.
///
/// The `From<DecodeError>` of this crate gives `BAD_ARGUMENT`, which the two
/// opening functions do not accept, so this conversion is made here.
/// A map above a limit takes the same path, because the ABI lists no
/// `BAD_ARGUMENT` for either function that opens a gRPC callout.
fn metadata<'a>(state: &HostState, bytes: &'a [u8]) -> Result<HeaderPairs<'a>, Failure> {
    let pairs = guest_pairs(state, bytes).map_err(|_| Status::ParseFailure)?;
    Ok(pairs
        .into_iter()
        .map(|(key, value)| (Cow::Borrowed(key), Cow::Borrowed(value)))
        .collect())
}

/// Sends one gRPC message and waits for one answer.
#[expect(
    clippy::too_many_arguments,
    reason = "the ABI fixes the parameter list of this host function"
)]
pub(super) fn proxy_grpc_call(
    ctx: &mut impl AsContextMut<Data = HostState>,
    upstream_name_data: i32,
    upstream_name_size: i32,
    service_name_data: i32,
    service_name_size: i32,
    method_name_data: i32,
    method_name_size: i32,
    serialized_initial_metadata_data: i32,
    serialized_initial_metadata_size: i32,
    message_data: i32,
    message_size: i32,
    timeout: i32,
    return_call_id: i32,
) -> Result<(), Failure> {
    let upstream = GuestSlice::try_from((upstream_name_data, upstream_name_size))?;
    let service = GuestSlice::try_from((service_name_data, service_name_size))?;
    let method = GuestSlice::try_from((method_name_data, method_name_size))?;
    let initial = GuestSlice::try_from((
        serialized_initial_metadata_data,
        serialized_initial_metadata_size,
    ))?;
    let message = GuestSlice::try_from((message_data, message_size))?;
    let id_ptr = GuestPtr::try_from(return_call_id)?;
    let timeout = Duration::from_millis(u64::from(timeout.cast_unsigned()));
    let (mut memory, state) = split(ctx)?;
    memory.read_u32(id_ptr)?;
    let request = GrpcCall::new(
        Cow::Borrowed(memory.read(upstream)?),
        Cow::Borrowed(memory.read(service)?),
        Cow::Borrowed(memory.read(method)?),
    )
    .with_initial_metadata(metadata(state, memory.read(initial)?)?)
    .with_message(Cow::Borrowed(memory.read(message)?))
    .with_timeout(timeout);
    let callout = open_callout(state, CalloutKind::GrpcCall, |service, call, id| {
        service.grpc_call(call, id, request).map_err(Status::from)
    })?;
    memory.write_u32(id_ptr, callout.get())?;
    Ok(())
}

/// Opens a gRPC stream that stays open for many messages.
#[expect(
    clippy::too_many_arguments,
    reason = "the ABI fixes the parameter list of this host function"
)]
pub(super) fn proxy_grpc_stream(
    ctx: &mut impl AsContextMut<Data = HostState>,
    upstream_name_data: i32,
    upstream_name_size: i32,
    service_name_data: i32,
    service_name_size: i32,
    method_name_data: i32,
    method_name_size: i32,
    serialized_initial_metadata_data: i32,
    serialized_initial_metadata_size: i32,
    return_stream_id: i32,
) -> Result<(), Failure> {
    let upstream = GuestSlice::try_from((upstream_name_data, upstream_name_size))?;
    let service = GuestSlice::try_from((service_name_data, service_name_size))?;
    let method = GuestSlice::try_from((method_name_data, method_name_size))?;
    let initial = GuestSlice::try_from((
        serialized_initial_metadata_data,
        serialized_initial_metadata_size,
    ))?;
    let id_ptr = GuestPtr::try_from(return_stream_id)?;
    let (mut memory, state) = split(ctx)?;
    memory.read_u32(id_ptr)?;
    let request = GrpcStream::new(
        Cow::Borrowed(memory.read(upstream)?),
        Cow::Borrowed(memory.read(service)?),
        Cow::Borrowed(memory.read(method)?),
    )
    .with_initial_metadata(metadata(state, memory.read(initial)?)?);
    let callout = open_callout(state, CalloutKind::GrpcStream, |service, call, id| {
        service.grpc_stream(call, id, request).map_err(Status::from)
    })?;
    memory.write_u32(id_ptr, callout.get())?;
    Ok(())
}

/// What an open callout of this guest is, for the three functions that take
/// one.
enum Found {
    /// The callout is open, with its identifier and its entry.
    Open(CalloutId, Callout),
    /// The guest opened this callout and it has ended, so the function
    /// answers `OK` and does nothing.
    Ended,
}

/// Finds the callout of an identifier the guest gave.
///
/// The effective context must be the context that opened the callout, so one
/// request cannot feed or end the callout of another request, and the
/// context the service hears is the context every delivery passes.
///
/// An identifier that this guest opened and that has ended answers `Ended`
/// before the context is read, because the entry that recorded its context is
/// gone.
/// A guest can therefore learn that a number was given out, which the
/// rustdoc of the three functions states.
fn find(state: &HostState, id: i32) -> Result<Found, Failure> {
    let callout = CalloutId::try_from(id).map_err(|_| Status::NotFound)?;
    let abi = state.abi();
    let Some(entry) = abi.callouts().get(callout) else {
        if abi.callouts().issued(callout) {
            return Ok(Found::Ended);
        }
        return Err(Status::NotFound.into());
    };
    let caller = context(state, Status::NotFound)?;
    if caller != entry.caller {
        return Err(Status::NotFound.into());
    }
    Ok(Found::Open(callout, entry))
}

/// Sends one message on a gRPC stream the guest opened.
pub(super) fn proxy_grpc_send(
    ctx: &mut impl AsContextMut<Data = HostState>,
    stream_id: i32,
    message_data: i32,
    message_size: i32,
    end_stream: i32,
) -> Result<(), Failure> {
    let message = GuestSlice::try_from((message_data, message_size))?;
    let (memory, state) = split(ctx)?;
    let bytes = memory.read(message)?;
    let end_of_stream = end_stream != 0;
    let (callout, entry) = match find(state, stream_id)? {
        Found::Ended => return Ok(()),
        Found::Open(callout, entry) => (callout, entry),
    };
    if entry.kind != CalloutKind::GrpcStream {
        return Err(Status::NotFound.into());
    }
    if entry.closed_by_guest {
        tracing::debug!(%callout, "the guest sends on a stream it closed");
        return Ok(());
    }
    let caller = context(state, Status::NotFound)?;
    let call = invocation(state, caller);
    let service = Arc::clone(state.abi().services().callouts());
    service.grpc_send(call, callout, bytes, end_of_stream);
    if end_of_stream {
        state.abi_mut().callouts_mut().close_by_guest(callout);
    }
    Ok(())
}

/// Ends a gRPC call or stream that the guest gives up.
pub(super) fn proxy_grpc_cancel(
    ctx: &mut impl AsContextMut<Data = HostState>,
    call_or_stream_id: i32,
) -> Result<(), Failure> {
    let (_, state) = split(ctx)?;
    let (callout, entry) = match find(state, call_or_stream_id)? {
        Found::Ended => return Ok(()),
        Found::Open(callout, entry) => (callout, entry),
    };
    if entry.kind == CalloutKind::HttpCall {
        return Err(Status::NotFound.into());
    }
    end_callout(state, callout);
    Ok(())
}

/// Closes a gRPC call, or the guest's side of a gRPC stream.
pub(super) fn proxy_grpc_close(
    ctx: &mut impl AsContextMut<Data = HostState>,
    call_or_stream_id: i32,
) -> Result<(), Failure> {
    let (_, state) = split(ctx)?;
    let (callout, entry) = match find(state, call_or_stream_id)? {
        Found::Ended => return Ok(()),
        Found::Open(callout, entry) => (callout, entry),
    };
    match entry.kind {
        CalloutKind::HttpCall => return Err(Status::NotFound.into()),
        // A call takes no more from either side, so it ends here and the
        // service hears the one method that takes a callout with no
        // delivery.
        CalloutKind::GrpcCall => end_callout(state, callout),
        CalloutKind::GrpcStream if !entry.closed_by_guest => {
            let caller = context(state, Status::NotFound)?;
            let call = invocation(state, caller);
            let service = Arc::clone(state.abi().services().callouts());
            state.abi_mut().callouts_mut().close_by_guest(callout);
            service.grpc_close(call, callout);
        }
        CalloutKind::GrpcStream => {}
    }
    Ok(())
}

/// Removes the callout and tells the service that it ended with no
/// delivery.
fn end_callout(state: &mut HostState, callout: CalloutId) {
    let Ok(caller) = context(state, Status::NotFound) else {
        return;
    };
    let call = invocation(state, caller);
    let service = Arc::clone(state.abi().services().callouts());
    state.abi_mut().callouts_mut().remove(callout);
    service.grpc_cancel(call, callout);
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::abi::v0_2_1::test_support::callouts::{
        GrpcAsk, RecordingCallouts, callout_hosted, callout_hosted_with_limits, services_with,
    };
    use crate::abi::v0_2_1::test_support::{engine, instance_with, outcome, wat_bytes, write};
    use crate::abi::v0_2_1::{Callback, ContextId, GrpcOpenRefusal, Invocation};
    use crate::codec::pairs::encode_pairs;
    use crate::runtime::{Engine, Instance, Limits, Module};

    const GUEST: &str = r#"(module
        (memory (export "memory") 1)
        (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 8192))"#;

    const UPSTREAM: i32 = 1024;
    const SERVICE: i32 = 2048;
    const METHOD: i32 = 3072;
    const METADATA: i32 = 4096;
    const MESSAGE: i32 = 5120;
    const RETURN_ID: i32 = 6000;
    const PAST_END: i32 = 65_534;

    /// The twelve arguments of a gRPC call, with the bytes in guest memory.
    struct Call([i32; 12]);

    fn metadata_bytes() -> Vec<u8> {
        encode_pairs(&[(b"k".as_slice(), b"v".as_slice())]).unwrap()
    }

    fn call_arguments(instance: &mut Instance, metadata: &[u8]) -> Call {
        let upstream = write(instance, UPSTREAM, b"authz");
        let service = write(instance, SERVICE, b"example.Authz");
        let method = write(instance, METHOD, b"Check");
        let initial = write(instance, METADATA, metadata);
        let message = write(instance, MESSAGE, b"hello");
        Call([
            upstream.0, upstream.1, service.0, service.1, method.0, method.1, initial.0, initial.1,
            message.0, message.1, 250, RETURN_ID,
        ])
    }

    fn grpc_call(instance: &mut Instance, call: &Call) -> Status {
        let a = call.0;
        outcome(proxy_grpc_call(
            instance.store_mut(),
            a[0],
            a[1],
            a[2],
            a[3],
            a[4],
            a[5],
            a[6],
            a[7],
            a[8],
            a[9],
            a[10],
            a[11],
        ))
    }

    fn grpc_stream(instance: &mut Instance, call: &Call) -> Status {
        let a = call.0;
        outcome(proxy_grpc_stream(
            instance.store_mut(),
            a[0],
            a[1],
            a[2],
            a[3],
            a[4],
            a[5],
            a[6],
            a[7],
            a[11],
        ))
    }

    /// An encoded map of `count` pairs, each one byte of key and value.
    fn big_metadata(count: usize) -> Vec<u8> {
        let pairs: Vec<(Vec<u8>, Vec<u8>)> = (0..count).map(|_| (vec![b'k'], vec![b'v'])).collect();
        encode_pairs(&pairs).unwrap()
    }

    /// The arguments of a call whose initial metadata sits clear of the
    /// other values.
    fn oversized(instance: &mut Instance) -> Call {
        let upstream = write(instance, UPSTREAM, b"authz");
        let service = write(instance, SERVICE, b"example.Authz");
        let method = write(instance, METHOD, b"Check");
        let initial = write(instance, 20_000, &big_metadata(1025));
        Call([
            upstream.0, upstream.1, service.0, service.1, method.0, method.1, initial.0, initial.1,
            MESSAGE, 0, 250, RETURN_ID,
        ])
    }

    #[test]
    fn grpc_metadata_above_the_pair_limit_is_a_parse_failure() {
        // Arrange
        let engine = engine();
        let service = Arc::new(RecordingCallouts::new());
        let (mut instance, _) = callout_hosted(&engine, GUEST, services_with(service.clone()));
        let call = oversized(&mut instance);

        // Act
        let result = grpc_call(&mut instance, &call);

        // Assert
        assert_eq!(
            result,
            Status::ParseFailure,
            "the ABI lists no BAD_ARGUMENT for the two functions that open a gRPC callout"
        );
        assert!(service.grpc_calls().is_empty());
    }

    #[test]
    fn grpc_metadata_above_the_byte_limit_is_a_parse_failure() {
        // Arrange
        let engine = engine();
        let service = Arc::new(RecordingCallouts::new());
        let limits = Limits::new().with_max_decoded_map_bytes(8);
        let (mut instance, _) =
            callout_hosted_with_limits(&engine, GUEST, services_with(service.clone()), &limits);
        let call = call_arguments(&mut instance, &metadata_bytes());

        // Act
        let result = grpc_call(&mut instance, &call);

        // Assert
        assert_eq!(result, Status::ParseFailure);
        assert!(service.grpc_calls().is_empty());
    }

    #[test]
    fn a_grpc_stream_refuses_metadata_above_the_pair_limit() {
        // Arrange
        let engine = engine();
        let service = Arc::new(RecordingCallouts::new());
        let (mut instance, _) = callout_hosted(&engine, GUEST, services_with(service.clone()));
        let call = oversized(&mut instance);

        // Act
        let result = grpc_stream(&mut instance, &call);

        // Assert
        assert_eq!(result, Status::ParseFailure);
        assert!(service.grpc_calls().is_empty());
    }

    fn written_id(instance: &mut Instance) -> u32 {
        instance
            .memory()
            .unwrap()
            .read_u32(GuestPtr::from_address(RETURN_ID.cast_unsigned()))
            .unwrap()
    }

    /// A hosted guest of `service` with an effective root and a running
    /// callback.
    fn hosted(engine: &Engine, service: &Arc<RecordingCallouts>) -> (Instance, ContextId) {
        callout_hosted(engine, GUEST, services_with(service.clone()))
    }

    /// Opens a callout of `kind` for `caller` under `root` and answers its
    /// identifier.
    fn open(instance: &mut Instance, kind: CalloutKind, caller: ContextId, root: ContextId) -> i32 {
        let table = instance.state_mut().abi_mut().callouts_mut();
        let id = table.reserve();
        table.enter(id, Callout::new(kind, caller, root));
        id.get().cast_signed()
    }

    fn send(instance: &mut Instance, id: i32, at: i32, bytes: &[u8], end: i32) -> Status {
        let (data, size) = write(instance, at, bytes);
        outcome(proxy_grpc_send(instance.store_mut(), id, data, size, end))
    }

    fn cancel(instance: &mut Instance, id: i32) -> Status {
        outcome(proxy_grpc_cancel(instance.store_mut(), id))
    }

    fn close(instance: &mut Instance, id: i32) -> Status {
        outcome(proxy_grpc_close(instance.store_mut(), id))
    }

    fn open_count(instance: &Instance) -> usize {
        instance.state().abi().callouts().len()
    }

    #[test]
    fn an_accepted_grpc_call_reaches_the_service_and_the_guest_gets_the_identifier() {
        // Arrange
        let engine = engine();
        let service = Arc::new(RecordingCallouts::new());
        let (mut instance, root) = hosted(&engine, &service);
        let arguments = call_arguments(&mut instance, &metadata_bytes());

        // Act
        let answer = grpc_call(&mut instance, &arguments);

        // Assert
        assert_eq!(answer, Status::Ok);
        assert_eq!(written_id(&mut instance), 1);
        let asked = service.grpc_calls();
        assert_eq!(asked.len(), 1);
        let (call, callout, ask) = &asked[0];
        let guest = instance.state().abi().guest();
        assert_eq!(
            *call,
            Invocation::new(guest, root).with_callback(Callback::RequestHeaders)
        );
        assert_eq!(callout.get(), 1);
        match ask {
            GrpcAsk::Call(request) => {
                assert_eq!(request.upstream.as_ref(), b"authz");
                assert_eq!(request.service.as_ref(), b"example.Authz");
                assert_eq!(request.method.as_ref(), b"Check");
                assert_eq!(request.initial_metadata.len(), 1);
                assert_eq!(request.message.as_ref(), b"hello");
                assert_eq!(request.timeout, Duration::from_millis(250));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(open_count(&instance), 1);
    }

    #[test]
    fn an_accepted_grpc_stream_reaches_the_service_and_takes_no_message() {
        // Arrange
        let engine = engine();
        let service = Arc::new(RecordingCallouts::new());
        let (mut instance, _) = hosted(&engine, &service);
        let arguments = call_arguments(&mut instance, &metadata_bytes());

        // Act
        let answer = grpc_stream(&mut instance, &arguments);

        // Assert
        assert_eq!(answer, Status::Ok);
        assert_eq!(written_id(&mut instance), 1);
        let asked = service.grpc_calls();
        assert_eq!(asked.len(), 1);
        match &asked[0].2 {
            GrpcAsk::Stream(request) => {
                assert_eq!(request.upstream.as_ref(), b"authz");
                assert_eq!(request.initial_metadata.len(), 1);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn every_pointer_of_the_two_openers_is_checked_before_the_service_is_asked() {
        // Arrange
        let engine = engine();
        let service = Arc::new(RecordingCallouts::new());
        let (mut instance, _) = hosted(&engine, &service);
        let good = call_arguments(&mut instance, &metadata_bytes());
        let mut cases = Vec::new();
        for index in [0, 2, 4, 6, 8, 11] {
            let mut broken = Call(good.0);
            broken.0[index] = PAST_END;
            cases.push(broken);
        }

        // Act
        let answers: Vec<Status> = cases
            .iter()
            .map(|case| grpc_call(&mut instance, case))
            .collect();

        // Assert
        assert_eq!(answers, vec![Status::InvalidMemoryAccess; 6]);
        assert!(service.grpc_calls().is_empty(), "the service was not asked");
        assert_eq!(open_count(&instance), 0);
    }

    #[test]
    fn metadata_that_does_not_decode_answers_parse_failure() {
        // Arrange
        let engine = engine();
        let service = Arc::new(RecordingCallouts::new());
        let (mut instance, _) = hosted(&engine, &service);
        let arguments = call_arguments(&mut instance, &[9, 9, 9, 9]);

        // Act
        let answer = grpc_call(&mut instance, &arguments);

        // Assert
        assert_eq!(answer, Status::ParseFailure);
        assert!(service.grpc_calls().is_empty());
        assert_eq!(open_count(&instance), 0);
    }

    #[test]
    fn a_null_empty_message_and_a_timeout_of_zero_are_accepted() {
        // Arrange
        let engine = engine();
        let service = Arc::new(RecordingCallouts::new());
        let (mut instance, _) = hosted(&engine, &service);
        let mut arguments = call_arguments(&mut instance, &metadata_bytes());
        arguments.0[8] = 0;
        arguments.0[9] = 0;
        arguments.0[10] = 0;

        // Act
        let answer = grpc_call(&mut instance, &arguments);

        // Assert
        assert_eq!(answer, Status::Ok);
        match &service.grpc_calls()[0].2 {
            GrpcAsk::Call(request) => {
                assert!(request.message.is_empty());
                assert_eq!(request.timeout, Duration::ZERO);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn each_refusal_of_an_opener_gives_its_status_and_releases_the_identifier() {
        // Arrange
        let engine = engine();
        let unknown =
            Arc::new(RecordingCallouts::new().refusing_grpc(GrpcOpenRefusal::UnknownUpstream));
        let failed = Arc::new(RecordingCallouts::new().refusing_grpc(GrpcOpenRefusal::Failed));
        let (mut first, _) = hosted(&engine, &unknown);
        let (mut second, _) = hosted(&engine, &failed);
        let one = call_arguments(&mut first, &metadata_bytes());
        let two = call_arguments(&mut second, &metadata_bytes());

        // Act
        let answers = [grpc_call(&mut first, &one), grpc_stream(&mut second, &two)];

        // Assert
        assert_eq!(answers, [Status::ParseFailure, Status::InternalFailure]);
        assert_eq!(open_count(&first), 0);
        assert_eq!(open_count(&second), 0);
    }

    #[test]
    fn a_guest_that_repeats_a_refused_grpc_call_does_not_grow_the_table() {
        // Arrange
        let engine = engine();
        let service = Arc::new(RecordingCallouts::new().refusing_grpc(GrpcOpenRefusal::Failed));
        let (mut instance, _) = hosted(&engine, &service);
        let arguments = call_arguments(&mut instance, &metadata_bytes());

        // Act
        let answers: Vec<Status> = (0..1000)
            .map(|_| grpc_call(&mut instance, &arguments))
            .collect();

        // Assert
        assert_eq!(answers, vec![Status::InternalFailure; 1000]);
        assert_eq!(open_count(&instance), 0);
        assert_eq!(service.grpc_calls().len(), 1000);
    }

    #[test]
    fn the_three_kinds_share_the_maximum_and_never_share_an_identifier() {
        // Arrange
        let engine = engine();
        let service = Arc::new(RecordingCallouts::new());
        let module = Module::new(&engine, &wat_bytes(GUEST)).unwrap();
        let services = services_with(service.clone()).with_max_open_callouts(2);
        let mut instance = instance_with(&engine, &module, services).unwrap();
        let root = instance
            .state_mut()
            .abi_mut()
            .contexts_mut()
            .create(None)
            .unwrap();
        instance
            .state_mut()
            .abi_mut()
            .contexts_mut()
            .set_effective(root);
        let arguments = call_arguments(&mut instance, &metadata_bytes());

        // Act
        let answers = [
            grpc_call(&mut instance, &arguments),
            grpc_stream(&mut instance, &arguments),
            grpc_call(&mut instance, &arguments),
        ];

        // Assert
        assert_eq!(
            answers,
            [Status::Ok, Status::Ok, Status::InternalFailure],
            "the third call meets the maximum of the three kinds together"
        );
        let identifiers: Vec<u32> = service
            .grpc_calls()
            .iter()
            .map(|(_, id, _)| id.get())
            .collect();
        assert_eq!(identifiers, vec![1, 2]);
        assert_eq!(open_count(&instance), 2);
    }

    #[test]
    fn an_opener_is_refused_without_an_effective_context() {
        // Arrange
        let engine = engine();
        let service = Arc::new(RecordingCallouts::new());
        let module = Module::new(&engine, &wat_bytes(GUEST)).unwrap();
        let mut instance = instance_with(&engine, &module, services_with(service.clone())).unwrap();
        let arguments = call_arguments(&mut instance, &metadata_bytes());

        // Act
        let answers = [
            grpc_call(&mut instance, &arguments),
            grpc_stream(&mut instance, &arguments),
        ];

        // Assert
        assert_eq!(answers, [Status::InternalFailure; 2]);
        assert!(service.grpc_calls().is_empty());
    }

    #[test]
    fn an_opener_is_refused_while_the_context_is_deleted() {
        // Arrange
        let engine = engine();
        let service = Arc::new(RecordingCallouts::new());
        let (mut instance, root) = hosted(&engine, &service);
        let arguments = call_arguments(&mut instance, &metadata_bytes());
        instance.state_mut().abi_mut().set_deleting(Some(root));

        // Act
        let answer = grpc_call(&mut instance, &arguments);

        // Assert
        assert_eq!(answer, Status::InternalFailure);
        assert!(service.grpc_calls().is_empty());
        assert_eq!(open_count(&instance), 0);
    }

    #[test]
    fn a_send_reaches_the_service_with_its_message_and_its_end_flag() {
        // Arrange
        let engine = engine();
        let service = Arc::new(RecordingCallouts::new());
        let (mut instance, root) = hosted(&engine, &service);
        let stream = open(&mut instance, CalloutKind::GrpcStream, root, root);

        // Act
        let answer = send(&mut instance, stream, MESSAGE, b"one", 7);

        // Assert
        assert_eq!(answer, Status::Ok);
        let asked = service.grpc_calls();
        assert_eq!(asked.len(), 1);
        assert_eq!(
            asked[0].2,
            GrpcAsk::Send {
                message: b"one".to_vec(),
                end_of_stream: true,
            },
            "every value but zero is an end of stream"
        );
        assert_eq!(open_count(&instance), 1, "the stream stays open");
    }

    #[test]
    fn a_send_checks_its_range_before_it_asks_the_service() {
        // Arrange
        let engine = engine();
        let service = Arc::new(RecordingCallouts::new());
        let (mut instance, root) = hosted(&engine, &service);
        let stream = open(&mut instance, CalloutKind::GrpcStream, root, root);

        // Act
        let answer = outcome(proxy_grpc_send(
            instance.store_mut(),
            stream,
            PAST_END,
            4,
            0,
        ));

        // Assert
        assert_eq!(answer, Status::InvalidMemoryAccess);
        assert!(service.grpc_calls().is_empty());
    }

    #[test]
    fn a_send_of_a_null_empty_message_reaches_the_service() {
        // Arrange
        let engine = engine();
        let service = Arc::new(RecordingCallouts::new());
        let (mut instance, root) = hosted(&engine, &service);
        let stream = open(&mut instance, CalloutKind::GrpcStream, root, root);

        // Act
        let answer = outcome(proxy_grpc_send(instance.store_mut(), stream, 0, 0, 0));

        // Assert
        assert_eq!(answer, Status::Ok);
        assert_eq!(
            service.grpc_calls()[0].2,
            GrpcAsk::Send {
                message: Vec::new(),
                end_of_stream: false,
            }
        );
    }

    #[test]
    fn a_send_the_crate_cannot_serve_is_not_found() {
        // Arrange
        let engine = engine();
        let service = Arc::new(RecordingCallouts::new());
        let (mut instance, root) = hosted(&engine, &service);
        let other = instance
            .state_mut()
            .abi_mut()
            .contexts_mut()
            .create(None)
            .unwrap();
        let unary = open(&mut instance, CalloutKind::GrpcCall, root, root);
        let http = open(&mut instance, CalloutKind::HttpCall, root, root);
        let theirs = open(&mut instance, CalloutKind::GrpcStream, other, other);

        // Act
        let answers = [
            send(&mut instance, 0, MESSAGE, b"a", 0),
            send(&mut instance, 99, MESSAGE, b"a", 0),
            send(&mut instance, unary, MESSAGE, b"a", 0),
            send(&mut instance, http, MESSAGE, b"a", 0),
            send(&mut instance, theirs, MESSAGE, b"a", 0),
        ];

        // Assert
        assert_eq!(answers, [Status::NotFound; 5]);
        assert!(service.grpc_calls().is_empty());
    }

    #[test]
    fn a_send_after_the_guest_closed_its_side_answers_ok_and_asks_nothing() {
        // Arrange
        let engine = engine();
        let service = Arc::new(RecordingCallouts::new());
        let (mut instance, root) = hosted(&engine, &service);
        let stream = open(&mut instance, CalloutKind::GrpcStream, root, root);
        assert_eq!(send(&mut instance, stream, MESSAGE, b"one", 1), Status::Ok);

        // Act
        let answer = send(&mut instance, stream, MESSAGE, b"two", 0);

        // Assert
        assert_eq!(answer, Status::Ok);
        assert_eq!(
            service.grpc_calls().len(),
            1,
            "only the first send was asked"
        );
        assert_eq!(open_count(&instance), 1);
    }

    #[test]
    fn a_cancel_ends_a_call_and_a_stream_and_tells_the_service_one_time() {
        // Arrange
        let engine = engine();
        let service = Arc::new(RecordingCallouts::new());
        let (mut instance, root) = hosted(&engine, &service);
        let unary = open(&mut instance, CalloutKind::GrpcCall, root, root);
        let stream = open(&mut instance, CalloutKind::GrpcStream, root, root);

        // Act
        let answers = [
            cancel(&mut instance, unary),
            cancel(&mut instance, stream),
            cancel(&mut instance, unary),
        ];

        // Assert
        assert_eq!(answers, [Status::Ok; 3]);
        assert_eq!(open_count(&instance), 0);
        let asks: Vec<GrpcAsk> = service
            .grpc_calls()
            .into_iter()
            .map(|(_, _, ask)| ask)
            .collect();
        assert_eq!(asks, vec![GrpcAsk::Cancel, GrpcAsk::Cancel]);
    }

    #[test]
    fn a_close_ends_a_call_through_the_cancel_of_the_service() {
        // Arrange
        let engine = engine();
        let service = Arc::new(RecordingCallouts::new());
        let (mut instance, root) = hosted(&engine, &service);
        let unary = open(&mut instance, CalloutKind::GrpcCall, root, root);

        // Act
        let answer = close(&mut instance, unary);

        // Assert
        assert_eq!(answer, Status::Ok);
        assert_eq!(open_count(&instance), 0);
        assert_eq!(service.grpc_calls()[0].2, GrpcAsk::Cancel);
    }

    #[test]
    fn a_close_of_a_stream_keeps_it_and_tells_the_service_one_time() {
        // Arrange
        let engine = engine();
        let service = Arc::new(RecordingCallouts::new());
        let (mut instance, root) = hosted(&engine, &service);
        let stream = open(&mut instance, CalloutKind::GrpcStream, root, root);

        // Act
        let answers = [close(&mut instance, stream), close(&mut instance, stream)];

        // Assert
        assert_eq!(answers, [Status::Ok; 2]);
        assert_eq!(open_count(&instance), 1);
        let asks: Vec<GrpcAsk> = service
            .grpc_calls()
            .into_iter()
            .map(|(_, _, ask)| ask)
            .collect();
        assert_eq!(asks, vec![GrpcAsk::Close]);
    }

    #[test]
    fn a_close_after_a_send_that_ends_the_stream_asks_nothing_more() {
        // Arrange
        let engine = engine();
        let service = Arc::new(RecordingCallouts::new());
        let (mut instance, root) = hosted(&engine, &service);
        let stream = open(&mut instance, CalloutKind::GrpcStream, root, root);
        assert_eq!(send(&mut instance, stream, MESSAGE, b"last", 1), Status::Ok);

        // Act
        let answer = close(&mut instance, stream);

        // Assert
        assert_eq!(answer, Status::Ok);
        let asks: Vec<GrpcAsk> = service
            .grpc_calls()
            .into_iter()
            .map(|(_, _, ask)| ask)
            .collect();
        assert_eq!(
            asks,
            vec![GrpcAsk::Send {
                message: b"last".to_vec(),
                end_of_stream: true,
            }]
        );
    }

    #[test]
    fn a_cancel_and_a_close_the_crate_cannot_serve_are_not_found() {
        // Arrange
        let engine = engine();
        let service = Arc::new(RecordingCallouts::new());
        let (mut instance, root) = hosted(&engine, &service);
        let other = instance
            .state_mut()
            .abi_mut()
            .contexts_mut()
            .create(None)
            .unwrap();
        let http = open(&mut instance, CalloutKind::HttpCall, root, root);
        let theirs = open(&mut instance, CalloutKind::GrpcCall, other, other);

        // Act
        let answers = [
            cancel(&mut instance, 0),
            cancel(&mut instance, -1),
            cancel(&mut instance, 99),
            cancel(&mut instance, http),
            cancel(&mut instance, theirs),
            close(&mut instance, 0),
            close(&mut instance, 99),
            close(&mut instance, http),
            close(&mut instance, theirs),
        ];

        // Assert
        assert_eq!(answers, [Status::NotFound; 9]);
        assert!(service.grpc_calls().is_empty());
        assert_eq!(open_count(&instance), 2, "nothing was removed");
    }

    #[test]
    fn the_three_functions_answer_ok_for_a_callout_that_ended() {
        // Arrange
        let engine = engine();
        let service = Arc::new(RecordingCallouts::new());
        let (mut instance, root) = hosted(&engine, &service);
        let stream = open(&mut instance, CalloutKind::GrpcStream, root, root);
        assert_eq!(cancel(&mut instance, stream), Status::Ok);
        let asked = service.grpc_calls().len();

        // Act
        let answers = [
            send(&mut instance, stream, MESSAGE, b"late", 0),
            cancel(&mut instance, stream),
            close(&mut instance, stream),
        ];

        // Assert
        assert_eq!(
            answers,
            [Status::Ok; 3],
            "a guest of the Rust SDK panics on any other status"
        );
        assert_eq!(
            service.grpc_calls().len(),
            asked,
            "the service was not asked again"
        );
    }

    #[test]
    fn the_three_functions_are_not_found_without_an_effective_context() {
        // Arrange
        let engine = engine();
        let service = Arc::new(RecordingCallouts::new());
        let module = Module::new(&engine, &wat_bytes(GUEST)).unwrap();
        let mut instance = instance_with(&engine, &module, services_with(service.clone())).unwrap();
        let root = instance
            .state_mut()
            .abi_mut()
            .contexts_mut()
            .create(None)
            .unwrap();
        let stream = open(&mut instance, CalloutKind::GrpcStream, root, root);

        // Act
        let answers = [
            send(&mut instance, stream, MESSAGE, b"a", 0),
            cancel(&mut instance, stream),
            close(&mut instance, stream),
        ];

        // Assert
        assert_eq!(answers, [Status::NotFound; 3]);
        assert!(service.grpc_calls().is_empty());
        assert_eq!(open_count(&instance), 1);
    }

    #[test]
    fn an_opener_is_refused_under_a_refused_root() {
        // Arrange
        let engine = engine();
        let service = Arc::new(RecordingCallouts::new());
        let (mut instance, root) = hosted(&engine, &service);
        let arguments = call_arguments(&mut instance, &metadata_bytes());
        instance.state_mut().abi_mut().contexts_mut().reject(root);

        // Act
        let answers = [
            grpc_call(&mut instance, &arguments),
            grpc_stream(&mut instance, &arguments),
        ];

        // Assert
        assert_eq!(answers, [Status::InternalFailure; 2]);
        assert!(service.grpc_calls().is_empty());
        assert_eq!(open_count(&instance), 0);
    }

    #[test]
    fn one_context_does_not_reach_the_callout_of_another_context_of_its_root() {
        // Arrange
        let engine = engine();
        let service = Arc::new(RecordingCallouts::new());
        let (mut instance, root) = hosted(&engine, &service);
        let contexts = (
            instance
                .state_mut()
                .abi_mut()
                .contexts_mut()
                .create(Some(root))
                .unwrap(),
            instance
                .state_mut()
                .abi_mut()
                .contexts_mut()
                .create(Some(root))
                .unwrap(),
        );
        let theirs = open(&mut instance, CalloutKind::GrpcStream, contexts.0, root);
        instance
            .state_mut()
            .abi_mut()
            .contexts_mut()
            .set_effective(contexts.1);

        // Act
        let answers = [
            send(&mut instance, theirs, MESSAGE, b"a", 0),
            cancel(&mut instance, theirs),
            close(&mut instance, theirs),
        ];

        // Assert
        assert_eq!(answers, [Status::NotFound; 3]);
        assert!(service.grpc_calls().is_empty());
        assert_eq!(open_count(&instance), 1, "the callout of the first stays");
    }

    #[test]
    fn the_context_that_opened_a_callout_reaches_it() {
        // Arrange
        let engine = engine();
        let service = Arc::new(RecordingCallouts::new());
        let (mut instance, root) = hosted(&engine, &service);
        let stream = instance
            .state_mut()
            .abi_mut()
            .contexts_mut()
            .create(Some(root))
            .unwrap();
        let mine = open(&mut instance, CalloutKind::GrpcStream, stream, root);
        instance
            .state_mut()
            .abi_mut()
            .contexts_mut()
            .set_effective(stream);

        // Act
        let answer = send(&mut instance, mine, MESSAGE, b"a", 0);

        // Assert
        assert_eq!(answer, Status::Ok);
        let asked = service.grpc_calls();
        assert_eq!(asked.len(), 1);
        assert_eq!(asked[0].0.context, stream, "the service hears the opener");
    }
}
