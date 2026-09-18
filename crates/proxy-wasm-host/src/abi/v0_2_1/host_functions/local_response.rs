//! `proxy_send_local_response`.

use std::borrow::Cow;

use wasmtime::AsContextMut;

use crate::abi::v0_2_1::LocalResponse;
use crate::abi::v0_2_1::host_functions::Failure;
use crate::abi::v0_2_1::host_functions::call::{from_embedder, with_stream};
use crate::abi::v0_2_1::types::Status;
use crate::codec::pairs::decode_pairs;
use crate::runtime::{GuestSlice, HostState, split};

/// The value every SDK sends when a response carries no gRPC status.
const NO_GRPC_STATUS: i32 = -1;

#[expect(
    clippy::too_many_arguments,
    reason = "the ABI fixes the parameter list of this host function"
)]
pub(super) fn proxy_send_local_response(
    ctx: &mut impl AsContextMut<Data = HostState>,
    status_code: i32,
    status_code_details_data: i32,
    status_code_details_size: i32,
    body_data: i32,
    body_size: i32,
    serialized_headers_data: i32,
    serialized_headers_size: i32,
    grpc_status: i32,
) -> Result<(), Failure> {
    let status_code = status_code.cast_unsigned();
    let details = GuestSlice::try_from((status_code_details_data, status_code_details_size))?;
    let body = GuestSlice::try_from((body_data, body_size))?;
    let headers = GuestSlice::try_from((serialized_headers_data, serialized_headers_size))?;
    let grpc_status = (grpc_status != NO_GRPC_STATUS).then(|| grpc_status.cast_unsigned());
    let (memory, state) = split(ctx)?;
    let details = memory.read(details)?;
    let body = memory.read(body)?;
    let headers = decode_pairs(memory.read(headers)?)?
        .into_iter()
        .map(|(key, value)| (Cow::Borrowed(key), Cow::Borrowed(value)))
        .collect();
    let mut response = LocalResponse::new(status_code)
        .with_status_code_details(Cow::Borrowed(details))
        .with_body(Cow::Borrowed(body))
        .with_headers(headers);
    if let Some(grpc_status) = grpc_status {
        response = response.with_grpc_status(grpc_status);
    }
    let (call, stream) = with_stream(state, Status::Unimplemented)?;
    from_embedder(
        "send_local_response",
        stream.send_local_response(call, response),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::v0_2_1::AbiAccess;
    use crate::abi::v0_2_1::test_support::{
        RecordingStream, bare, hosted, outcome, status, unhosted, write,
    };
    use crate::abi::v0_2_1::{Callback, NoStream};
    use crate::codec::pairs::encode_pairs;
    use crate::runtime::Instance;
    use crate::runtime::test_support::engine;

    const DETAILS: i32 = 1024;
    const BODY: i32 = 1100;
    const HEADERS: i32 = 1200;
    const PAST_END: i32 = 65_534;

    const GUEST: &str = r#"(module
        (import "env" "proxy_send_local_response"
            (func $send (param i32 i32 i32 i32 i32 i32 i32 i32) (result i32)))
        (memory (export "memory") 1)
        (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 4096)
        (func (export "send") (param i32 i32 i32 i32 i32 i32 i32 i32) (result i32)
            local.get 0 local.get 1 local.get 2 local.get 3
            local.get 4 local.get 5 local.get 6 local.get 7 call $send))"#;

    fn headers(instance: &mut Instance) -> (i32, i32) {
        let encoded = encode_pairs(&[(b"a".as_slice(), b"1".as_slice())]).unwrap();
        write(instance, HEADERS, &encoded)
    }

    #[test]
    fn the_whole_response_reaches_the_embedder() {
        // Arrange
        let engine = engine();
        let (mut instance, root) = hosted(&engine, GUEST, RecordingStream::new());
        let (details, details_len) = write(&mut instance, DETAILS, b"denied");
        let (body, body_len) = write(&mut instance, BODY, b"no");
        let (pairs, pairs_len) = headers(&mut instance);

        // Act
        let result = instance
            .call::<(i32, i32, i32, i32, i32, i32, i32, i32), i32>(
                "send",
                (
                    403,
                    details,
                    details_len,
                    body,
                    body_len,
                    pairs,
                    pairs_len,
                    -1,
                ),
            )
            .map(status);

        // Assert
        assert_eq!(result.unwrap(), Status::Ok);
        let stream = RecordingStream::take(instance.state_mut());
        let (call, response) = stream.local_response().expect("a response was recorded");
        assert_eq!(response.status_code, 403);
        assert_eq!(response.status_code_details.as_ref(), b"denied");
        assert_eq!(response.body.as_ref(), b"no");
        assert_eq!(response.headers.len(), 1);
        assert_eq!(response.headers[0].0.as_ref(), b"a");
        assert_eq!(response.grpc_status, None);
        assert_eq!(call.context, root);
        assert_eq!(call.callback, Some(Callback::RequestHeaders));
    }

    #[test]
    fn a_grpc_status_that_is_not_the_absent_value_reaches_the_embedder() {
        // Arrange
        let engine = engine();
        let (mut instance, _) = hosted(&engine, GUEST, RecordingStream::new());
        let (pairs, pairs_len) = headers(&mut instance);

        // Act
        let result = instance
            .call::<(i32, i32, i32, i32, i32, i32, i32, i32), i32>(
                "send",
                (200, 0, 0, 0, 0, pairs, pairs_len, 14),
            )
            .map(status);

        // Assert
        assert_eq!(result.unwrap(), Status::Ok);
        let stream = RecordingStream::take(instance.state_mut());
        assert_eq!(stream.local_response().unwrap().1.grpc_status, Some(14));
    }

    #[test]
    fn a_null_pointer_with_a_zero_size_is_an_empty_value() {
        // Arrange
        let engine = engine();
        let (mut instance, _) = hosted(&engine, GUEST, RecordingStream::new());

        // Act
        let result = instance
            .call::<(i32, i32, i32, i32, i32, i32, i32, i32), i32>(
                "send",
                (200, 0, 0, 0, 0, 0, 0, -1),
            )
            .map(status);

        // Assert
        assert_eq!(result.unwrap(), Status::Ok);
        let stream = RecordingStream::take(instance.state_mut());
        let (_, response) = stream.local_response().unwrap();
        assert!(response.status_code_details.is_empty());
        assert!(response.body.is_empty());
        assert!(response.headers.is_empty());
    }

    #[test]
    fn a_header_block_that_does_not_decode_is_a_bad_argument() {
        // Arrange
        let engine = engine();
        let (mut instance, _) = hosted(&engine, GUEST, RecordingStream::new());
        let (pairs, _) = write(&mut instance, HEADERS, &[1, 0, 0, 0, 9, 0, 0, 0]);

        // Act
        let result = outcome(proxy_send_local_response(
            instance.store_mut(),
            200,
            0,
            0,
            0,
            0,
            pairs,
            8,
            -1,
        ));

        // Assert
        assert_eq!(result, Status::BadArgument);
        assert!(
            RecordingStream::take(instance.state_mut())
                .local_response()
                .is_none()
        );
    }

    #[test]
    fn the_default_body_reports_unimplemented() {
        // Arrange
        let engine = engine();
        let (mut instance, _) = unhosted(&engine, GUEST);
        instance
            .state_mut()
            .abi_mut()
            .set_stream_host(Box::new(NoStream));

        // Act
        let result = outcome(proxy_send_local_response(
            instance.store_mut(),
            200,
            0,
            0,
            0,
            0,
            0,
            0,
            -1,
        ));

        // Assert
        assert_eq!(result, Status::Unimplemented);
    }

    #[test]
    fn no_effective_context_reports_unimplemented() {
        // Arrange
        let engine = engine();
        let mut instance = bare(&engine, GUEST);

        // Act
        let result = outcome(proxy_send_local_response(
            instance.store_mut(),
            200,
            0,
            0,
            0,
            0,
            0,
            0,
            -1,
        ));

        // Assert
        assert_eq!(result, Status::Unimplemented);
    }

    #[test]
    fn every_data_pointer_is_checked_before_the_embedder_is_asked() {
        // Arrange
        let engine = engine();
        let (mut instance, _) = hosted(&engine, GUEST, RecordingStream::new());

        // Act
        let results = [
            outcome(proxy_send_local_response(
                instance.store_mut(),
                200,
                PAST_END,
                4,
                0,
                0,
                0,
                0,
                -1,
            )),
            outcome(proxy_send_local_response(
                instance.store_mut(),
                200,
                0,
                0,
                PAST_END,
                4,
                0,
                0,
                -1,
            )),
            outcome(proxy_send_local_response(
                instance.store_mut(),
                200,
                0,
                0,
                0,
                0,
                PAST_END,
                4,
                -1,
            )),
        ];

        // Assert
        assert_eq!(results, [Status::InvalidMemoryAccess; 3]);
        assert!(
            RecordingStream::take(instance.state_mut())
                .local_response()
                .is_none()
        );
    }
}
