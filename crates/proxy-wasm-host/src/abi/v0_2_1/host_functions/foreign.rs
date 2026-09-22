//! `proxy_call_foreign_function`.
//!
//! This is the one host function whose return pointers are optional, which
//! the ABI document states.
//! A guest that passes the address zero for one of them asks the host not to
//! write that value.

use std::borrow::Cow;

use wasmtime::AsContextMut;

use crate::abi::v0_2_1::ForeignCall;
use crate::abi::v0_2_1::host_functions::Failure;
use crate::abi::v0_2_1::host_functions::call::{from_embedder, with_stream};
use crate::abi::v0_2_1::types::Status;
use crate::runtime::{GuestPtr, GuestSlice, HostState, split, write_optional_return};

pub(super) fn proxy_call_foreign_function(
    ctx: &mut impl AsContextMut<Data = HostState>,
    name_data: i32,
    name_size: i32,
    arguments_data: i32,
    arguments_size: i32,
    return_results_data: i32,
    return_results_size: i32,
) -> Result<(), Failure> {
    let name = GuestSlice::try_from((name_data, name_size))?;
    let arguments = GuestSlice::try_from((arguments_data, arguments_size))?;
    let data_ptr = wanted(return_results_data)?;
    let size_ptr = wanted(return_results_size)?;
    let (memory, state) = split(ctx)?;
    if let Some(pointer) = data_ptr {
        memory.read_u32(pointer)?;
    }
    if let Some(pointer) = size_ptr {
        memory.read_u32(pointer)?;
    }
    let name = memory.read(name)?;
    let arguments = memory.read(arguments)?;
    let request = ForeignCall::new(Cow::Borrowed(name), Cow::Borrowed(arguments));
    let (call, stream) = with_stream(state, Status::NotFound)?;
    let results = from_embedder(
        "call_foreign_function",
        stream.call_foreign_function(call, request),
    )?;
    write_optional_return(ctx, &results, data_ptr, size_ptr)?;
    Ok(())
}

/// The return pointer the guest gave, or `None` when it wants no value.
///
/// The ABI document calls the return values of this function optional, and
/// the address zero is how a guest says so.
fn wanted(pointer: i32) -> Result<Option<GuestPtr>, Failure> {
    if pointer == 0 {
        return Ok(None);
    }
    Ok(Some(GuestPtr::try_from(pointer)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::v0_2_1::AbiAccess;
    use crate::abi::v0_2_1::test_support::{
        RecordingStream, bare, engine, hosted, outcome, returned, status, unhosted, write,
    };
    use crate::runtime::Instance;

    const NAME: i32 = 1024;
    const ARGUMENTS: i32 = 1100;
    const RETURN_DATA: i32 = 2000;
    const RETURN_SIZE: i32 = 2004;
    const PAST_END: i32 = 65_534;

    /// The same guest, with an allocator that counts its calls at address 8.
    const COUNTING: &str = r#"(module
        (import "env" "proxy_call_foreign_function"
            (func $call (param i32 i32 i32 i32 i32 i32) (result i32)))
        (memory (export "memory") 1)
        (func (export "proxy_on_memory_allocate") (param i32) (result i32)
            (i32.store8 (i32.const 8) (i32.add (i32.load8_u (i32.const 8)) (i32.const 1)))
            i32.const 4096)
        (func (export "call") (param i32 i32 i32 i32 i32 i32) (result i32)
            local.get 0 local.get 1 local.get 2 local.get 3 local.get 4 local.get 5 call $call))"#;

    const GUEST: &str = r#"(module
        (import "env" "proxy_call_foreign_function"
            (func $call (param i32 i32 i32 i32 i32 i32) (result i32)))
        (memory (export "memory") 1)
        (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 4096)
        (func (export "call") (param i32 i32 i32 i32 i32 i32) (result i32)
            local.get 0 local.get 1 local.get 2 local.get 3 local.get 4 local.get 5 call $call))"#;

    fn call_foreign(instance: &mut Instance, name: &[u8], arguments: &[u8]) -> Status {
        let (_, name_len) = write(instance, NAME, name);
        let (_, argument_len) = write(instance, ARGUMENTS, arguments);
        status(
            instance
                .call::<(i32, i32, i32, i32, i32, i32), i32>(
                    "call",
                    (
                        NAME,
                        name_len,
                        ARGUMENTS,
                        argument_len,
                        RETURN_DATA,
                        RETURN_SIZE,
                    ),
                )
                .unwrap(),
        )
    }

    /// The `u32` in guest memory at `at`.
    fn word(instance: &mut Instance, at: u32) -> u32 {
        instance
            .memory()
            .unwrap()
            .read_u32(crate::runtime::GuestPtr::from_address(at))
            .unwrap()
    }

    /// Calls the foreign function with the two return pointers given.
    fn call_returning(instance: &mut Instance, data_ptr: i32, size_ptr: i32) -> Status {
        let (_, name_len) = write(instance, NAME, b"compress");
        let (_, argument_len) = write(instance, ARGUMENTS, b"payload");
        status(
            instance
                .call::<(i32, i32, i32, i32, i32, i32), i32>(
                    "call",
                    (NAME, name_len, ARGUMENTS, argument_len, data_ptr, size_ptr),
                )
                .unwrap(),
        )
    }

    #[test]
    fn a_foreign_call_with_no_data_pointer_allocates_nothing_in_the_guest() {
        // Arrange
        let engine = engine();
        let stream = RecordingStream::new().with_foreign_function(b"compress", b"done");
        let (mut instance, _) = hosted(&engine, COUNTING, stream);

        // Act
        let result = call_returning(&mut instance, 0, RETURN_SIZE);

        // Assert
        assert_eq!(result, Status::Ok);
        assert_eq!(word(&mut instance, 8), 0, "the allocator ran");
        assert_eq!(word(&mut instance, 0), 0, "the first word was written");
    }

    #[test]
    fn a_foreign_call_with_no_data_pointer_still_runs_the_embedder() {
        // Arrange
        let engine = engine();
        let stream = RecordingStream::new().with_foreign_function(b"compress", b"done");
        let (mut instance, root) = hosted(&engine, GUEST, stream);

        // Act
        let result = call_returning(&mut instance, 0, RETURN_SIZE);

        // Assert
        assert_eq!(result, Status::Ok);
        let stream = RecordingStream::take(instance.state_mut());
        let (call, request) = &stream.foreign_calls()[0];
        assert_eq!(call.context, root);
        assert_eq!(request.name.as_ref(), b"compress");
        assert_eq!(request.arguments.as_ref(), b"payload");
    }

    #[test]
    fn a_foreign_call_with_no_data_pointer_still_writes_the_size() {
        // Arrange
        let engine = engine();
        let stream = RecordingStream::new().with_foreign_function(b"compress", b"done");
        let (mut instance, _) = hosted(&engine, GUEST, stream);

        // Act
        let result = call_returning(&mut instance, 0, RETURN_SIZE);

        // Assert
        assert_eq!(result, Status::Ok);
        assert_eq!(word(&mut instance, RETURN_SIZE.cast_unsigned()), 4);
        assert_eq!(word(&mut instance, RETURN_DATA.cast_unsigned()), 0);
    }

    #[test]
    fn a_foreign_call_with_no_size_pointer_writes_the_address_alone() {
        // Arrange
        let engine = engine();
        let stream = RecordingStream::new().with_foreign_function(b"compress", b"done");
        let (mut instance, _) = hosted(&engine, COUNTING, stream);

        // Act
        let result = call_returning(&mut instance, RETURN_DATA, 0);

        // Assert
        assert_eq!(result, Status::Ok);
        assert_eq!(word(&mut instance, 8), 1, "the allocator did not run once");
        assert_eq!(word(&mut instance, RETURN_DATA.cast_unsigned()), 4096);
        assert_eq!(word(&mut instance, 0), 0, "the first word was written");
    }

    #[test]
    fn a_foreign_call_with_both_pointers_writes_both() {
        // Arrange
        let engine = engine();
        let stream = RecordingStream::new().with_foreign_function(b"compress", b"done");
        let (mut instance, _) = hosted(&engine, GUEST, stream);

        // Act
        let result = call_returning(&mut instance, RETURN_DATA, RETURN_SIZE);

        // Assert
        assert_eq!(result, Status::Ok);
        assert_eq!(word(&mut instance, RETURN_DATA.cast_unsigned()), 4096);
        assert_eq!(word(&mut instance, RETURN_SIZE.cast_unsigned()), 4);
        assert_eq!(returned(&mut instance, 2000, 2004), b"done");
    }

    #[test]
    fn the_name_and_the_arguments_reach_the_stream_state() {
        // Arrange
        let engine = engine();
        let stream = RecordingStream::new().with_foreign_function(b"compress", b"done");
        let (mut instance, root) = hosted(&engine, GUEST, stream);

        // Act
        let result = call_foreign(&mut instance, b"compress", b"payload");

        // Assert
        assert_eq!(result, Status::Ok);
        assert_eq!(returned(&mut instance, 2000, 2004), b"done");
        let stream = RecordingStream::take(instance.state_mut());
        let (call, request) = &stream.foreign_calls()[0];
        assert_eq!(call.context, root);
        assert_eq!(request.name.as_ref(), b"compress");
        assert_eq!(request.arguments.as_ref(), b"payload");
    }

    #[test]
    fn an_empty_result_is_returned_as_two_zeros() {
        // Arrange
        let engine = engine();
        let stream = RecordingStream::new().with_foreign_function(b"ping", b"");
        let (mut instance, _) = hosted(&engine, GUEST, stream);

        // Act
        let result = call_foreign(&mut instance, b"ping", b"");

        // Assert
        assert_eq!(result, Status::Ok);
        assert_eq!(returned(&mut instance, 2000, 2004), b"");
    }

    #[test]
    fn a_name_the_stream_state_does_not_serve_is_not_found() {
        // Arrange
        let engine = engine();
        let (mut instance, _) = hosted(&engine, GUEST, RecordingStream::new());

        // Act
        let result = call_foreign(&mut instance, b"missing", b"");

        // Assert
        assert_eq!(result, Status::NotFound);
    }

    #[test]
    fn a_body_with_no_stream_state_or_no_context_is_not_found() {
        // Arrange
        let engine = engine();
        let (mut without_stream, _) = unhosted(&engine, GUEST);
        let mut without_context = bare(&engine, GUEST);

        // Act
        let results = [
            call_foreign(&mut without_stream, b"compress", b""),
            call_foreign(&mut without_context, b"compress", b""),
        ];

        // Assert
        assert_eq!(results, [Status::NotFound; 2]);
    }

    #[test]
    fn every_pointer_is_checked_before_the_stream_state_is_asked() {
        // Arrange
        let engine = engine();
        let (mut instance, _) = hosted(&engine, GUEST, RecordingStream::new());

        // Act
        let results = [
            outcome(proxy_call_foreign_function(
                instance.store_mut(),
                PAST_END,
                4,
                ARGUMENTS,
                1,
                RETURN_DATA,
                RETURN_SIZE,
            )),
            outcome(proxy_call_foreign_function(
                instance.store_mut(),
                NAME,
                1,
                PAST_END,
                4,
                RETURN_DATA,
                RETURN_SIZE,
            )),
            outcome(proxy_call_foreign_function(
                instance.store_mut(),
                NAME,
                1,
                ARGUMENTS,
                1,
                PAST_END,
                RETURN_SIZE,
            )),
            outcome(proxy_call_foreign_function(
                instance.store_mut(),
                NAME,
                1,
                ARGUMENTS,
                1,
                RETURN_DATA,
                PAST_END,
            )),
        ];

        // Assert
        assert_eq!(results, [Status::InvalidMemoryAccess; 4]);
        assert!(
            RecordingStream::take(instance.state_mut())
                .foreign_calls()
                .is_empty()
        );
    }

    #[test]
    fn a_body_under_a_refused_root_is_not_found() {
        // Arrange
        let engine = engine();
        let (mut instance, root) = hosted(&engine, GUEST, RecordingStream::new());
        instance.state_mut().abi_mut().contexts_mut().reject(root);

        // Act
        let result = call_foreign(&mut instance, b"compress", b"");

        // Assert
        assert_eq!(result, Status::NotFound);
    }
}
