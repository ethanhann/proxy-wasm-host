//! `proxy_http_call` and `proxy_get_status`.

use std::borrow::Cow;
use std::sync::Arc;
use std::time::Duration;

use wasmtime::AsContextMut;

use crate::abi::v0_2_1::AbiAccess;
use crate::abi::v0_2_1::callout::{Callout, Delivery};
use crate::abi::v0_2_1::host_functions::Failure;
use crate::abi::v0_2_1::host_functions::call::{context, invocation};
use crate::abi::v0_2_1::types::Status;
use crate::abi::v0_2_1::{CalloutKind, HeaderPairs, HttpCall};
use crate::codec::pairs::decode_pairs;
use crate::runtime::{GuestPtr, GuestSlice, HostState, split, write_return};

/// The header names an HTTP call must have, which the ABI requires.
const REQUIRED_HEADERS: [&[u8]; 3] = [b":authority", b":method", b":path"];

fn borrowed<'a>(pairs: Vec<(&'a [u8], &'a [u8])>) -> HeaderPairs<'a> {
    pairs
        .into_iter()
        .map(|(key, value)| (Cow::Borrowed(key), Cow::Borrowed(value)))
        .collect()
}

/// Starts an HTTP call through the callout service.
///
/// Every pointer is read before the service is asked, which includes the four
/// bytes the identifier goes to.
/// The write at the end therefore cannot fail, and the guest gets no error
/// for a callout that is open.
#[expect(
    clippy::too_many_arguments,
    reason = "the ABI fixes the parameter list of this host function"
)]
pub(super) fn proxy_http_call(
    ctx: &mut impl AsContextMut<Data = HostState>,
    upstream_name_data: i32,
    upstream_name_size: i32,
    serialized_headers_data: i32,
    serialized_headers_size: i32,
    body_data: i32,
    body_size: i32,
    serialized_trailers_data: i32,
    serialized_trailers_size: i32,
    timeout: i32,
    return_call_id: i32,
) -> Result<(), Failure> {
    let upstream = GuestSlice::try_from((upstream_name_data, upstream_name_size))?;
    let headers = GuestSlice::try_from((serialized_headers_data, serialized_headers_size))?;
    let body = GuestSlice::try_from((body_data, body_size))?;
    let trailers = GuestSlice::try_from((serialized_trailers_data, serialized_trailers_size))?;
    let id_ptr = GuestPtr::try_from(return_call_id)?;
    let timeout = Duration::from_millis(u64::from(timeout.cast_unsigned()));
    let (mut memory, state) = split(ctx)?;
    memory.read_u32(id_ptr)?;
    let request = HttpCall::new(Cow::Borrowed(memory.read(upstream)?))
        .with_headers(borrowed(decode_pairs(memory.read(headers)?)?))
        .with_body(Cow::Borrowed(memory.read(body)?))
        .with_trailers(borrowed(decode_pairs(memory.read(trailers)?)?))
        .with_timeout(timeout);
    let named = |name: &[u8]| request.headers.iter().any(|(key, _)| key.as_ref() == name);
    if !REQUIRED_HEADERS.into_iter().all(named) {
        return Err(Status::BadArgument.into());
    }

    let caller = context(state, Status::InternalFailure)?;
    let root = state
        .abi()
        .contexts()
        .root_of(caller)
        .ok_or(Status::InternalFailure)?;
    let maximum = state.abi().services().max_open_callouts();
    if state.abi().callouts().len() >= maximum {
        tracing::warn!(maximum, context = %caller, "the guest has its maximum of open callouts");
        return Err(Status::InternalFailure.into());
    }
    let call = invocation(state, caller);
    let service = Arc::clone(state.abi().services().callouts());
    let callout = state.abi_mut().callouts_mut().reserve();
    service
        .http_call(call, callout, request)
        .map_err(Status::from)?;
    let entry = Callout {
        kind: CalloutKind::HttpCall,
        caller,
        root,
    };
    state.abi_mut().callouts_mut().enter(callout, entry);
    memory.write_u32(id_ptr, callout.get())?;
    Ok(())
}

/// Answers the status of the callout that the running callback delivers.
///
/// The ABI gives the status of an HTTP call no meaning, so a delivered HTTP
/// call response answers code zero and an empty message.
/// No context and no stream state is asked.
pub(super) fn proxy_get_status(
    ctx: &mut impl AsContextMut<Data = HostState>,
    return_status_code: i32,
    return_status_message_data: i32,
    return_status_message_size: i32,
) -> Result<(), Failure> {
    let code_ptr = GuestPtr::try_from(return_status_code)?;
    let data_ptr = GuestPtr::try_from(return_status_message_data)?;
    let size_ptr = GuestPtr::try_from(return_status_message_size)?;
    let (memory, state) = split(ctx)?;
    memory.read_u32(code_ptr)?;
    memory.read_u32(data_ptr)?;
    memory.read_u32(size_ptr)?;
    let (code, message): (u32, &[u8]) = match state.abi().delivery() {
        Some(Delivery::HttpCallResponse(_)) => (0, &[]),
        None => return Err(Status::NotFound.into()),
    };
    write_return(ctx, message, data_ptr, size_ptr)?;
    let (mut memory, _) = split(ctx)?;
    memory.write_u32(code_ptr, code)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::v0_2_1::test_support::callouts::{
        RecordingCallouts, callout_hosted, services_with,
    };
    use crate::abi::v0_2_1::test_support::{
        engine, instance_with, outcome, returned, services, wat_bytes, write,
    };
    use crate::abi::v0_2_1::{Callback, CalloutId, HttpCallRefusal, HttpCallResponse, Invocation};
    use crate::codec::pairs::encode_pairs;
    use crate::runtime::{Engine, Instance, Module};

    const GUEST: &str = r#"(module
        (memory (export "memory") 1)
        (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 8192))"#;

    const UPSTREAM: i32 = 1024;
    const HEADERS: i32 = 2048;
    const BODY: i32 = 3072;
    const TRAILERS: i32 = 4096;
    const RETURN_ID: i32 = 5000;
    const PAST_END: i32 = 65_534;

    const REQUEST: [(&[u8], &[u8]); 3] = [
        (b":authority", b"authz"),
        (b":method", b"GET"),
        (b":path", b"/check"),
    ];

    /// The ten arguments of a call, with the bytes written to the guest.
    struct Arguments([i32; 10]);

    fn arguments(instance: &mut Instance, headers: &[u8]) -> Arguments {
        let upstream = write(instance, UPSTREAM, b"authz");
        let headers = write(instance, HEADERS, headers);
        let body = write(instance, BODY, b"hello");
        let trailers = write(instance, TRAILERS, &encode_pairs(&[(b"t", b"v")]).unwrap());
        Arguments([
            upstream.0, upstream.1, headers.0, headers.1, body.0, body.1, trailers.0, trailers.1,
            250, RETURN_ID,
        ])
    }

    /// A hosted guest of `service` and the arguments of a call with
    /// `headers`.
    fn case(
        engine: &Engine,
        service: &Arc<RecordingCallouts>,
        headers: &[(&[u8], &[u8])],
    ) -> (Instance, Arguments) {
        let (mut instance, _) = callout_hosted(engine, GUEST, services_with(service.clone()));
        let arguments = arguments(&mut instance, &encode_pairs(headers).unwrap());
        (instance, arguments)
    }

    fn http_call(instance: &mut Instance, arguments: &Arguments) -> Status {
        let a = arguments.0;
        outcome(proxy_http_call(
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
        ))
    }

    fn written_id(instance: &mut Instance) -> u32 {
        instance
            .memory()
            .unwrap()
            .read_u32(GuestPtr::from_address(RETURN_ID.cast_unsigned()))
            .unwrap()
    }

    fn open(instance: &Instance) -> usize {
        instance.state().abi().callouts().len()
    }

    #[test]
    fn an_accepted_call_reaches_the_service_and_the_guest_gets_the_identifier() {
        // Arrange
        let engine = engine();
        let service = Arc::new(RecordingCallouts::new());
        let (mut instance, root) = callout_hosted(&engine, GUEST, services_with(service.clone()));
        let arguments = arguments(&mut instance, &encode_pairs(&REQUEST).unwrap());

        // Act
        let answer = http_call(&mut instance, &arguments);

        // Assert
        assert_eq!(answer, Status::Ok);
        assert_eq!(written_id(&mut instance), 1);
        let calls = service.calls();
        assert_eq!(calls.len(), 1);
        let (call, callout, request) = &calls[0];
        assert_eq!(
            *call,
            Invocation::new(root).with_callback(Callback::RequestHeaders)
        );
        assert_eq!(callout.get(), 1);
        assert_eq!(request.upstream.as_ref(), b"authz");
        assert_eq!(request.headers.len(), 3);
        assert_eq!(request.body.as_ref(), b"hello");
        assert_eq!(request.trailers.len(), 1);
        assert_eq!(request.timeout, Duration::from_millis(250));
        let entry = instance.state().abi().callouts().get(*callout);
        assert_eq!(
            entry,
            Some(Callout {
                kind: CalloutKind::HttpCall,
                caller: root,
                root
            })
        );
    }

    #[test]
    fn each_refusal_gives_its_status_and_leaves_the_table_empty() {
        // Arrange
        let engine = engine();
        let refusals = [HttpCallRefusal::UnknownUpstream, HttpCallRefusal::Failed];
        let mut cases = refusals.map(|refusal| {
            let service = Arc::new(RecordingCallouts::new().refusing(refusal));
            case(&engine, &service, &REQUEST)
        });

        // Act
        let answers = cases
            .each_mut()
            .map(|(instance, arguments)| http_call(instance, arguments));

        // Assert
        assert_eq!(answers, [Status::BadArgument, Status::InternalFailure]);
        for (instance, _) in &mut cases {
            assert_eq!((open(instance), written_id(instance)), (0, 0));
        }
    }

    #[test]
    fn a_guest_that_repeats_a_refused_call_does_not_grow_the_table() {
        // Arrange
        let engine = engine();
        let service = Arc::new(RecordingCallouts::new().refusing(HttpCallRefusal::Failed));
        let (mut instance, _) = callout_hosted(&engine, GUEST, services_with(service.clone()));
        let arguments = arguments(&mut instance, &encode_pairs(&REQUEST).unwrap());

        // Act
        let answers: Vec<Status> = (0..1000)
            .map(|_| http_call(&mut instance, &arguments))
            .collect();

        // Assert
        assert!(answers.iter().all(|s| *s == Status::InternalFailure));
        assert_eq!(open(&instance), 0);
        assert_eq!(service.calls().len(), 1000);
    }

    #[test]
    fn services_with_no_callout_service_refuse_as_an_internal_failure() {
        // Arrange
        let engine = engine();
        let (mut instance, _) = callout_hosted(&engine, GUEST, services());
        let arguments = arguments(&mut instance, &encode_pairs(&REQUEST).unwrap());

        // Act
        let answer = http_call(&mut instance, &arguments);

        // Assert
        assert_eq!(answer, Status::InternalFailure);
        assert_eq!(open(&instance), 0);
    }

    #[test]
    fn every_pointer_is_checked_before_the_service_is_asked() {
        // Arrange
        let engine = engine();
        let service = Arc::new(RecordingCallouts::new());
        let (mut instance, good) = case(&engine, &service, &REQUEST);
        let bad = [0_usize, 2, 4, 6, 9].map(|at| {
            let mut arguments = good.0;
            arguments[at] = PAST_END;
            Arguments(arguments)
        });

        // Act
        let answers = bad
            .each_ref()
            .map(|arguments| http_call(&mut instance, arguments));

        // Assert
        assert_eq!(answers, [Status::InvalidMemoryAccess; 5]);
        assert!(service.calls().is_empty());
        assert_eq!(open(&instance), 0);
    }

    #[test]
    fn headers_that_lack_a_required_name_are_a_bad_argument() {
        // Arrange
        let engine = engine();
        let service = Arc::new(RecordingCallouts::new());
        let lacking: [Vec<(&[u8], &[u8])>; 4] = [
            REQUEST[1..].to_vec(),
            vec![REQUEST[0], REQUEST[2]],
            REQUEST[..2].to_vec(),
            vec![(b":Authority", b"authz"), REQUEST[1], REQUEST[2]],
        ];
        let mut cases = lacking.map(|headers| case(&engine, &service, &headers));

        // Act
        let answers = cases
            .each_mut()
            .map(|(instance, arguments)| http_call(instance, arguments));

        // Assert
        assert_eq!(answers, [Status::BadArgument; 4]);
        assert!(service.calls().is_empty());
    }

    #[test]
    fn a_map_that_does_not_decode_is_a_bad_argument() {
        // Arrange
        let engine = engine();
        let service = Arc::new(RecordingCallouts::new());
        let (mut instance, _) = callout_hosted(&engine, GUEST, services_with(service.clone()));
        let arguments = arguments(&mut instance, &[9, 0, 0, 0, 1]);

        // Act
        let answer = http_call(&mut instance, &arguments);

        // Assert
        assert_eq!(answer, Status::BadArgument);
        assert!(service.calls().is_empty());
    }

    #[test]
    fn an_empty_body_and_the_three_empty_trailer_forms_are_accepted() {
        // Arrange
        let engine = engine();
        let service = Arc::new(RecordingCallouts::new());
        let forms: [&[u8]; 3] = [&[], &[0], &[0, 0, 0, 0]];
        let mut cases = forms.map(|form| {
            let (mut instance, mut arguments) = case(&engine, &service, &REQUEST);
            let trailers = write(&mut instance, TRAILERS, form);
            (arguments.0[4], arguments.0[5]) = (0, 0);
            (arguments.0[6], arguments.0[7]) = trailers;
            (instance, arguments)
        });

        // Act
        let answers = cases
            .each_mut()
            .map(|(instance, arguments)| http_call(instance, arguments));

        // Assert
        assert_eq!(answers, [Status::Ok; 3]);
        let calls = service.calls();
        assert_eq!(calls.len(), 3);
        for (_, _, request) in &calls {
            assert!(request.body.is_empty() && request.trailers.is_empty());
        }
    }

    #[test]
    fn a_timeout_of_zero_reaches_the_service_as_zero() {
        // Arrange
        let engine = engine();
        let service = Arc::new(RecordingCallouts::new());
        let (mut instance, mut arguments) = case(&engine, &service, &REQUEST);
        arguments.0[8] = 0;

        // Act
        let answer = http_call(&mut instance, &arguments);

        // Assert
        assert_eq!(answer, Status::Ok);
        assert_eq!(service.calls()[0].2.timeout, Duration::ZERO);
    }

    #[test]
    fn no_effective_context_and_a_refused_root_are_an_internal_failure() {
        // Arrange
        let engine = engine();
        let service = Arc::new(RecordingCallouts::new());
        let module = Module::new(&engine, &wat_bytes(GUEST)).unwrap();
        let mut bare = instance_with(&engine, &module, services_with(service.clone())).unwrap();
        let bare_arguments = arguments(&mut bare, &encode_pairs(&REQUEST).unwrap());
        let (mut refused, refused_arguments) = case(&engine, &service, &REQUEST);
        let root = refused.state().abi().contexts().effective().unwrap();
        refused.state_mut().abi_mut().contexts_mut().reject(root);

        // Act
        let answers = [
            http_call(&mut bare, &bare_arguments),
            http_call(&mut refused, &refused_arguments),
        ];

        // Assert
        assert_eq!(answers, [Status::InternalFailure; 2]);
        assert!(service.calls().is_empty());
    }

    /// A case whose guest may have two open callouts and has `count`.
    fn at_most_two(service: &Arc<RecordingCallouts>, count: usize) -> (Instance, Arguments) {
        let services = services_with(service.clone()).with_max_open_callouts(2);
        let (mut instance, root) = callout_hosted(&engine(), GUEST, services);
        let arguments = arguments(&mut instance, &encode_pairs(&REQUEST).unwrap());
        let table = instance.state_mut().abi_mut().callouts_mut();
        for _ in 0..count {
            let id = table.reserve();
            let kind = CalloutKind::HttpCall;
            table.enter(
                id,
                Callout {
                    kind,
                    caller: root,
                    root,
                },
            );
        }
        (instance, arguments)
    }

    #[test]
    fn a_call_at_the_maximum_is_refused_and_the_service_is_not_asked() {
        // Arrange
        let service = Arc::new(RecordingCallouts::new());
        let (mut instance, arguments) = at_most_two(&service, 2);

        // Act
        let answer = http_call(&mut instance, &arguments);

        // Assert
        assert_eq!(answer, Status::InternalFailure);
        assert!(service.calls().is_empty());
        assert_eq!((open(&instance), written_id(&mut instance)), (2, 0));
    }

    #[test]
    fn a_call_below_the_maximum_is_accepted() {
        // Arrange
        let service = Arc::new(RecordingCallouts::new());
        let (mut instance, arguments) = at_most_two(&service, 1);

        // Act
        let answer = http_call(&mut instance, &arguments);

        // Assert
        assert_eq!(answer, Status::Ok);
        assert_eq!((open(&instance), written_id(&mut instance)), (2, 2));
    }

    #[test]
    fn a_maximum_below_the_open_count_keeps_the_open_callouts() {
        // Arrange
        let service = Arc::new(RecordingCallouts::new());
        let (mut instance, arguments) = at_most_two(&service, 2);
        let services = instance.state_mut().abi_mut().services_mut();
        *services = services.clone().with_max_open_callouts(1);

        // Act
        let answer = http_call(&mut instance, &arguments);

        // Assert
        assert_eq!(answer, Status::InternalFailure);
        assert_eq!(open(&instance), 2);
    }

    const RETURN_CODE: i32 = 6000;
    const RETURN_DATA: i32 = 6004;
    const RETURN_SIZE: i32 = 6008;

    fn get_status(instance: &mut Instance) -> Status {
        outcome(proxy_get_status(
            instance.store_mut(),
            RETURN_CODE,
            RETURN_DATA,
            RETURN_SIZE,
        ))
    }

    fn deliver(instance: &mut Instance, response: HttpCallResponse<'static>) {
        let callout = CalloutId::try_from(1_u32).unwrap();
        instance
            .state_mut()
            .abi_mut()
            .set_delivery(Some(Delivery::http_call_response(callout, response)));
    }

    fn code(instance: &mut Instance) -> u32 {
        instance
            .memory()
            .unwrap()
            .read_u32(GuestPtr::from_address(RETURN_CODE.cast_unsigned()))
            .unwrap()
    }

    #[test]
    fn the_status_is_not_found_with_no_delivery() {
        // Arrange
        let engine = engine();
        let (mut instance, _) = callout_hosted(&engine, GUEST, services());

        // Act
        let answer = get_status(&mut instance);

        // Assert
        assert_eq!(answer, Status::NotFound);
    }

    #[test]
    fn a_delivered_http_call_answers_code_zero_and_an_empty_message() {
        // Arrange
        let engine = engine();
        let (mut instance, _) = callout_hosted(&engine, GUEST, services());
        instance
            .memory()
            .unwrap()
            .write_u32(GuestPtr::from_address(RETURN_CODE.cast_unsigned()), 77)
            .unwrap();
        deliver(&mut instance, HttpCallResponse::failed());

        // Act
        let answer = get_status(&mut instance);

        // Assert
        assert_eq!(answer, Status::Ok);
        assert_eq!(code(&mut instance), 0);
        assert!(
            returned(
                &mut instance,
                RETURN_DATA.cast_unsigned(),
                RETURN_SIZE.cast_unsigned()
            )
            .is_empty()
        );
    }

    #[test]
    fn every_status_pointer_is_checked_before_the_delivery_is_read() {
        // Arrange
        let engine = engine();
        let (mut instance, _) = callout_hosted(&engine, GUEST, services());
        deliver(&mut instance, HttpCallResponse::failed());
        let calls = [
            (PAST_END, RETURN_DATA, RETURN_SIZE),
            (RETURN_CODE, PAST_END, RETURN_SIZE),
            (RETURN_CODE, RETURN_DATA, PAST_END),
        ];

        // Act
        let answers =
            calls.map(|(c, d, s)| outcome(proxy_get_status(instance.store_mut(), c, d, s)));

        // Assert
        assert_eq!(answers, [Status::InvalidMemoryAccess; 3]);
    }
}
