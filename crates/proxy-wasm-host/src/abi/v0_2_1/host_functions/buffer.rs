//! The three buffer functions.
//!
//! The crate serves `VM_CONFIGURATION` and `PLUGIN_CONFIGURATION` from the
//! values the embedder gave it, and the three buffers of a delivery from the
//! value that delivery holds.
//! It asks the stream state for every other buffer.
//! Each body resolves the buffer type before the context, so a configuration
//! read never needs a callback to be running.

use wasmtime::AsContextMut;

use crate::abi::v0_2_1::AbiAccess;
use crate::abi::v0_2_1::Access;
use crate::abi::v0_2_1::host_functions::Failure;
use crate::abi::v0_2_1::host_functions::Served;
use crate::abi::v0_2_1::host_functions::call::{context, from_embedder, with_stream};
use crate::abi::v0_2_1::payload::{DeliveredBuffer, serves_buffer};
use crate::abi::v0_2_1::types::{BufferType, Status};
use crate::buffer::clamp_range;
use crate::runtime::{GuestPtr, GuestSlice, HostState, split, write_return};
use crate::{Buffer, NotAllowed};

/// Where the bytes of one buffer come from.
enum Source<'a> {
    Crate(&'a [u8]),
    Stream(&'a mut dyn Buffer),
}

impl Source<'_> {
    fn len(&self) -> usize {
        match self {
            Self::Crate(bytes) => bytes.len(),
            Self::Stream(buffer) => buffer.len(),
        }
    }

    /// The bytes of the clamped range, so an embedder never receives a range
    /// that runs past its buffer.
    fn copy_range(&self, start: usize, size: usize) -> Vec<u8> {
        let range = clamp_range(self.len(), start, size);
        match self {
            Self::Crate(bytes) => bytes[range].to_vec(),
            Self::Stream(buffer) => {
                let mut bytes = Vec::with_capacity(range.len());
                buffer.copy_range_into(range.start, range.len(), &mut bytes);
                bytes
            }
        }
    }
}

/// The status for a buffer write the embedder refused.
///
/// The buffers section of the ABI lists `NOT_FOUND` for a buffer that is not
/// available, and a buffer the embedder will not change is not available for
/// that write.
/// The conversion that `?` would reach answers `BAD_ARGUMENT`, which is the
/// status the HTTP fields section lists for a map, so a buffer write goes
/// through this function instead.
fn refused(_: NotAllowed) -> Status {
    Status::NotFound
}

/// A `size_t` argument, which arrives negative when it is above `i32::MAX`.
fn as_usize(value: i32) -> usize {
    usize::try_from(value.cast_unsigned()).unwrap_or(usize::MAX)
}

/// A buffer the crate answers from what the embedder supplied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CrateBuffer {
    VmConfiguration,
    PluginConfiguration,
    /// A buffer of the value the embedder delivered for the running
    /// callback.
    Delivered(DeliveredBuffer),
}

/// Whether the crate serves this buffer itself.
///
/// The configuration buffers come from what the embedder supplied before the
/// call.
/// The body of an HTTP call response, the message of a gRPC callout, and the
/// arguments of a foreign function call come from the value the embedder
/// delivered for the running callback, and [`serves_buffer`] holds that list
/// for every family.
/// No implementation of the embedder sees a request for one of them.
///
/// See [`Served`] for the rule.
fn served(buffer_type: BufferType) -> Served<CrateBuffer> {
    match buffer_type {
        BufferType::VmConfiguration => Served::Crate(CrateBuffer::VmConfiguration),
        BufferType::PluginConfiguration => Served::Crate(CrateBuffer::PluginConfiguration),
        _ => match serves_buffer(buffer_type) {
            Some(which) => Served::Crate(CrateBuffer::Delivered(which)),
            None => Served::Embedder,
        },
    }
}

fn read_buffer(state: &mut HostState, buffer_type: BufferType) -> Result<Source<'_>, Failure> {
    match served(buffer_type) {
        Served::Crate(CrateBuffer::VmConfiguration) => {
            Ok(Source::Crate(state.abi().services().vm_configuration()))
        }
        Served::Crate(CrateBuffer::PluginConfiguration) => {
            let root = context(state, Status::NotFound)?;
            let plugin = state
                .abi()
                .contexts()
                .plugin(root)
                .ok_or(Status::NotFound)?;
            Ok(Source::Crate(plugin.configuration()))
        }
        Served::Crate(CrateBuffer::Delivered(which)) => {
            match state
                .abi()
                .delivery()
                .and_then(|delivery| delivery.buffer(which))
            {
                Some(bytes) => Ok(Source::Crate(bytes)),
                None => Err(Status::NotFound.into()),
            }
        }
        Served::Embedder => {
            let announced = state.abi_mut().take_announced(buffer_type);
            let (call, stream) = with_stream(state, Status::NotFound)?;
            let buffer = from_embedder("buffer", stream.buffer(call, Access::Read, buffer_type))?;
            report_announced(buffer_type, announced, buffer.len());
            Ok(Source::Stream(buffer))
        }
    }
}

/// Reports a length that differs from the size the running callback
/// announced.
///
/// The guest reads what the stream state holds, so a size that is too large
/// gives the guest a short read with an `OK` status and a size that is too
/// small hides bytes.
/// Neither answer says what the mistake is, so the crate says it here.
/// The caller takes the record, so one callback reports at most one
/// difference.
fn report_announced(buffer_type: BufferType, announced: Option<u32>, length: usize) {
    let Some(announced) = announced else {
        return;
    };
    if u64::from(announced) != length.try_into().unwrap_or(u64::MAX) {
        tracing::warn!(
            buffer = ?buffer_type,
            announced,
            length,
            "the callback announced a size the stream state does not hold"
        );
    }
}

fn write_buffer(
    state: &mut HostState,
    buffer_type: BufferType,
) -> Result<&mut dyn Buffer, Failure> {
    match served(buffer_type) {
        Served::Crate(_) => Err(Status::NotFound.into()),
        Served::Embedder => {
            let (call, stream) = with_stream(state, Status::NotFound)?;
            from_embedder("buffer", stream.buffer(call, Access::Write, buffer_type))
        }
    }
}

pub(super) fn proxy_get_buffer_bytes(
    ctx: &mut impl AsContextMut<Data = HostState>,
    buffer_id: i32,
    start: i32,
    max_size: i32,
    return_value_data: i32,
    return_value_size: i32,
) -> Result<(), Failure> {
    let buffer_type = BufferType::try_from(buffer_id)?;
    let start = as_usize(start);
    let max_size = as_usize(max_size);
    let data_ptr = GuestPtr::try_from(return_value_data)?;
    let size_ptr = GuestPtr::try_from(return_value_size)?;
    let (memory, state) = split(ctx)?;
    memory.read_u32(data_ptr)?;
    memory.read_u32(size_ptr)?;
    let source = read_buffer(state, buffer_type)?;
    if start > source.len() {
        return Err(Status::BadArgument.into());
    }
    let bytes = source.copy_range(start, max_size);
    write_return(ctx, &bytes, data_ptr, size_ptr)?;
    Ok(())
}

pub(super) fn proxy_set_buffer_bytes(
    ctx: &mut impl AsContextMut<Data = HostState>,
    buffer_id: i32,
    start: i32,
    size: i32,
    value_data: i32,
    value_size: i32,
) -> Result<(), Failure> {
    let buffer_type = BufferType::try_from(buffer_id)?;
    let start = as_usize(start);
    let size = as_usize(size);
    let value = GuestSlice::try_from((value_data, value_size))?;
    let (memory, state) = split(ctx)?;
    let value = memory.read(value)?;
    let buffer = write_buffer(state, buffer_type)?;
    let range = clamp_range(buffer.len(), start, size);
    buffer
        .replace(range.start, range.len(), value)
        .map_err(refused)?;
    Ok(())
}

/// Reports the size of a buffer.
///
/// The ABI names the third parameter `return_unused` and defines no meaning
/// for it, so the body writes zero there.
pub(super) fn proxy_get_buffer_status(
    ctx: &mut impl AsContextMut<Data = HostState>,
    buffer_id: i32,
    return_buffer_size: i32,
    return_unused: i32,
) -> Result<(), Failure> {
    let buffer_type = BufferType::try_from(buffer_id)?;
    let size_ptr = GuestPtr::try_from(return_buffer_size)?;
    let unused_ptr = GuestPtr::try_from(return_unused)?;
    let (memory, state) = split(ctx)?;
    memory.read_u32(size_ptr)?;
    memory.read_u32(unused_ptr)?;
    let len = read_buffer(state, buffer_type)?.len();
    let len = u32::try_from(len).map_err(|_| Status::InternalFailure)?;
    let (mut memory, _) = split(ctx)?;
    memory.write_u32(size_ptr, len)?;
    memory.write_u32(unused_ptr, 0)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::abi::v0_2_1::payload::Delivery;
    use std::borrow::Cow;

    use super::*;
    use crate::abi::v0_2_1::VmServices;
    use crate::abi::v0_2_1::test_support::{
        RecordingSink, RecordingStream, bare, engine, hosted, instance_with, outcome, status,
        unhosted, wat_bytes, write,
    };
    use crate::abi::v0_2_1::types::PeerType;
    use crate::abi::v0_2_1::{Access, CalloutId, ContextId, HttpCallResponse, PluginConfig};
    use crate::runtime::{GuestSlice, Instance, Module};

    const BODY: i32 = BufferType::HttpRequestBody as i32;
    const VM: i32 = BufferType::VmConfiguration as i32;
    const PLUGIN: i32 = BufferType::PluginConfiguration as i32;
    const VALUE: i32 = 1024;
    const RETURN_DATA: i32 = 2000;
    const RETURN_SIZE: i32 = 2004;
    const STATUS_SIZE: i32 = 2008;
    const STATUS_UNUSED: i32 = 2012;
    const SENTINEL: u32 = 0x7f7f_7f7f;
    const PAST_END: i32 = 65_534;

    const GUEST: &str = r#"(module
        (import "env" "proxy_get_buffer_bytes" (func $get (param i32 i32 i32 i32 i32) (result i32)))
        (import "env" "proxy_set_buffer_bytes" (func $set (param i32 i32 i32 i32 i32) (result i32)))
        (import "env" "proxy_get_buffer_status" (func $status (param i32 i32 i32) (result i32)))
        (memory (export "memory") 1)
        (func (export "proxy_on_memory_allocate") (param i32) (result i32)
            (i32.store8 (i32.const 0) (i32.add (i32.load8_u (i32.const 0)) (i32.const 1)))
            i32.const 4096)
        (func (export "get") (param i32 i32 i32 i32 i32) (result i32)
            local.get 0 local.get 1 local.get 2 local.get 3 local.get 4 call $get)
        (func (export "set") (param i32 i32 i32 i32 i32) (result i32)
            local.get 0 local.get 1 local.get 2 local.get 3 local.get 4 call $set)
        (func (export "status") (param i32 i32 i32) (result i32)
            local.get 0 local.get 1 local.get 2 call $status))"#;

    fn body_of(bytes: &[u8]) -> RecordingStream {
        RecordingStream::new().with_buffer(BufferType::HttpRequestBody, bytes)
    }

    fn get(instance: &mut Instance, buffer: i32, start: i32, max_size: i32) -> Status {
        status(
            instance
                .call::<(i32, i32, i32, i32, i32), i32>(
                    "get",
                    (buffer, start, max_size, RETURN_DATA, RETURN_SIZE),
                )
                .unwrap(),
        )
    }

    fn set(instance: &mut Instance, buffer: i32, start: i32, size: i32, value: &[u8]) -> Status {
        let (_, len) = write(instance, VALUE, value);
        status(
            instance
                .call::<(i32, i32, i32, i32, i32), i32>("set", (buffer, start, size, VALUE, len))
                .unwrap(),
        )
    }

    fn returned(instance: &mut Instance) -> Vec<u8> {
        let memory = instance.memory().unwrap();
        let data = memory.read_u32(GuestPtr::from_address(2000)).unwrap();
        let size = memory.read_u32(GuestPtr::from_address(2004)).unwrap();
        if size == 0 {
            return Vec::new();
        }
        let slice = GuestSlice::new(GuestPtr::from_address(data), size).unwrap();
        memory.read(slice).unwrap().to_vec()
    }

    fn return_slots(instance: &mut Instance) -> (u32, u32) {
        let memory = instance.memory().unwrap();
        (
            memory.read_u32(GuestPtr::from_address(2000)).unwrap(),
            memory.read_u32(GuestPtr::from_address(2004)).unwrap(),
        )
    }

    fn status_slots(instance: &mut Instance) -> (u32, u32) {
        let memory = instance.memory().unwrap();
        (
            memory.read_u32(GuestPtr::from_address(2008)).unwrap(),
            memory.read_u32(GuestPtr::from_address(2012)).unwrap(),
        )
    }

    fn seed(instance: &mut Instance, addresses: &[u32]) {
        let mut memory = instance.memory().unwrap();
        for address in addresses {
            memory
                .write_u32(GuestPtr::from_address(*address), SENTINEL)
                .unwrap();
        }
    }

    fn buffer_status(instance: &mut Instance, buffer: i32) -> Status {
        status(
            instance
                .call::<(i32, i32, i32), i32>("status", (buffer, STATUS_SIZE, STATUS_UNUSED))
                .unwrap(),
        )
    }

    fn allocator_calls(instance: &mut Instance) -> u8 {
        let slice = GuestSlice::try_from((0, 1)).unwrap();
        instance.memory().unwrap().read(slice).unwrap()[0]
    }

    fn configured(vm: &[u8], plugin: &[u8]) -> Instance {
        let engine = engine();
        let module = Module::new(&engine, &wat_bytes(GUEST)).unwrap();
        let services = VmServices::new(std::sync::Arc::new(RecordingSink::default()))
            .with_vm_configuration(vm.to_vec());
        let mut instance = instance_with(&engine, &module, services).unwrap();
        let state = instance.state_mut();
        let root = state.abi_mut().contexts_mut().create(None).unwrap();
        state.abi_mut().contexts_mut().set_plugin(
            root,
            PluginConfig::new().with_configuration(plugin.to_vec()),
        );
        state.abi_mut().contexts_mut().set_effective(root);
        instance
    }

    #[test]
    fn the_largest_max_size_returns_the_whole_buffer() {
        // Arrange
        let engine = engine();
        let (mut instance, _) = hosted(&engine, GUEST, body_of(b"0123456789"));

        // Act
        let result = get(&mut instance, BODY, 0, -1);

        // Assert
        assert_eq!(result, Status::Ok);
        assert_eq!(returned(&mut instance), b"0123456789");
    }

    #[test]
    fn a_range_past_the_end_is_cut_at_the_end() {
        // Arrange
        let engine = engine();
        let (mut instance, _) = hosted(&engine, GUEST, body_of(b"0123456789"));

        // Act
        let result = get(&mut instance, BODY, 7, 100);

        // Assert
        assert_eq!(result, Status::Ok);
        assert_eq!(returned(&mut instance), b"789");
    }

    #[test]
    fn an_empty_buffer_is_returned_without_an_allocation() {
        // Arrange
        let engine = engine();
        let (mut instance, _) = hosted(&engine, GUEST, body_of(b""));

        // Act
        let result = get(&mut instance, BODY, 0, -1);

        // Assert
        assert_eq!(result, Status::Ok);
        assert_eq!(return_slots(&mut instance), (0, 0));
        assert_eq!(allocator_calls(&mut instance), 0);
    }

    #[test]
    fn a_start_at_the_end_is_an_empty_answer() {
        // Arrange
        let engine = engine();
        let (mut instance, _) = hosted(&engine, GUEST, body_of(b"0123456789"));

        // Act
        let result = get(&mut instance, BODY, 10, 4);

        // Assert
        assert_eq!(result, Status::Ok);
        assert_eq!(return_slots(&mut instance), (0, 0));
    }

    #[test]
    fn a_start_past_the_end_and_an_unknown_buffer_are_bad_arguments() {
        // Arrange
        let engine = engine();
        let (mut instance, _) = hosted(&engine, GUEST, body_of(b"0123456789"));

        // Act
        let results = [get(&mut instance, BODY, 11, 1), get(&mut instance, 9, 0, 1)];

        // Assert
        assert_eq!(results, [Status::BadArgument; 2]);
    }

    #[test]
    fn the_status_a_stream_returns_passes_through() {
        // Arrange
        let engine = engine();
        let stream = RecordingStream::new()
            .with_buffer(BufferType::HttpRequestBody, b"body")
            .refusing_buffer(BufferType::HttpRequestBody, Status::BadArgument);
        let (mut instance, root) = hosted(&engine, GUEST, stream);

        // Act
        let result = get(&mut instance, BODY, 0, 1);

        // Assert
        assert_eq!(result, Status::BadArgument);
        let stream = RecordingStream::take(instance.state_mut());
        let (call, access, buffer) = stream.buffer_calls()[0];
        assert_eq!(buffer, BufferType::HttpRequestBody);
        assert_eq!((call.context, access), (root, Access::Read));
    }

    #[test]
    fn a_buffer_the_crate_cannot_reach_is_not_found() {
        // Arrange
        let engine = engine();
        let (mut unserved, _) = hosted(&engine, GUEST, RecordingStream::new());
        let (mut without_stream, _) = unhosted(&engine, GUEST);

        // Act
        let results = [
            get(&mut unserved, BODY, 0, 1),
            get(&mut without_stream, BODY, 0, 1),
        ];

        // Assert
        assert_eq!(results, [Status::NotFound; 2]);
    }

    #[test]
    fn the_vm_configuration_is_served_without_a_stream_and_without_a_context() {
        // Arrange
        let engine = engine();
        let module = Module::new(&engine, &wat_bytes(GUEST)).unwrap();
        let services = VmServices::new(std::sync::Arc::new(RecordingSink::default()))
            .with_vm_configuration(b"vm bytes".to_vec());
        let mut instance = instance_with(&engine, &module, services).unwrap();

        // Act
        let result = get(&mut instance, VM, 0, -1);

        // Assert
        assert_eq!(result, Status::Ok);
        assert_eq!(returned(&mut instance), b"vm bytes");
    }

    #[test]
    fn the_plugin_configuration_is_served_through_the_root_of_the_effective_context() {
        // Arrange
        let mut instance = configured(b"vm", b"plugin bytes");

        // Act
        let result = get(&mut instance, PLUGIN, 0, -1);

        // Assert
        assert_eq!(result, Status::Ok);
        assert_eq!(returned(&mut instance), b"plugin bytes");
    }

    #[test]
    fn the_plugin_configuration_of_a_refused_root_is_not_found() {
        // Arrange
        let mut instance = configured(b"vm", b"plugin bytes");
        let root = instance.state().abi().contexts().effective().unwrap();
        instance.state_mut().abi_mut().contexts_mut().reject(root);

        // Act
        let result = get(&mut instance, PLUGIN, 0, -1);

        // Assert
        assert_eq!(result, Status::NotFound);
    }

    #[test]
    fn a_root_with_no_plugin_has_no_plugin_configuration() {
        // Arrange
        let engine = engine();
        let (mut instance, _) = unhosted(&engine, GUEST);

        // Act
        let result = get(&mut instance, PLUGIN, 0, -1);

        // Assert
        assert_eq!(result, Status::NotFound);
    }

    #[test]
    fn the_plugin_configuration_without_an_effective_context_is_not_found() {
        // Arrange
        let engine = engine();
        let mut instance = bare(&engine, GUEST);

        // Act
        let result = get(&mut instance, PLUGIN, 0, -1);

        // Assert
        assert_eq!(result, Status::NotFound);
    }

    #[test]
    fn the_status_of_the_plugin_configuration_is_its_length() {
        // Arrange
        let mut instance = configured(b"vm", b"plugin bytes");
        seed(&mut instance, &[2008, 2012]);

        // Act
        let result = buffer_status(&mut instance, PLUGIN);

        // Assert
        assert_eq!(result, Status::Ok);
        assert_eq!(status_slots(&mut instance), (12, 0));
    }

    #[test]
    fn a_start_and_a_size_of_zero_prepend() {
        // Arrange
        let engine = engine();
        let (mut instance, _) = hosted(&engine, GUEST, body_of(b"body"));

        // Act
        let result = set(&mut instance, BODY, 0, 0, b"new ");

        // Assert
        assert_eq!(result, Status::Ok);
        assert_eq!(
            RecordingStream::bytes_in(instance.state_mut(), BufferType::HttpRequestBody),
            b"new body"
        );
    }

    #[test]
    fn the_largest_start_appends() {
        // Arrange
        let engine = engine();
        let (mut instance, _) = hosted(&engine, GUEST, body_of(b"body"));

        // Act
        let result = set(&mut instance, BODY, -1, 0, b" more");

        // Assert
        assert_eq!(result, Status::Ok);
        assert_eq!(
            RecordingStream::bytes_in(instance.state_mut(), BufferType::HttpRequestBody),
            b"body more"
        );
    }

    #[test]
    fn a_size_of_zero_inside_the_buffer_injects() {
        // Arrange
        let engine = engine();
        let (mut instance, _) = hosted(&engine, GUEST, body_of(b"body"));

        // Act
        let result = set(&mut instance, BODY, 2, 0, b"--");

        // Assert
        assert_eq!(result, Status::Ok);
        assert_eq!(
            RecordingStream::bytes_in(instance.state_mut(), BufferType::HttpRequestBody),
            b"bo--dy"
        );
    }

    #[test]
    fn a_range_inside_the_buffer_is_replaced() {
        // Arrange
        let engine = engine();
        let (mut instance, _) = hosted(&engine, GUEST, body_of(b"body"));

        // Act
        let result = set(&mut instance, BODY, 1, 2, b"ea");

        // Assert
        assert_eq!(result, Status::Ok);
        assert_eq!(
            RecordingStream::bytes_in(instance.state_mut(), BufferType::HttpRequestBody),
            b"beay"
        );
    }

    #[test]
    fn a_write_a_buffer_refuses_is_not_found() {
        // Arrange
        let engine = engine();
        let stream =
            RecordingStream::new().with_read_only_buffer(BufferType::HttpRequestBody, b"body");
        let (mut instance, _) = hosted(&engine, GUEST, stream);

        // Act
        let result = set(&mut instance, BODY, 0, 0, b"new ");

        // Assert
        assert_eq!(result, Status::NotFound);
        assert_eq!(
            RecordingStream::bytes_in(instance.state_mut(), BufferType::HttpRequestBody),
            b"body"
        );
    }

    #[test]
    fn a_write_to_the_vm_configuration_is_not_found() {
        // Arrange
        let mut instance = configured(b"vm bytes", b"plugin");

        // Act
        let result = set(&mut instance, VM, 0, 0, b"new ");

        // Assert
        assert_eq!(result, Status::NotFound);
        assert_eq!(
            instance.state().abi().services().vm_configuration(),
            b"vm bytes"
        );
    }

    #[test]
    fn the_embedder_receives_a_clamped_range_on_the_write_path() {
        // Arrange
        let engine = engine();
        let stream =
            RecordingStream::new().with_recording_buffer(BufferType::HttpRequestBody, b"body");
        let (mut instance, root) = hosted(&engine, GUEST, stream);

        // Act
        let result = set(&mut instance, BODY, -1, -1, b" more");

        // Assert
        assert_eq!(result, Status::Ok);
        let stream = RecordingStream::take(instance.state_mut());
        assert_eq!(stream.ranges(), vec![(4, 0)]);
        let (call, access, _) = stream.buffer_calls()[0];
        assert_eq!((call.context, access), (root, Access::Write));
    }

    #[test]
    fn the_embedder_receives_a_range_inside_its_buffer() {
        // Arrange
        let engine = engine();
        let stream = RecordingStream::new()
            .with_recording_buffer(BufferType::HttpRequestBody, b"0123456789");
        let (mut instance, _) = hosted(&engine, GUEST, stream);

        // Act
        let result = get(&mut instance, BODY, 0, -1);

        // Assert
        assert_eq!(result, Status::Ok);
        assert_eq!(
            RecordingStream::take(instance.state_mut()).ranges(),
            vec![(0, 10)]
        );
    }

    #[test]
    fn the_buffer_status_writes_the_length_and_a_zero_over_a_seeded_slot() {
        // Arrange
        let engine = engine();
        let (mut instance, _) = hosted(&engine, GUEST, body_of(b"0123456789"));
        seed(&mut instance, &[2008, 2012]);

        // Act
        let result = buffer_status(&mut instance, BODY);

        // Assert
        assert_eq!(result, Status::Ok);
        assert_eq!(status_slots(&mut instance), (10, 0));
    }

    #[test]
    fn the_buffer_status_of_an_empty_buffer_is_two_zeros() {
        // Arrange
        let engine = engine();
        let (mut instance, _) = hosted(&engine, GUEST, body_of(b""));
        seed(&mut instance, &[2008, 2012]);

        // Act
        let result = buffer_status(&mut instance, BODY);

        // Assert
        assert_eq!(result, Status::Ok);
        assert_eq!(status_slots(&mut instance), (0, 0));
    }

    #[test]
    fn every_pointer_argument_is_checked_before_the_buffer_is_reached() {
        // Arrange
        let engine = engine();
        let (mut instance, _) = hosted(&engine, GUEST, body_of(b"0123456789"));

        // Act
        let results = [
            outcome(proxy_get_buffer_bytes(
                instance.store_mut(),
                BODY,
                0,
                1,
                PAST_END,
                RETURN_SIZE,
            )),
            outcome(proxy_get_buffer_bytes(
                instance.store_mut(),
                BODY,
                0,
                1,
                RETURN_DATA,
                PAST_END,
            )),
            outcome(proxy_set_buffer_bytes(
                instance.store_mut(),
                BODY,
                0,
                0,
                PAST_END,
                4,
            )),
            outcome(proxy_get_buffer_status(
                instance.store_mut(),
                BODY,
                PAST_END,
                RETURN_SIZE,
            )),
            outcome(proxy_get_buffer_status(
                instance.store_mut(),
                BODY,
                RETURN_DATA,
                PAST_END,
            )),
        ];

        // Assert
        assert_eq!(results, [Status::InvalidMemoryAccess; 5]);
        assert_eq!(allocator_calls(&mut instance), 0);
        let stream = RecordingStream::take(instance.state_mut());
        assert!(stream.buffer_calls().is_empty());
        assert_eq!(stream.bytes(BufferType::HttpRequestBody), b"0123456789");
    }

    #[test]
    fn a_configuration_the_crate_serves_is_never_asked_of_the_stream_state() {
        // Arrange
        // The crate answers the two configuration buffers itself, so an
        // embedder must never see them.
        let mut instance = configured(b"vm bytes", b"plugin bytes");
        instance
            .state_mut()
            .abi_mut()
            .set_stream_state(Box::new(RecordingStream::new()));

        // Act
        let results = [
            get(&mut instance, VM, 0, -1),
            get(&mut instance, PLUGIN, 0, -1),
        ];

        // Assert
        assert_eq!(results, [Status::Ok; 2]);
        assert!(
            RecordingStream::take(instance.state_mut())
                .buffer_calls()
                .is_empty()
        );
    }

    #[test]
    fn the_plugin_configuration_of_another_root_is_not_reachable() {
        // Arrange
        // The crate serves the configuration of the root that owns the
        // effective context, so a second root's bytes must stay out of reach.
        let mut instance = configured(b"vm", b"mine");
        let contexts = instance.state_mut().abi_mut().contexts_mut();
        let theirs = contexts.create(None).unwrap();
        contexts.set_plugin(
            theirs,
            PluginConfig::new().with_configuration(b"theirs".to_vec()),
        );

        // Act
        let result = get(&mut instance, PLUGIN, 0, -1);

        // Assert
        assert_eq!(result, Status::Ok);
        assert_eq!(returned(&mut instance), b"mine");
    }

    #[test]
    fn a_write_to_a_configuration_the_crate_serves_is_never_asked_of_the_stream_state() {
        // Arrange
        let mut instance = configured(b"vm bytes", b"plugin bytes");
        instance
            .state_mut()
            .abi_mut()
            .set_stream_state(Box::new(RecordingStream::new()));

        // Act
        let results = [
            set(&mut instance, VM, 0, -1, b"new"),
            set(&mut instance, PLUGIN, 0, -1, b"new"),
        ];

        // Assert
        assert_eq!(results, [Status::NotFound; 2]);
        assert!(
            RecordingStream::take(instance.state_mut())
                .buffer_calls()
                .is_empty()
        );
    }

    #[test]
    fn a_guest_under_a_refused_root_is_not_served_the_plugin_configuration() {
        // Arrange
        // The existing test drives the body. This one drives a guest, so it
        // dies if the rejection is not consulted before the read.
        let mut instance = configured(b"vm", b"plugin bytes");
        let root = instance.state().abi().contexts().effective().unwrap();
        instance.state_mut().abi_mut().contexts_mut().reject(root);

        // Act
        let result = get(&mut instance, PLUGIN, 0, -1);

        // Assert
        assert_eq!(result, Status::NotFound);
    }

    const DELIVERED_BODY: i32 = BufferType::HttpCallResponseBody as i32;
    const GRPC_MESSAGE: i32 = BufferType::GrpcCallMessage as i32;

    /// An instance with a recording stream, in a delivery of a response with
    /// `body` when one is given.
    fn delivering(body: Option<&'static [u8]>) -> Instance {
        let (mut instance, _) = hosted(&engine(), GUEST, RecordingStream::new());
        let delivery = body.map(|body| {
            let header = vec![(
                Cow::Borrowed(b":status".as_slice()),
                Cow::Borrowed(b"200".as_slice()),
            )];
            let response = HttpCallResponse::received(header).with_body(Cow::Borrowed(body));
            Delivery::http_call_response(CalloutId::try_from(1_u32).unwrap(), response)
        });
        instance.state_mut().abi_mut().set_delivery(delivery);
        instance
    }

    fn asked(instance: &mut Instance) -> usize {
        RecordingStream::take(instance.state_mut()).calls().len()
    }

    #[test]
    fn the_body_of_a_delivered_response_is_sliced_as_every_buffer_is() {
        // Arrange
        let mut instance = delivering(Some(b"hello"));
        let ranges = [(0, i32::MAX), (1, 3), (5, 1)];

        // Act
        let answers = ranges.map(|(start, max_size)| {
            (
                get(&mut instance, DELIVERED_BODY, start, max_size),
                returned(&mut instance),
            )
        });

        // Assert
        assert_eq!(
            answers,
            [
                (Status::Ok, b"hello".to_vec()),
                (Status::Ok, b"ell".to_vec()),
                (Status::Ok, Vec::new()),
            ]
        );
        assert_eq!(asked(&mut instance), 0, "the stream state was not asked");
    }

    #[test]
    fn a_delivered_body_refuses_a_start_past_its_end_and_every_write() {
        // Arrange
        let mut instance = delivering(Some(b"hello"));

        // Act
        let answers = [
            get(&mut instance, DELIVERED_BODY, 6, 1),
            set(&mut instance, DELIVERED_BODY, 0, 0, b"new "),
        ];

        // Assert
        assert_eq!(answers, [Status::BadArgument, Status::NotFound]);
        assert_eq!(asked(&mut instance), 0, "the stream state was not asked");
    }

    #[test]
    fn the_status_of_a_delivered_body_is_its_length() {
        // Arrange
        let mut instance = delivering(Some(b"hello"));
        seed(&mut instance, &[2008, 2012]);

        // Act
        let result = buffer_status(&mut instance, DELIVERED_BODY);

        // Assert
        assert_eq!(result, Status::Ok);
        assert_eq!(status_slots(&mut instance), (5, 0));
    }

    #[test]
    fn the_body_of_a_response_is_not_found_outside_a_delivery() {
        // Arrange
        let mut instance = delivering(None);

        // Act
        let answers = [
            get(&mut instance, DELIVERED_BODY, 0, i32::MAX),
            set(&mut instance, DELIVERED_BODY, 0, 0, b"new "),
            buffer_status(&mut instance, DELIVERED_BODY),
        ];

        // Assert
        assert_eq!(answers, [Status::NotFound; 3]);
        assert_eq!(asked(&mut instance), 0, "the stream state was not asked");
    }

    #[test]
    fn the_predicate_names_the_five_crate_buffers_and_sends_the_rest_to_the_embedder() {
        // Arrange
        let asked = [
            BufferType::VmConfiguration,
            BufferType::PluginConfiguration,
            BufferType::HttpCallResponseBody,
            BufferType::GrpcCallMessage,
            BufferType::ForeignFunctionArguments,
            BufferType::HttpRequestBody,
            BufferType::HttpResponseBody,
        ];

        // Act
        let answers = asked.map(served);

        // Assert
        assert_eq!(
            answers,
            [
                Served::Crate(CrateBuffer::VmConfiguration),
                Served::Crate(CrateBuffer::PluginConfiguration),
                Served::Crate(CrateBuffer::Delivered(
                    DeliveredBuffer::HttpCallResponseBody
                )),
                Served::Crate(CrateBuffer::Delivered(DeliveredBuffer::GrpcCallMessage)),
                Served::Crate(CrateBuffer::Delivered(
                    DeliveredBuffer::ForeignFunctionArguments
                )),
                Served::Embedder,
                Served::Embedder,
            ]
        );
    }

    /// An instance with a recording stream, in a delivery of the gRPC
    /// message `message`.
    fn delivering_message(message: &'static [u8]) -> Instance {
        let (mut instance, _) = hosted(&engine(), GUEST, RecordingStream::new());
        let callout = CalloutId::try_from(1_u32).unwrap();
        let delivery = Delivery::grpc_message(callout, Cow::Borrowed(message));
        instance.state_mut().abi_mut().set_delivery(Some(delivery));
        instance
    }

    #[test]
    fn the_message_of_a_grpc_delivery_is_sliced_as_every_buffer_is() {
        // Arrange
        let mut instance = delivering_message(b"hello");
        let ranges = [(0, i32::MAX), (1, 3), (5, 1)];

        // Act
        let answers = ranges.map(|(start, max_size)| {
            (
                get(&mut instance, GRPC_MESSAGE, start, max_size),
                returned(&mut instance),
            )
        });

        // Assert
        assert_eq!(answers[0], (Status::Ok, b"hello".to_vec()));
        assert_eq!(answers[1], (Status::Ok, b"ell".to_vec()));
        assert_eq!(answers[2], (Status::Ok, Vec::new()));
        assert_eq!(
            buffer_status(&mut instance, GRPC_MESSAGE),
            Status::Ok,
            "the status of the buffer"
        );
        assert_eq!(asked(&mut instance), 0, "the stream state was not asked");
    }

    #[test]
    fn a_write_to_the_message_of_a_grpc_delivery_is_not_found() {
        // Arrange
        let mut instance = delivering_message(b"hello");

        // Act
        let answer = set(&mut instance, GRPC_MESSAGE, 0, 0, b"new ");

        // Assert
        assert_eq!(answer, Status::NotFound);
        assert_eq!(asked(&mut instance), 0);
    }

    #[test]
    fn the_message_buffer_is_not_found_in_another_delivery_and_outside_one() {
        // Arrange
        let mut response = delivering(Some(b"hello"));
        let mut outside = delivering(None);

        // Act
        let answers = [
            get(&mut response, GRPC_MESSAGE, 0, i32::MAX),
            buffer_status(&mut response, GRPC_MESSAGE),
            get(&mut outside, GRPC_MESSAGE, 0, i32::MAX),
            buffer_status(&mut outside, GRPC_MESSAGE),
        ];

        // Assert
        assert_eq!(answers, [Status::NotFound; 4]);
        assert_eq!(asked(&mut response), 0);
        assert_eq!(asked(&mut outside), 0);
    }

    #[test]
    fn the_response_body_is_not_found_in_a_grpc_delivery() {
        // Arrange
        let mut instance = delivering_message(b"hello");

        // Act
        let answers = [
            get(&mut instance, DELIVERED_BODY, 0, i32::MAX),
            buffer_status(&mut instance, DELIVERED_BODY),
            set(&mut instance, DELIVERED_BODY, 0, 0, b"new "),
        ];

        // Assert
        assert_eq!(answers, [Status::NotFound; 3]);
        assert_eq!(asked(&mut instance), 0);
    }

    /// A guest that reads the downstream data buffer and answers the status.
    const READER: &str = r#"(module
        (import "env" "proxy_get_buffer_bytes" (func $get (param i32 i32 i32 i32 i32) (result i32)))
        (memory (export "memory") 1)
        (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 8192)
        (func (export "proxy_abi_version_0_2_1"))
        (func (export "proxy_on_downstream_data") (param i32 i32 i32) (result i32)
            (i32.store (i32.const 0)
                (call $get (i32.const 2) (i32.const 0) (i32.const 2147483647)
                    (i32.const 100) (i32.const 104)))
            i32.const 0)
        (func (export "proxy_on_new_connection") (param i32) (result i32) i32.const 0))"#;

    /// Runs one downstream data callback of `announced` bytes on a stream
    /// state that holds `bytes`, and answers the warnings it reported.
    fn data_callback(announced: u32, bytes: &'static [u8], read: bool) -> usize {
        use crate::abi::v0_2_1::test_support::events::warnings;
        let engine = engine();
        let host = crate::abi::v0_2_1::Host::new(&engine).unwrap();
        let wat = if read {
            READER
        } else {
            r#"(module
                (memory (export "memory") 1)
                (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 8192)
                (func (export "proxy_abi_version_0_2_1")))"#
        };
        let module = Module::new(&engine, &wat_bytes(wat)).unwrap();
        let mut guest = crate::abi::v0_2_1::Guest::new(
            &host,
            &module,
            crate::abi::v0_2_1::test_support::services(),
            &crate::runtime::Limits::default(),
        )
        .unwrap();
        let root = guest.enter_root().on_context_create(None).unwrap();
        let stream = guest.enter_root().on_context_create(Some(root)).unwrap();
        let state = RecordingStream::new().with_buffer(BufferType::DownstreamData, bytes);
        warnings(|| {
            let mut scope = guest.enter(state);
            let _ = scope.on_downstream_data(stream, announced, false);
            drop(scope.finish());
        })
    }

    #[test]
    fn a_size_that_the_stream_state_does_not_hold_is_reported_one_time() {
        // Arrange
        let announced = 4096;

        // Act
        let warnings = data_callback(announced, b"four", true);

        // Assert
        assert_eq!(warnings, 1);
    }

    #[test]
    fn a_size_that_agrees_with_the_stream_state_reports_nothing() {
        // Arrange
        let announced = 4;

        // Act
        let warnings = data_callback(announced, b"four", true);

        // Assert
        assert_eq!(warnings, 0);
    }

    #[test]
    fn a_guest_that_does_not_read_the_buffer_reports_nothing() {
        // Arrange
        let announced = 4096;

        // Act
        let warnings = data_callback(announced, b"four", false);

        // Assert
        assert_eq!(warnings, 0);
    }

    #[test]
    fn a_read_outside_a_data_callback_reports_nothing() {
        // Arrange
        use crate::abi::v0_2_1::test_support::events::warnings;
        let state = RecordingStream::new().with_buffer(BufferType::DownstreamData, b"four");
        let (mut instance, _) = hosted(&engine(), GUEST, state);
        let buffer = BufferType::DownstreamData as i32;

        // Act
        let counted = warnings(|| {
            get(&mut instance, buffer, 0, i32::MAX);
        });

        // Assert
        assert_eq!(counted, 0);
        assert_eq!(returned(&mut instance), b"four".to_vec());
    }

    /// A guest that reads the downstream data buffer twice and reads it
    /// again in a connection close callback.
    const TWICE: &str = r#"(module
        (import "env" "proxy_get_buffer_bytes" (func $get (param i32 i32 i32 i32 i32) (result i32)))
        (import "env" "proxy_get_buffer_status" (func $size (param i32 i32 i32) (result i32)))
        (memory (export "memory") 1)
        (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 8192)
        (func (export "proxy_abi_version_0_2_1"))
        (func $read
            (i32.store (i32.const 0) (call $size (i32.const 2) (i32.const 8) (i32.const 12)))
            (i32.store (i32.const 4)
                (call $get (i32.const 2) (i32.const 0) (i32.const 2147483647)
                    (i32.const 100) (i32.const 104))))
        (func (export "proxy_on_downstream_data") (param i32 i32 i32) (result i32)
            call $read
            i32.const 0)
        (func (export "proxy_on_downstream_connection_close") (param i32 i32)
            call $read))"#;

    /// A guest, its stream context, and a stream state that holds `bytes`.
    fn data_guest(bytes: &'static [u8]) -> (crate::abi::v0_2_1::Guest, ContextId) {
        let engine = engine();
        let host = crate::abi::v0_2_1::Host::new(&engine).unwrap();
        let module = Module::new(&engine, &wat_bytes(TWICE)).unwrap();
        let mut guest = crate::abi::v0_2_1::Guest::new(
            &host,
            &module,
            crate::abi::v0_2_1::test_support::services(),
            &crate::runtime::Limits::default(),
        )
        .unwrap();
        let root = guest.enter_root().on_context_create(None).unwrap();
        let stream = guest.enter_root().on_context_create(Some(root)).unwrap();
        let _ = bytes;
        (guest, stream)
    }

    #[test]
    fn one_data_callback_reports_one_warning_however_often_the_guest_reads() {
        // Arrange
        use crate::abi::v0_2_1::test_support::events::warnings;
        let (mut guest, stream) = data_guest(b"four");
        let state = RecordingStream::new().with_buffer(BufferType::DownstreamData, b"four");

        // Act
        let counted = warnings(|| {
            let mut scope = guest.enter(state);
            let _ = scope.on_downstream_data(stream, 4096, false);
            drop(scope.finish());
        });

        // Assert
        assert_eq!(counted, 1, "the guest read the buffer twice");
    }

    #[test]
    fn a_size_no_guest_read_ends_with_its_callback() {
        // Arrange
        use crate::abi::v0_2_1::test_support::events::warnings;
        let quiet = r#"(module
            (import "env" "proxy_get_buffer_bytes" (func $get (param i32 i32 i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 8192)
            (func (export "proxy_abi_version_0_2_1"))
            (func (export "proxy_on_downstream_data") (param i32 i32 i32) (result i32) i32.const 0)
            (func (export "proxy_on_downstream_connection_close") (param i32 i32)
                (i32.store (i32.const 0)
                    (call $get (i32.const 2) (i32.const 0) (i32.const 2147483647)
                        (i32.const 100) (i32.const 104)))))"#;
        let engine = engine();
        let host = crate::abi::v0_2_1::Host::new(&engine).unwrap();
        let module = Module::new(&engine, &wat_bytes(quiet)).unwrap();
        let mut guest = crate::abi::v0_2_1::Guest::new(
            &host,
            &module,
            crate::abi::v0_2_1::test_support::services(),
            &crate::runtime::Limits::default(),
        )
        .unwrap();
        let root = guest.enter_root().on_context_create(None).unwrap();
        let stream = guest.enter_root().on_context_create(Some(root)).unwrap();
        let first = RecordingStream::new().with_buffer(BufferType::DownstreamData, b"four");
        let second = RecordingStream::new().with_buffer(BufferType::DownstreamData, b"four");
        let mut scope = guest.enter(first);
        let _ = scope.on_downstream_data(stream, 4096, false);
        drop(scope.finish());

        // Act
        let later = warnings(|| {
            let mut scope = guest.enter(second);
            let _ = scope.on_downstream_connection_close(stream, PeerType::Local);
            drop(scope.finish());
        });

        // Assert
        assert_eq!(
            later, 0,
            "the record of the data callback ended with that callback"
        );
    }

    #[test]
    fn the_record_of_a_size_ends_with_its_callback() {
        // Arrange
        use crate::abi::v0_2_1::test_support::events::warnings;
        let (mut guest, stream) = data_guest(b"four");
        let first = RecordingStream::new().with_buffer(BufferType::DownstreamData, b"four");
        let second = RecordingStream::new().with_buffer(BufferType::DownstreamData, b"four");
        let counted = warnings(|| {
            let mut scope = guest.enter(first);
            let _ = scope.on_downstream_data(stream, 4096, false);
            drop(scope.finish());
        });

        // Act
        let later = warnings(|| {
            let mut scope = guest.enter(second);
            let _ = scope.on_downstream_connection_close(stream, PeerType::Local);
            drop(scope.finish());
        });

        // Assert
        assert_eq!(counted, 1, "the data callback reported the difference");
        assert_eq!(later, 0, "the close callback announced no size");
    }

    #[test]
    fn a_read_of_another_buffer_inside_a_data_callback_reports_nothing() {
        // Arrange
        use crate::abi::v0_2_1::test_support::events::warnings;
        let reader = r#"(module
            (import "env" "proxy_get_buffer_bytes" (func $get (param i32 i32 i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 8192)
            (func (export "proxy_abi_version_0_2_1"))
            (func (export "proxy_on_downstream_data") (param i32 i32 i32) (result i32)
                (i32.store (i32.const 0)
                    (call $get (i32.const 3) (i32.const 0) (i32.const 2147483647)
                        (i32.const 100) (i32.const 104)))
                i32.const 0))"#;
        let engine = engine();
        let host = crate::abi::v0_2_1::Host::new(&engine).unwrap();
        let module = Module::new(&engine, &wat_bytes(reader)).unwrap();
        let mut guest = crate::abi::v0_2_1::Guest::new(
            &host,
            &module,
            crate::abi::v0_2_1::test_support::services(),
            &crate::runtime::Limits::default(),
        )
        .unwrap();
        let root = guest.enter_root().on_context_create(None).unwrap();
        let stream = guest.enter_root().on_context_create(Some(root)).unwrap();
        let state = RecordingStream::new()
            .with_buffer(BufferType::DownstreamData, b"four")
            .with_buffer(BufferType::UpstreamData, b"other");

        // Act
        let counted = warnings(|| {
            let mut scope = guest.enter(state);
            let _ = scope.on_downstream_data(stream, 4096, false);
            drop(scope.finish());
        });

        // Assert
        assert_eq!(counted, 0, "the guest read the buffer of the other side");
    }
}
