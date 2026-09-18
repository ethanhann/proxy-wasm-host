//! The seven header map functions.
//!
//! Each one converts its arguments, checks every guest address it will use,
//! asks the stream host for the map, acts on the map, and last writes to
//! guest memory.

use std::borrow::Cow;

use wasmtime::AsContextMut;

use crate::abi::v0_2_1::Access;
use crate::abi::v0_2_1::host_functions::Failure;
use crate::abi::v0_2_1::host_functions::call::{from_embedder, with_stream};
use crate::abi::v0_2_1::types::{MapType, Status};
use crate::codec::pairs::decode_pairs;
use crate::header_map::HeaderMap;
use crate::runtime::{GuestPtr, GuestSlice, HostState, split, write_return};

fn map(
    state: &mut HostState,
    map_type: MapType,
    access: Access,
) -> Result<&mut dyn HeaderMap, Failure> {
    let (call, stream) = with_stream(state, Status::BadArgument)?;
    from_embedder("header_map", stream.header_map(call, access, map_type))
}

pub(super) fn proxy_get_header_map_size(
    ctx: &mut impl AsContextMut<Data = HostState>,
    map_id: i32,
    return_size: i32,
) -> Result<(), Failure> {
    let map_type = MapType::try_from(map_id)?;
    let return_size = GuestPtr::try_from(return_size)?;
    let (memory, state) = split(ctx)?;
    memory.read_u32(return_size)?;
    let size = map(state, map_type, Access::Read)?.encoded_size();
    let size = u32::try_from(size).map_err(|_| Status::InternalFailure)?;
    let (mut memory, _) = split(ctx)?;
    memory.write_u32(return_size, size)?;
    Ok(())
}

pub(super) fn proxy_get_header_map_pairs(
    ctx: &mut impl AsContextMut<Data = HostState>,
    map_id: i32,
    return_data: i32,
    return_size: i32,
) -> Result<(), Failure> {
    let map_type = MapType::try_from(map_id)?;
    let return_data = GuestPtr::try_from(return_data)?;
    let return_size = GuestPtr::try_from(return_size)?;
    let (memory, state) = split(ctx)?;
    memory.read_u32(return_data)?;
    memory.read_u32(return_size)?;
    let bytes = map(state, map_type, Access::Read)?.encode()?;
    write_return(ctx, &bytes, return_data, return_size)?;
    Ok(())
}

pub(super) fn proxy_set_header_map_pairs(
    ctx: &mut impl AsContextMut<Data = HostState>,
    map_id: i32,
    serialized_pairs_data: i32,
    serialized_pairs_size: i32,
) -> Result<(), Failure> {
    let map_type = MapType::try_from(map_id)?;
    let slice = GuestSlice::try_from((serialized_pairs_data, serialized_pairs_size))?;
    let (memory, state) = split(ctx)?;
    let bytes = memory.read(slice)?;
    let pairs = decode_pairs(bytes)?;
    map(state, map_type, Access::Write)?.replace_all(&pairs)?;
    Ok(())
}

pub(super) fn proxy_get_header_map_value(
    ctx: &mut impl AsContextMut<Data = HostState>,
    map_id: i32,
    key_data: i32,
    key_size: i32,
    return_data: i32,
    return_size: i32,
) -> Result<(), Failure> {
    let map_type = MapType::try_from(map_id)?;
    let key = GuestSlice::try_from((key_data, key_size))?;
    let return_data = GuestPtr::try_from(return_data)?;
    let return_size = GuestPtr::try_from(return_size)?;
    let (memory, state) = split(ctx)?;
    memory.read_u32(return_data)?;
    memory.read_u32(return_size)?;
    let key = memory.read(key)?;
    let value = map(state, map_type, Access::Read)?
        .get(key)
        .map(Cow::into_owned)
        .ok_or(Status::NotFound)?;
    write_return(ctx, &value, return_data, return_size)?;
    Ok(())
}

pub(super) fn proxy_add_header_map_value(
    ctx: &mut impl AsContextMut<Data = HostState>,
    map_id: i32,
    key_data: i32,
    key_size: i32,
    value_data: i32,
    value_size: i32,
) -> Result<(), Failure> {
    let map_type = MapType::try_from(map_id)?;
    let key = GuestSlice::try_from((key_data, key_size))?;
    let value = GuestSlice::try_from((value_data, value_size))?;
    let (memory, state) = split(ctx)?;
    let key = memory.read(key)?;
    let value = memory.read(value)?;
    map(state, map_type, Access::Write)?.add(key, value)?;
    Ok(())
}

pub(super) fn proxy_replace_header_map_value(
    ctx: &mut impl AsContextMut<Data = HostState>,
    map_id: i32,
    key_data: i32,
    key_size: i32,
    value_data: i32,
    value_size: i32,
) -> Result<(), Failure> {
    let map_type = MapType::try_from(map_id)?;
    let key = GuestSlice::try_from((key_data, key_size))?;
    let value = GuestSlice::try_from((value_data, value_size))?;
    let (memory, state) = split(ctx)?;
    let key = memory.read(key)?;
    let value = memory.read(value)?;
    map(state, map_type, Access::Write)?.set(key, value)?;
    Ok(())
}

pub(super) fn proxy_remove_header_map_value(
    ctx: &mut impl AsContextMut<Data = HostState>,
    map_id: i32,
    key_data: i32,
    key_size: i32,
) -> Result<(), Failure> {
    let map_type = MapType::try_from(map_id)?;
    let key = GuestSlice::try_from((key_data, key_size))?;
    let (memory, state) = split(ctx)?;
    let key = memory.read(key)?;
    map(state, map_type, Access::Write)?.remove(key)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::v0_2_1::test_support::{RecordingStream, hosted, outcome, status, write};
    use crate::abi::v0_2_1::{Callback, ContextId};
    use crate::codec::pairs::encode_pairs;
    use crate::runtime::test_support::{engine, instance};
    use crate::runtime::{Engine, Instance};

    const REQUEST: i32 = MapType::HttpRequestHeaders as i32;
    const RESPONSE: i32 = MapType::HttpResponseHeaders as i32;
    const KEY: i32 = 1024;
    const VALUE: i32 = 1100;
    const RETURN_DATA: i32 = 2000;
    const RETURN_SIZE: i32 = 2004;
    const PAST_END: i32 = 65_534;

    const GUEST: &str = r#"(module
        (import "env" "proxy_get_header_map_size" (func $size (param i32 i32) (result i32)))
        (import "env" "proxy_get_header_map_pairs" (func $pairs (param i32 i32 i32) (result i32)))
        (import "env" "proxy_set_header_map_pairs" (func $set_pairs (param i32 i32 i32) (result i32)))
        (import "env" "proxy_get_header_map_value" (func $get (param i32 i32 i32 i32 i32) (result i32)))
        (import "env" "proxy_add_header_map_value" (func $add (param i32 i32 i32 i32 i32) (result i32)))
        (import "env" "proxy_replace_header_map_value" (func $replace (param i32 i32 i32 i32 i32) (result i32)))
        (import "env" "proxy_remove_header_map_value" (func $remove (param i32 i32 i32) (result i32)))
        (import "env" "proxy_set_effective_context" (func $effective (param i32) (result i32)))
        (memory (export "memory") 1)
        (func (export "proxy_on_memory_allocate") (param i32) (result i32)
            (i32.store8 (i32.const 0) (i32.add (i32.load8_u (i32.const 0)) (i32.const 1)))
            i32.const 4096)
        (func (export "size") (param i32 i32) (result i32) local.get 0 local.get 1 call $size)
        (func (export "pairs") (param i32 i32 i32) (result i32) local.get 0 local.get 1 local.get 2 call $pairs)
        (func (export "set_pairs") (param i32 i32 i32) (result i32) local.get 0 local.get 1 local.get 2 call $set_pairs)
        (func (export "get") (param i32 i32 i32 i32 i32) (result i32)
            local.get 0 local.get 1 local.get 2 local.get 3 local.get 4 call $get)
        (func (export "add") (param i32 i32 i32 i32 i32) (result i32)
            local.get 0 local.get 1 local.get 2 local.get 3 local.get 4 call $add)
        (func (export "replace") (param i32 i32 i32 i32 i32) (result i32)
            local.get 0 local.get 1 local.get 2 local.get 3 local.get 4 call $replace)
        (func (export "remove") (param i32 i32 i32) (result i32) local.get 0 local.get 1 local.get 2 call $remove)
        (func (export "effective") (param i32) (result i32) local.get 0 call $effective))"#;

    fn setup(stream: RecordingStream) -> (Engine, Instance, ContextId) {
        let engine = engine();
        let (instance, root) = hosted(&engine, GUEST, stream);
        (engine, instance, root)
    }

    fn two_pairs() -> RecordingStream {
        RecordingStream::new().with_map(MapType::HttpRequestHeaders, &[("a", "1"), ("b", "22")])
    }

    fn read(instance: &mut Instance, at: u32, len: u32) -> Vec<u8> {
        let slice = GuestSlice::new(GuestPtr::from_address(at), len).unwrap();
        instance.memory().unwrap().read(slice).unwrap().to_vec()
    }

    fn return_slots(instance: &mut Instance) -> (u32, u32) {
        let memory = instance.memory().unwrap();
        let data = memory.read_u32(GuestPtr::from_address(2000)).unwrap();
        let size = memory.read_u32(GuestPtr::from_address(2004)).unwrap();
        (data, size)
    }

    fn allocator_calls(instance: &mut Instance) -> u8 {
        read(instance, 0, 1)[0]
    }

    fn seed_return_slots(instance: &mut Instance) {
        let mut memory = instance.memory().unwrap();
        memory
            .write_u32(GuestPtr::from_address(2000), 0x7f7f_7f7f)
            .unwrap();
        memory
            .write_u32(GuestPtr::from_address(2004), 0x7f7f_7f7f)
            .unwrap();
    }

    /// Drives every header map body once against `map_id` on a bare store.
    fn drive_all(instance: &mut Instance, map_id: i32) -> Vec<Status> {
        let store = instance.store_mut();
        vec![
            outcome(proxy_get_header_map_size(store, map_id, RETURN_SIZE)),
            outcome(proxy_get_header_map_pairs(
                store,
                map_id,
                RETURN_DATA,
                RETURN_SIZE,
            )),
            outcome(proxy_set_header_map_pairs(store, map_id, KEY, 0)),
            outcome(proxy_get_header_map_value(
                store,
                map_id,
                KEY,
                1,
                RETURN_DATA,
                RETURN_SIZE,
            )),
            outcome(proxy_add_header_map_value(store, map_id, KEY, 1, VALUE, 1)),
            outcome(proxy_replace_header_map_value(
                store, map_id, KEY, 1, VALUE, 1,
            )),
            outcome(proxy_remove_header_map_value(store, map_id, KEY, 1)),
        ]
    }

    #[test]
    fn size_reports_the_encoded_size_and_zero_for_an_empty_map() {
        // Arrange
        let (_engine, mut full, _) = setup(two_pairs());
        let (_engine2, mut empty, _) = setup(RecordingStream::new());
        seed_return_slots(&mut empty);

        // Act
        let results = (
            full.call::<(i32, i32), i32>("size", (REQUEST, RETURN_SIZE))
                .map(status),
            empty
                .call::<(i32, i32), i32>("size", (REQUEST, RETURN_SIZE))
                .map(status),
        );

        // Assert
        assert_eq!(results.0.unwrap(), Status::Ok);
        assert_eq!(results.1.unwrap(), Status::Ok);
        assert_eq!(return_slots(&mut full).1, 4 + 16 + 4 + 5);
        assert_eq!(return_slots(&mut empty).1, 0);
    }

    #[test]
    fn pairs_round_trip_through_the_codec() {
        // Arrange
        let (_engine, mut instance, _) = setup(two_pairs());

        // Act
        let result = instance
            .call::<(i32, i32, i32), i32>("pairs", (REQUEST, RETURN_DATA, RETURN_SIZE))
            .map(status);

        // Assert
        assert_eq!(result.unwrap(), Status::Ok);
        let (data, size) = return_slots(&mut instance);
        assert_eq!(data, 4096);
        let bytes = read(&mut instance, data, size);
        let decoded = decode_pairs(&bytes).unwrap();
        assert_eq!(
            decoded,
            vec![(b"a".as_slice(), b"1".as_slice()), (b"b", b"22")]
        );
    }

    #[test]
    fn pairs_of_an_empty_map_write_two_zeros_without_allocating() {
        // Arrange
        let (_engine, mut instance, _) = setup(RecordingStream::new());
        seed_return_slots(&mut instance);

        // Act
        let result = instance
            .call::<(i32, i32, i32), i32>("pairs", (REQUEST, RETURN_DATA, RETURN_SIZE))
            .map(status);

        // Assert
        assert_eq!(result.unwrap(), Status::Ok);
        assert_eq!(return_slots(&mut instance), (0, 0));
        assert_eq!(allocator_calls(&mut instance), 0);
    }

    #[test]
    fn set_pairs_replaces_the_map_and_accepts_every_empty_encoding() {
        // Arrange
        let (_engine, mut instance, _) = setup(two_pairs());
        let encoded = encode_pairs(&[("x", "9"), ("y", "8")]).unwrap();
        let (data, size) = write(&mut instance, KEY, &encoded);
        let replaced = instance
            .call::<(i32, i32, i32), i32>("set_pairs", (REQUEST, data, size))
            .map(status)
            .unwrap();
        let after_replace = RecordingStream::pairs_in(instance.state_mut());
        let (zero, _) = write(&mut instance, VALUE, &[0, 0, 0, 0]);

        // Act
        let results = (
            instance
                .call::<(i32, i32, i32), i32>("set_pairs", (REQUEST, data, 0))
                .map(status),
            instance
                .call::<(i32, i32, i32), i32>("set_pairs", (REQUEST, zero, 1))
                .map(status),
            instance
                .call::<(i32, i32, i32), i32>("set_pairs", (REQUEST, zero, 4))
                .map(status),
        );

        // Assert
        assert_eq!(replaced, Status::Ok);
        assert_eq!(
            after_replace,
            vec![("x".into(), "9".into()), ("y".into(), "8".into())]
        );
        assert_eq!(results.0.unwrap(), Status::Ok);
        assert_eq!(results.1.unwrap(), Status::Ok);
        assert_eq!(results.2.unwrap(), Status::Ok);
        assert!(RecordingStream::pairs_in(instance.state_mut()).is_empty());
    }

    #[test]
    fn set_pairs_with_a_truncated_encoding_leaves_the_map_unchanged() {
        // Arrange
        let (_engine, mut instance, _) = setup(two_pairs());
        let encoded = encode_pairs(&[("x", "9")]).unwrap();
        let (data, size) = write(&mut instance, KEY, &encoded[..encoded.len() - 2]);

        // Act
        let result = instance
            .call::<(i32, i32, i32), i32>("set_pairs", (REQUEST, data, size))
            .map(status);

        // Assert
        assert_eq!(result.unwrap(), Status::BadArgument);
        assert_eq!(
            RecordingStream::pairs_in(instance.state_mut()),
            vec![("a".into(), "1".into()), ("b".into(), "22".into())]
        );
    }

    #[test]
    fn get_value_returns_the_first_value_or_not_found() {
        // Arrange
        let stream = RecordingStream::new().with_map(
            MapType::HttpRequestHeaders,
            &[("a", "1"), ("a", "2"), ("e", ""), ("", "empty-key")],
        );
        let (_engine, mut instance, _) = setup(stream);
        let (a, a_len) = write(&mut instance, KEY, b"a");
        let (e, e_len) = write(&mut instance, KEY + 8, b"e");
        let (z, z_len) = write(&mut instance, KEY + 16, b"z");

        // Act
        let results = [
            instance
                .call::<(i32, i32, i32, i32, i32), i32>(
                    "get",
                    (REQUEST, a, a_len, RETURN_DATA, RETURN_SIZE),
                )
                .map(status),
            instance
                .call::<(i32, i32, i32, i32, i32), i32>(
                    "get",
                    (REQUEST, z, z_len, RETURN_DATA, RETURN_SIZE),
                )
                .map(status),
            instance
                .call::<(i32, i32, i32, i32, i32), i32>(
                    "get",
                    (REQUEST, e, e_len, RETURN_DATA, RETURN_SIZE),
                )
                .map(status),
            instance
                .call::<(i32, i32, i32, i32, i32), i32>(
                    "get",
                    (REQUEST, KEY, 0, RETURN_DATA, RETURN_SIZE),
                )
                .map(status),
        ];

        // Assert
        assert_eq!(results[0].as_ref().unwrap(), &Status::Ok);
        assert_eq!(results[1].as_ref().unwrap(), &Status::NotFound);
        assert_eq!(results[2].as_ref().unwrap(), &Status::Ok);
        assert_eq!(results[3].as_ref().unwrap(), &Status::Ok);
        let (data, size) = return_slots(&mut instance);
        assert_eq!(read(&mut instance, data, size), b"empty-key");
        assert_eq!(allocator_calls(&mut instance), 2);
    }

    #[test]
    fn get_value_of_an_empty_value_writes_two_zeros() {
        // Arrange
        let stream = RecordingStream::new().with_map(MapType::HttpRequestHeaders, &[("e", "")]);
        let (_engine, mut instance, _) = setup(stream);
        seed_return_slots(&mut instance);
        let (e, e_len) = write(&mut instance, KEY, b"e");

        // Act
        let result = instance
            .call::<(i32, i32, i32, i32, i32), i32>(
                "get",
                (REQUEST, e, e_len, RETURN_DATA, RETURN_SIZE),
            )
            .map(status);

        // Assert
        assert_eq!(result.unwrap(), Status::Ok);
        assert_eq!(return_slots(&mut instance), (0, 0));
        assert_eq!(allocator_calls(&mut instance), 0);
    }

    #[test]
    fn add_replace_and_remove_change_the_map() {
        // Arrange
        let stream = RecordingStream::new().with_map(
            MapType::HttpRequestHeaders,
            &[("a", "1"), ("b", "22"), ("b", "3")],
        );
        let (_engine, mut instance, _) = setup(stream);
        let (a, a_len) = write(&mut instance, KEY, b"a");
        let (b, b_len) = write(&mut instance, KEY + 8, b"b");
        let (v, v_len) = write(&mut instance, VALUE, b"new");
        let (z, z_len) = write(&mut instance, KEY + 16, b"z");

        // Act
        let results = [
            instance
                .call::<(i32, i32, i32, i32, i32), i32>("add", (REQUEST, a, a_len, v, 0))
                .map(status),
            instance
                .call::<(i32, i32, i32, i32, i32), i32>("replace", (REQUEST, b, b_len, v, v_len))
                .map(status),
            instance
                .call::<(i32, i32, i32), i32>("remove", (REQUEST, z, z_len))
                .map(status),
        ];

        // Assert
        assert!(
            results
                .iter()
                .all(|result| result.as_ref().unwrap() == &Status::Ok)
        );
        assert_eq!(
            RecordingStream::pairs_in(instance.state_mut()),
            vec![
                ("a".into(), "1".into()),
                ("b".into(), "new".into()),
                ("a".into(), String::new())
            ]
        );
    }

    #[test]
    fn remove_of_every_pair_of_a_key_is_ok() {
        // Arrange
        let stream = RecordingStream::new().with_map(
            MapType::HttpRequestHeaders,
            &[("a", "1"), ("a", "2"), ("b", "3")],
        );
        let (_engine, mut instance, _) = setup(stream);
        let (a, a_len) = write(&mut instance, KEY, b"a");

        // Act
        let result = instance
            .call::<(i32, i32, i32), i32>("remove", (REQUEST, a, a_len))
            .map(status);

        // Assert
        assert_eq!(result.unwrap(), Status::Ok);
        assert_eq!(
            RecordingStream::pairs_in(instance.state_mut()),
            vec![("b".into(), "3".into())]
        );
    }

    #[test]
    fn an_unknown_map_type_is_bad_argument_everywhere() {
        // Arrange
        let (_engine, mut instance, _) = setup(two_pairs());

        // Act
        let results = [
            outcome(proxy_get_header_map_size(
                instance.store_mut(),
                8,
                RETURN_SIZE,
            )),
            outcome(proxy_get_header_map_pairs(
                instance.store_mut(),
                8,
                RETURN_DATA,
                RETURN_SIZE,
            )),
            outcome(proxy_set_header_map_pairs(instance.store_mut(), 8, KEY, 0)),
            outcome(proxy_get_header_map_value(
                instance.store_mut(),
                8,
                KEY,
                1,
                RETURN_DATA,
                RETURN_SIZE,
            )),
            outcome(proxy_add_header_map_value(
                instance.store_mut(),
                8,
                KEY,
                1,
                VALUE,
                1,
            )),
            outcome(proxy_replace_header_map_value(
                instance.store_mut(),
                8,
                KEY,
                1,
                VALUE,
                1,
            )),
            outcome(proxy_remove_header_map_value(
                instance.store_mut(),
                8,
                KEY,
                1,
            )),
        ];

        // Assert
        assert!(results.iter().all(|status| *status == Status::BadArgument));
        assert!(
            RecordingStream::take(instance.state_mut())
                .calls()
                .is_empty()
        );
    }

    #[test]
    fn without_a_stream_host_or_an_effective_context_every_function_is_bad_argument() {
        // Arrange
        let (_engine, mut no_stream, _) = setup(two_pairs());
        let _ = no_stream.state_mut().abi_mut().take_stream_host();
        let (_engine2, mut no_context, root) = setup(two_pairs());
        let _ = no_context.state_mut().abi_mut().contexts_mut().remove(root);

        // Act
        let results = (
            drive_all(&mut no_stream, REQUEST),
            drive_all(&mut no_context, REQUEST),
        );

        // Assert
        assert_eq!(results.0, vec![Status::BadArgument; 7]);
        assert_eq!(results.1, vec![Status::BadArgument; 7]);
    }

    #[test]
    fn a_refused_root_is_not_served() {
        // Arrange
        let (_engine, mut instance, root) = setup(two_pairs());
        instance.state_mut().abi_mut().contexts_mut().reject(root);

        // Act
        let results = drive_all(&mut instance, REQUEST);

        // Assert
        assert_eq!(results, vec![Status::BadArgument; 7]);
        assert!(
            RecordingStream::take(instance.state_mut())
                .calls()
                .is_empty()
        );
    }

    #[test]
    fn the_embedder_status_passes_through_for_every_function() {
        // Arrange
        let stream =
            RecordingStream::new().refusing(MapType::HttpResponseHeaders, Status::NotFound);
        let (_engine, mut instance, _) = setup(stream);

        // Act
        let results = drive_all(&mut instance, RESPONSE);

        // Assert
        assert_eq!(results, vec![Status::NotFound; 7]);
        assert_eq!(RecordingStream::take(instance.state_mut()).calls().len(), 7);
    }

    #[test]
    fn an_ok_refusal_is_reported_as_internal_failure() {
        // Arrange
        let stream = RecordingStream::new().refusing(MapType::HttpResponseHeaders, Status::Ok);
        let (_engine, mut instance, _) = setup(stream);

        // Act
        let results = drive_all(&mut instance, RESPONSE);

        // Assert
        assert_eq!(results, vec![Status::InternalFailure; 7]);
    }

    #[test]
    fn not_allowed_is_bad_argument_and_leaves_the_map_unchanged() {
        // Arrange
        let stream =
            RecordingStream::new().with_read_only_map(MapType::HttpRequestHeaders, &[("a", "1")]);
        let (_engine, mut instance, _) = setup(stream);
        let (a, a_len) = write(&mut instance, KEY, b"a");
        let (v, v_len) = write(&mut instance, VALUE, b"new");

        // Act
        let result = instance
            .call::<(i32, i32, i32, i32, i32), i32>("replace", (REQUEST, a, a_len, v, v_len))
            .map(status);

        // Assert
        assert_eq!(result.unwrap(), Status::BadArgument);
        assert_eq!(
            RecordingStream::pairs_in(instance.state_mut()),
            vec![("a".into(), "1".into())]
        );
    }

    #[test]
    fn a_null_allocator_is_internal_failure_and_a_trapping_one_unwinds() {
        // Arrange
        let null = GUEST.replace("i32.const 4096)", "i32.const 0)");
        let trapping = GUEST.replace("i32.const 4096)", "unreachable)");
        let engine = engine();
        let mut instances: Vec<Instance> = [null, trapping]
            .iter()
            .map(|wat| {
                let mut instance = instance(&engine, wat).unwrap();
                let state = instance.state_mut();
                let root = state.abi_mut().contexts_mut().create(None).unwrap();
                state.abi_mut().contexts_mut().set_effective(root);
                state.abi_mut().set_stream_host(Box::new(two_pairs()));
                instance
            })
            .collect();

        // Act
        let results = (
            instances[0]
                .call::<(i32, i32, i32), i32>("pairs", (REQUEST, RETURN_DATA, RETURN_SIZE))
                .map(status),
            instances[1]
                .call::<(i32, i32, i32), i32>("pairs", (REQUEST, RETURN_DATA, RETURN_SIZE))
                .map(status),
        );

        // Assert
        assert_eq!(results.0.unwrap(), Status::InternalFailure);
        assert!(!instances[0].is_poisoned());
        assert!(matches!(results.1, Err(crate::Error::Trap { .. })));
        assert!(instances[1].is_poisoned());
    }

    #[test]
    fn the_stream_sees_the_call_context_the_access_and_a_changed_effective_context() {
        // Arrange
        let (_engine, mut instance, root) = setup(two_pairs());
        let stream = instance
            .state_mut()
            .abi_mut()
            .contexts_mut()
            .create(Some(root))
            .unwrap();
        let (a, a_len) = write(&mut instance, KEY, b"a");
        let _ = instance
            .call::<(i32, i32), i32>("size", (REQUEST, RETURN_SIZE))
            .unwrap();
        let _ = instance
            .call::<i32, i32>("effective", stream.wire())
            .unwrap();

        // Act
        let _ = instance
            .call::<(i32, i32, i32), i32>("remove", (REQUEST, a, a_len))
            .unwrap();

        // Assert
        let calls = RecordingStream::take(instance.state_mut()).calls().to_vec();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].0.context, root);
        assert_eq!(calls[0].0.callback, Some(Callback::RequestHeaders));
        assert_eq!(calls[0].1, Access::Read);
        assert_eq!(calls[1].0.context, stream);
        assert_eq!(calls[1].1, Access::Write);
        assert_eq!(calls[1].1, Access::Write);
    }

    #[test]
    fn every_pointer_argument_is_bounds_checked_before_the_map_is_asked() {
        // Arrange
        let (_engine, mut instance, _) = setup(two_pairs());
        let store = instance.store_mut();

        // Act
        let results = [
            outcome(proxy_get_header_map_size(store, REQUEST, PAST_END)),
            outcome(proxy_get_header_map_pairs(
                store,
                REQUEST,
                PAST_END,
                RETURN_SIZE,
            )),
            outcome(proxy_get_header_map_pairs(
                store,
                REQUEST,
                RETURN_DATA,
                PAST_END,
            )),
            outcome(proxy_set_header_map_pairs(store, REQUEST, PAST_END, 8)),
            outcome(proxy_get_header_map_value(
                store,
                REQUEST,
                PAST_END,
                8,
                RETURN_DATA,
                RETURN_SIZE,
            )),
            outcome(proxy_get_header_map_value(
                store,
                REQUEST,
                KEY,
                1,
                PAST_END,
                RETURN_SIZE,
            )),
            outcome(proxy_get_header_map_value(
                store,
                REQUEST,
                KEY,
                1,
                RETURN_DATA,
                PAST_END,
            )),
            outcome(proxy_add_header_map_value(
                store, REQUEST, PAST_END, 8, VALUE, 1,
            )),
            outcome(proxy_add_header_map_value(
                store, REQUEST, KEY, 1, PAST_END, 8,
            )),
            outcome(proxy_replace_header_map_value(
                store, REQUEST, PAST_END, 8, VALUE, 1,
            )),
            outcome(proxy_replace_header_map_value(
                store, REQUEST, KEY, 1, PAST_END, 8,
            )),
            outcome(proxy_remove_header_map_value(store, REQUEST, PAST_END, 8)),
        ];

        // Assert
        assert!(
            results
                .iter()
                .all(|status| *status == Status::InvalidMemoryAccess)
        );
        assert_eq!(allocator_calls(&mut instance), 0);
        assert!(
            RecordingStream::take(instance.state_mut())
                .calls()
                .is_empty()
        );
    }
}
