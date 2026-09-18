//! `proxy_continue_stream` and `proxy_close_stream`.

use wasmtime::AsContextMut;

use crate::abi::v0_2_1::host_functions::Failure;
use crate::abi::v0_2_1::host_functions::call::{from_embedder, with_stream};
use crate::abi::v0_2_1::types::{Status, StreamType};
use crate::runtime::HostState;

pub(super) fn proxy_continue_stream(
    ctx: &mut impl AsContextMut<Data = HostState>,
    stream_type: i32,
) -> Result<(), Failure> {
    let stream_type = StreamType::try_from(stream_type)?;
    let mut ctx = ctx.as_context_mut();
    let state = ctx.data_mut();
    let (call, stream) = with_stream(state, Status::Unimplemented)?;
    from_embedder("continue_stream", stream.continue_stream(call, stream_type))
}

pub(super) fn proxy_close_stream(
    ctx: &mut impl AsContextMut<Data = HostState>,
    stream_type: i32,
) -> Result<(), Failure> {
    let stream_type = StreamType::try_from(stream_type)?;
    let mut ctx = ctx.as_context_mut();
    let state = ctx.data_mut();
    let (call, stream) = with_stream(state, Status::Unimplemented)?;
    from_embedder("close_stream", stream.close_stream(call, stream_type))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::v0_2_1::AbiAccess;
    use crate::abi::v0_2_1::test_support::stream::Operation;
    use crate::abi::v0_2_1::test_support::{
        RecordingStream, bare, hosted, outcome, status, unhosted,
    };
    use crate::abi::v0_2_1::{Callback, NoStream};
    use crate::runtime::Instance;
    use crate::runtime::test_support::engine;

    const GUEST: &str = r#"(module
        (import "env" "proxy_continue_stream" (func $resume (param i32) (result i32)))
        (import "env" "proxy_close_stream" (func $close (param i32) (result i32)))
        (memory (export "memory") 1)
        (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 1024)
        (func (export "resume") (param i32) (result i32) local.get 0 call $resume)
        (func (export "close") (param i32) (result i32) local.get 0 call $close))"#;

    fn drive(instance: &mut Instance, export: &str, stream_type: StreamType) -> Status {
        status(
            instance
                .call::<i32, i32>(export, i32::from(stream_type))
                .unwrap(),
        )
    }

    #[test]
    fn every_stream_type_reaches_the_embedder_as_a_resume() {
        // Arrange
        let engine = engine();
        let (mut instance, root) = hosted(&engine, GUEST, RecordingStream::new());

        // Act
        let results: Vec<Status> = StreamType::ALL
            .iter()
            .map(|kind| drive(&mut instance, "resume", *kind))
            .collect();

        // Assert
        assert_eq!(results, vec![Status::Ok; 4]);
        let stream = RecordingStream::take(instance.state_mut());
        let seen: Vec<Operation> = stream.operations().iter().map(|(_, op)| *op).collect();
        let expected: Vec<Operation> = StreamType::ALL
            .iter()
            .map(|kind| Operation::Continue(*kind))
            .collect();
        assert_eq!(seen, expected);
        assert_eq!(stream.operations()[0].0.context, root);
        assert_eq!(
            stream.operations()[0].0.callback,
            Some(Callback::RequestHeaders)
        );
    }

    #[test]
    fn every_stream_type_reaches_the_embedder_as_a_close() {
        // Arrange
        let engine = engine();
        let (mut instance, _) = hosted(&engine, GUEST, RecordingStream::new());

        // Act
        let results: Vec<Status> = StreamType::ALL
            .iter()
            .map(|kind| drive(&mut instance, "close", *kind))
            .collect();

        // Assert
        assert_eq!(results, vec![Status::Ok; 4]);
        let stream = RecordingStream::take(instance.state_mut());
        let seen: Vec<Operation> = stream.operations().iter().map(|(_, op)| *op).collect();
        let expected: Vec<Operation> = StreamType::ALL
            .iter()
            .map(|kind| Operation::Close(*kind))
            .collect();
        assert_eq!(seen, expected);
    }

    #[test]
    fn an_unknown_stream_type_is_a_bad_argument() {
        // Arrange
        let engine = engine();
        let (mut instance, _) = hosted(&engine, GUEST, RecordingStream::new());

        // Act
        let results = [
            status(instance.call::<i32, i32>("resume", 4).unwrap()),
            status(instance.call::<i32, i32>("close", 4).unwrap()),
        ];

        // Assert
        assert_eq!(results, [Status::BadArgument; 2]);
        assert!(
            RecordingStream::take(instance.state_mut())
                .operations()
                .is_empty()
        );
    }

    #[test]
    fn a_success_for_a_stream_that_was_never_touched_is_an_internal_failure() {
        // Arrange
        let engine = engine();
        let stream = RecordingStream::new().refusing_with_ok();
        let (mut instance, _) = hosted(&engine, GUEST, stream);

        // Act
        let result = drive(&mut instance, "resume", StreamType::HttpRequest);

        // Assert
        assert_eq!(result, Status::InternalFailure);
    }

    #[test]
    fn no_stream_state_and_no_effective_context_both_report_unimplemented() {
        // Arrange
        let engine = engine();
        let (mut without_stream, _) = unhosted(&engine, GUEST);
        let mut without_context = bare(&engine, GUEST);

        // Act
        let results = [
            outcome(proxy_continue_stream(without_stream.store_mut(), 0)),
            outcome(proxy_close_stream(without_stream.store_mut(), 0)),
            outcome(proxy_continue_stream(without_context.store_mut(), 0)),
            outcome(proxy_close_stream(without_context.store_mut(), 0)),
        ];

        // Assert
        assert_eq!(results, [Status::Unimplemented; 4]);
    }

    #[test]
    fn the_default_bodies_report_unimplemented() {
        // Arrange
        let engine = engine();
        let (mut instance, _) = unhosted(&engine, GUEST);
        instance
            .state_mut()
            .abi_mut()
            .set_stream_state(Box::new(NoStream));

        // Act
        let results = [
            outcome(proxy_continue_stream(instance.store_mut(), 0)),
            outcome(proxy_close_stream(instance.store_mut(), 0)),
        ];

        // Assert
        assert_eq!(results, [Status::Unimplemented; 2]);
    }

    #[test]
    fn the_status_a_stream_returns_passes_through() {
        // Arrange
        let engine = engine();
        let stream = RecordingStream::new().refusing_operation(Status::BadArgument);
        let (mut instance, _) = hosted(&engine, GUEST, stream);

        // Act
        let results = [
            outcome(proxy_continue_stream(instance.store_mut(), 0)),
            outcome(proxy_close_stream(instance.store_mut(), 0)),
        ];

        // Assert
        assert_eq!(results, [Status::BadArgument; 2]);
        assert_eq!(
            RecordingStream::take(instance.state_mut())
                .operations()
                .len(),
            2
        );
    }
}
