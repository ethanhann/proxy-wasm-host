//! `proxy_get_status`.
//!
//! The status belongs to the callout that the running callback delivers, so
//! the stream state answers it.

use wasmtime::AsContextMut;

use crate::abi::v0_2_1::host_functions::Failure;
use crate::abi::v0_2_1::host_functions::call::{from_embedder, with_stream};
use crate::abi::v0_2_1::types::Status;
use crate::runtime::{GuestPtr, HostState, split, write_return};

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
    let (call, stream) = with_stream(state, Status::Unimplemented)?;
    let status = from_embedder("callout_status", stream.callout_status(call))?;
    let code = status.code;
    let message = status.message.into_owned();
    write_return(ctx, &message, data_ptr, size_ptr)?;
    let (mut memory, _) = split(ctx)?;
    memory.write_u32(code_ptr, code)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::v0_2_1::AbiAccess;
    use crate::abi::v0_2_1::test_support::{RecordingStream, bare, hosted, outcome, status};
    use crate::abi::v0_2_1::{Callback, NoStream};
    use crate::runtime::test_support::engine;
    use crate::runtime::{GuestSlice, Instance};

    const CODE: i32 = 2000;
    const DATA: i32 = 2004;
    const SIZE: i32 = 2008;
    const SENTINEL: u32 = 0x7f7f_7f7f;
    const PAST_END: i32 = 65_534;

    const GUEST: &str = r#"(module
        (import "env" "proxy_get_status" (func $status (param i32 i32 i32) (result i32)))
        (memory (export "memory") 1)
        (func (export "proxy_on_memory_allocate") (param i32) (result i32)
            (i32.store8 (i32.const 0) (i32.add (i32.load8_u (i32.const 0)) (i32.const 1)))
            i32.const 4096)
        (func (export "status") (param i32 i32 i32) (result i32)
            local.get 0 local.get 1 local.get 2 call $status))"#;

    fn return_slots(instance: &mut Instance) -> (u32, u32, u32) {
        let memory = instance.memory().unwrap();
        (
            memory.read_u32(GuestPtr::from_address(2000)).unwrap(),
            memory.read_u32(GuestPtr::from_address(2004)).unwrap(),
            memory.read_u32(GuestPtr::from_address(2008)).unwrap(),
        )
    }

    fn seed_return_slots(instance: &mut Instance) {
        let mut memory = instance.memory().unwrap();
        for address in [2000, 2004, 2008] {
            memory
                .write_u32(GuestPtr::from_address(address), SENTINEL)
                .unwrap();
        }
    }

    fn allocator_calls(instance: &mut Instance) -> u8 {
        let slice = GuestSlice::try_from((0, 1)).unwrap();
        instance.memory().unwrap().read(slice).unwrap()[0]
    }

    #[test]
    fn the_code_and_the_message_of_the_callout_are_written() {
        // Arrange
        let engine = engine();
        let stream = RecordingStream::new().with_callout_status(503, b"unavailable");
        let (mut instance, root) = hosted(&engine, GUEST, stream);

        // Act
        let result = instance
            .call::<(i32, i32, i32), i32>("status", (CODE, DATA, SIZE))
            .map(status);

        // Assert
        assert_eq!(result.unwrap(), Status::Ok);
        let (code, data, size) = return_slots(&mut instance);
        assert_eq!((code, size), (503, 11));
        let slice = GuestSlice::new(GuestPtr::from_address(data), size).unwrap();
        assert_eq!(
            instance.memory().unwrap().read(slice).unwrap(),
            b"unavailable"
        );
        let stream = RecordingStream::take(instance.state_mut());
        assert_eq!(stream.callout_calls()[0].context, root);
        assert_eq!(
            stream.callout_calls()[0].callback,
            Some(Callback::RequestHeaders)
        );
    }

    #[test]
    fn an_empty_message_is_returned_as_two_zeros() {
        // Arrange
        let engine = engine();
        let stream = RecordingStream::new().with_callout_status(200, b"");
        let (mut instance, _) = hosted(&engine, GUEST, stream);
        seed_return_slots(&mut instance);

        // Act
        let result = instance
            .call::<(i32, i32, i32), i32>("status", (CODE, DATA, SIZE))
            .map(status);

        // Assert
        assert_eq!(result.unwrap(), Status::Ok);
        assert_eq!(return_slots(&mut instance), (200, 0, 0));
        assert_eq!(allocator_calls(&mut instance), 0);
    }

    #[test]
    fn the_default_body_reports_unimplemented() {
        // Arrange
        let engine = engine();
        let mut instance = bare(&engine, GUEST);
        instance
            .state_mut()
            .abi_mut()
            .set_stream_state(Box::new(NoStream));
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

        // Act
        let result = outcome(proxy_get_status(instance.store_mut(), CODE, DATA, SIZE));

        // Assert
        assert_eq!(result, Status::Unimplemented);
    }

    #[test]
    fn a_stream_with_no_callout_and_no_effective_context_both_report_unimplemented() {
        // Arrange
        let engine = engine();
        let (mut with_stream, _) = hosted(&engine, GUEST, RecordingStream::new());
        let mut without_context = bare(&engine, GUEST);

        // Act
        let results = [
            outcome(proxy_get_status(with_stream.store_mut(), CODE, DATA, SIZE)),
            outcome(proxy_get_status(
                without_context.store_mut(),
                CODE,
                DATA,
                SIZE,
            )),
        ];

        // Assert
        assert_eq!(results, [Status::Unimplemented; 2]);
    }

    #[test]
    fn every_return_pointer_is_checked_before_the_embedder_is_asked() {
        // Arrange
        let engine = engine();
        let stream = RecordingStream::new().with_callout_status(503, b"unavailable");
        let (mut instance, _) = hosted(&engine, GUEST, stream);
        seed_return_slots(&mut instance);

        // Act
        let results = [
            outcome(proxy_get_status(instance.store_mut(), PAST_END, DATA, SIZE)),
            outcome(proxy_get_status(instance.store_mut(), CODE, PAST_END, SIZE)),
            outcome(proxy_get_status(instance.store_mut(), CODE, DATA, PAST_END)),
        ];

        // Assert
        assert_eq!(results, [Status::InvalidMemoryAccess; 3]);
        assert_eq!(return_slots(&mut instance), (SENTINEL, SENTINEL, SENTINEL));
        assert_eq!(allocator_calls(&mut instance), 0);
        assert!(
            RecordingStream::take(instance.state_mut())
                .callout_calls()
                .is_empty()
        );
    }
}
