//! The five gRPC callout functions.
//!
//! The crate delivers no gRPC callback, so a gRPC callout could never end in
//! the guest.
//! Each function therefore refuses, with a status that every guest SDK
//! accepts from it.

use wasmtime::AsContextMut;

use crate::abi::v0_2_1::host_functions::Failure;
use crate::abi::v0_2_1::types::Status;
use crate::runtime::HostState;

fn refuse(function: &'static str, status: Status) -> Result<(), Failure> {
    tracing::warn!(function, %status, "gRPC callouts are not served");
    Err(status.into())
}

#[expect(
    clippy::too_many_arguments,
    reason = "the ABI fixes the parameter list of this host function"
)]
pub(super) fn proxy_grpc_call(
    _: &mut impl AsContextMut<Data = HostState>,
    _upstream_name_data: i32,
    _upstream_name_size: i32,
    _service_name_data: i32,
    _service_name_size: i32,
    _method_name_data: i32,
    _method_name_size: i32,
    _serialized_initial_metadata_data: i32,
    _serialized_initial_metadata_size: i32,
    _message_data: i32,
    _message_size: i32,
    _timeout: i32,
    _return_call_id: i32,
) -> Result<(), Failure> {
    refuse("proxy_grpc_call", Status::InternalFailure)
}

#[expect(
    clippy::too_many_arguments,
    reason = "the ABI fixes the parameter list of this host function"
)]
pub(super) fn proxy_grpc_stream(
    _: &mut impl AsContextMut<Data = HostState>,
    _upstream_name_data: i32,
    _upstream_name_size: i32,
    _service_name_data: i32,
    _service_name_size: i32,
    _method_name_data: i32,
    _method_name_size: i32,
    _serialized_initial_metadata_data: i32,
    _serialized_initial_metadata_size: i32,
    _return_stream_id: i32,
) -> Result<(), Failure> {
    refuse("proxy_grpc_stream", Status::InternalFailure)
}

/// No gRPC stream can be open, so no identifier names one.
pub(super) fn proxy_grpc_send(
    _: &mut impl AsContextMut<Data = HostState>,
    _stream_id: i32,
    _message_data: i32,
    _message_size: i32,
    _end_stream: i32,
) -> Result<(), Failure> {
    refuse("proxy_grpc_send", Status::NotFound)
}

pub(super) fn proxy_grpc_cancel(
    _: &mut impl AsContextMut<Data = HostState>,
    _call_or_stream_id: i32,
) -> Result<(), Failure> {
    refuse("proxy_grpc_cancel", Status::NotFound)
}

pub(super) fn proxy_grpc_close(
    _: &mut impl AsContextMut<Data = HostState>,
    _call_or_stream_id: i32,
) -> Result<(), Failure> {
    refuse("proxy_grpc_close", Status::NotFound)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::v0_2_1::test_support::{MINIMAL_GUEST, engine, outcome, unhosted};

    #[test]
    fn the_two_functions_that_start_a_grpc_callout_refuse_as_an_internal_failure() {
        // Arrange
        let engine = engine();
        let (mut instance, _) = unhosted(&engine, MINIMAL_GUEST);

        // Act
        let answers = [
            outcome(proxy_grpc_call(
                instance.store_mut(),
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
            )),
            outcome(proxy_grpc_stream(
                instance.store_mut(),
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
            )),
        ];

        // Assert
        assert_eq!(answers, [Status::InternalFailure; 2]);
        assert!(!instance.is_poisoned());
    }

    #[test]
    fn send_cancel_and_close_find_no_grpc_callout_for_any_identifier() {
        // Arrange
        let engine = engine();
        let (mut instance, _) = unhosted(&engine, MINIMAL_GUEST);
        let identifiers = [0, 1, -1];

        // Act
        let answers: Vec<[Status; 3]> = identifiers
            .iter()
            .map(|&id| {
                [
                    outcome(proxy_grpc_send(instance.store_mut(), id, 0, 0, 1)),
                    outcome(proxy_grpc_cancel(instance.store_mut(), id)),
                    outcome(proxy_grpc_close(instance.store_mut(), id)),
                ]
            })
            .collect();

        // Assert
        assert_eq!(answers, [[Status::NotFound; 3]; 3]);
    }
}
