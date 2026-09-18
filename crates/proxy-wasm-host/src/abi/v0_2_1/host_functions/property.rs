//! `proxy_get_property` and `proxy_set_property`.
//!
//! The ABI names three properties that belong to Proxy-Wasm itself, and the
//! crate answers those from the plugin of the root and from the host
//! services.
//! Every other path goes to the stream host.

use wasmtime::AsContextMut;

use crate::abi::v0_2_1::host_functions::Failure;
use crate::abi::v0_2_1::host_functions::call::{context, from_embedder, with_stream};
use crate::abi::v0_2_1::types::Status;
use crate::codec::path::decode_path;
use crate::runtime::{GuestPtr, GuestSlice, HostState, split, write_return};

/// The plugin name, the plugin root id, and the VM id.
const PLUGIN_NAME: &[u8] = b"plugin_name";
const PLUGIN_ROOT_ID: &[u8] = b"plugin_root_id";
const PLUGIN_VM_ID: &[u8] = b"plugin_vm_id";

/// The value of a property the crate answers itself.
///
/// A path that is one of the three is answered or refused here and never
/// reaches the stream host, which is what the plugin configuration buffer
/// does.
/// The VM id needs no context, as the VM configuration needs none.
/// The plugin name and the plugin root id hang on a root context, so they
/// follow the rule every other body follows and refuse a root the guest
/// rejected.
fn well_known(state: &HostState, path: &[&[u8]]) -> Option<Result<Vec<u8>, Failure>> {
    let [segment] = path else {
        return None;
    };
    match *segment {
        PLUGIN_VM_ID => Some(Ok(state.services().vm_id().to_vec())),
        PLUGIN_NAME | PLUGIN_ROOT_ID => Some(plugin_value(state, segment)),
        _ => None,
    }
}

fn plugin_value(state: &HostState, segment: &[u8]) -> Result<Vec<u8>, Failure> {
    let effective = context(state, Status::NotFound)?;
    let plugin = state
        .abi()
        .contexts()
        .plugin(effective)
        .ok_or(Status::NotFound)?;
    if segment == PLUGIN_NAME {
        Ok(plugin.name().to_vec())
    } else {
        Ok(plugin.root_id().to_vec())
    }
}

fn is_well_known(path: &[&[u8]]) -> bool {
    matches!(path, [PLUGIN_NAME | PLUGIN_ROOT_ID | PLUGIN_VM_ID])
}

pub(super) fn proxy_get_property(
    ctx: &mut impl AsContextMut<Data = HostState>,
    path_data: i32,
    path_size: i32,
    return_value_data: i32,
    return_value_size: i32,
) -> Result<(), Failure> {
    let path = GuestSlice::try_from((path_data, path_size))?;
    let data_ptr = GuestPtr::try_from(return_value_data)?;
    let size_ptr = GuestPtr::try_from(return_value_size)?;
    let (memory, state) = split(ctx)?;
    memory.read_u32(data_ptr)?;
    memory.read_u32(size_ptr)?;
    let path = decode_path(memory.read(path)?);
    let value = if let Some(value) = well_known(state, &path) {
        value?
    } else {
        let (call, stream) = with_stream(state, Status::NotFound)?;
        from_embedder("property", stream.property(call, &path))?
    };
    write_return(ctx, &value, data_ptr, size_ptr)?;
    Ok(())
}

pub(super) fn proxy_set_property(
    ctx: &mut impl AsContextMut<Data = HostState>,
    path_data: i32,
    path_size: i32,
    value_data: i32,
    value_size: i32,
) -> Result<(), Failure> {
    let path = GuestSlice::try_from((path_data, path_size))?;
    let value = GuestSlice::try_from((value_data, value_size))?;
    let (memory, state) = split(ctx)?;
    let path = decode_path(memory.read(path)?);
    let value = memory.read(value)?;
    if is_well_known(&path) {
        return Err(Status::NotFound.into());
    }
    let (call, stream) = with_stream(state, Status::NotFound)?;
    from_embedder("set_property", stream.set_property(call, &path, value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::v0_2_1::Plugin;
    use crate::abi::v0_2_1::test_support::{
        RecordingStream, bare, hosted, outcome, returned, status, unhosted, write,
    };
    use crate::codec::path::encode_path;
    use crate::runtime::test_support::{RecordingSink, engine, wat_bytes};
    use crate::runtime::{HostServices, Instance, Limits, Module};

    const PATH: i32 = 1024;
    const VALUE: i32 = 1200;
    const RETURN_DATA: i32 = 2000;
    const RETURN_SIZE: i32 = 2004;
    const PAST_END: i32 = 65_534;

    const GUEST: &str = r#"(module
        (import "env" "proxy_get_property" (func $get (param i32 i32 i32 i32) (result i32)))
        (import "env" "proxy_set_property" (func $set (param i32 i32 i32 i32) (result i32)))
        (memory (export "memory") 1)
        (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 4096)
        (func (export "get") (param i32 i32 i32 i32) (result i32)
            local.get 0 local.get 1 local.get 2 local.get 3 call $get)
        (func (export "set") (param i32 i32 i32 i32) (result i32)
            local.get 0 local.get 1 local.get 2 local.get 3 call $set))"#;

    fn get(instance: &mut Instance, path: &[&[u8]]) -> Status {
        let encoded = encode_path(path);
        let (_, len) = write(instance, PATH, &encoded);
        status(
            instance
                .call::<(i32, i32, i32, i32), i32>("get", (PATH, len, RETURN_DATA, RETURN_SIZE))
                .unwrap(),
        )
    }

    fn set(instance: &mut Instance, path: &[&[u8]], value: &[u8]) -> Status {
        let encoded = encode_path(path);
        let (_, path_len) = write(instance, PATH, &encoded);
        let (_, value_len) = write(instance, VALUE, value);
        status(
            instance
                .call::<(i32, i32, i32, i32), i32>("set", (PATH, path_len, VALUE, value_len))
                .unwrap(),
        )
    }

    /// An instance with a VM id and a plugin on the root that a callback set.
    fn with_plugin(engine: &crate::runtime::Engine) -> Instance {
        let module = Module::new(engine, &wat_bytes(GUEST)).unwrap();
        let services = HostServices::new(std::sync::Arc::new(RecordingSink::default()))
            .with_vm_id(b"vm-1".to_vec());
        let mut instance = Instance::new(engine, &module, services, &Limits::default()).unwrap();
        let state = instance.state_mut();
        let root = state.abi_mut().contexts_mut().create(None).unwrap();
        state.abi_mut().contexts_mut().set_plugin(
            root,
            Plugin::new()
                .with_name(*b"auth")
                .with_root_id(*b"auth_root"),
        );
        state.abi_mut().contexts_mut().set_effective(root);
        instance
    }

    #[test]
    fn the_plugin_name_is_served_by_the_crate() {
        // Arrange
        let engine = engine();
        let mut instance = with_plugin(&engine);

        // Act
        let result = get(&mut instance, &[b"plugin_name"]);

        // Assert
        assert_eq!(result, Status::Ok);
        assert_eq!(returned(&mut instance, 2000, 2004), b"auth");
    }

    #[test]
    fn the_plugin_root_id_is_served_by_the_crate() {
        // Arrange
        let engine = engine();
        let mut instance = with_plugin(&engine);

        // Act
        let result = get(&mut instance, &[b"plugin_root_id"]);

        // Assert
        assert_eq!(result, Status::Ok);
        assert_eq!(returned(&mut instance, 2000, 2004), b"auth_root");
    }

    #[test]
    fn the_vm_id_is_served_without_a_plugin_and_without_a_context() {
        // Arrange
        let engine = engine();
        let module = Module::new(&engine, &wat_bytes(GUEST)).unwrap();
        let services = HostServices::new(std::sync::Arc::new(RecordingSink::default()))
            .with_vm_id(b"vm-1".to_vec());
        let mut instance = Instance::new(&engine, &module, services, &Limits::default()).unwrap();

        // Act
        let result = get(&mut instance, &[b"plugin_vm_id"]);

        // Assert
        assert_eq!(result, Status::Ok);
        assert_eq!(returned(&mut instance, 2000, 2004), b"vm-1");
    }

    #[test]
    fn a_well_known_path_never_reaches_the_stream_host() {
        // Arrange
        let engine = engine();
        let stream = RecordingStream::new().with_property(&[b"plugin_name"], b"wrong");
        let (mut instance, root) = hosted(&engine, GUEST, stream);
        instance
            .state_mut()
            .abi_mut()
            .contexts_mut()
            .set_plugin(root, Plugin::new().with_name(*b"auth"));

        // Act
        let result = get(&mut instance, &[b"plugin_name"]);

        // Assert
        assert_eq!(result, Status::Ok);
        assert_eq!(returned(&mut instance, 2000, 2004), b"auth");
    }

    #[test]
    fn another_path_reaches_the_stream_host() {
        // Arrange
        let engine = engine();
        let stream = RecordingStream::new().with_property(&[b"route", b"name"], b"main");
        let (mut instance, _) = hosted(&engine, GUEST, stream);

        // Act
        let result = get(&mut instance, &[b"route", b"name"]);

        // Assert
        assert_eq!(result, Status::Ok);
        assert_eq!(returned(&mut instance, 2000, 2004), b"main");
        let stream = RecordingStream::take(instance.state_mut());
        let (call, path) = &stream.property_reads()[0];
        assert_eq!(call.context.get(), 1);
        assert_eq!(path, &vec![b"route".to_vec(), b"name".to_vec()]);
    }

    #[test]
    fn a_path_the_stream_host_does_not_serve_is_not_found() {
        // Arrange
        let engine = engine();
        let (mut instance, _) = hosted(&engine, GUEST, RecordingStream::new());

        // Act
        let result = get(&mut instance, &[b"route", b"name"]);

        // Assert
        assert_eq!(result, Status::NotFound);
    }

    #[test]
    fn an_empty_path_reaches_the_stream_host() {
        // Arrange
        let engine = engine();
        let (mut instance, _) = hosted(&engine, GUEST, RecordingStream::new());

        // Act
        let result = get(&mut instance, &[]);

        // Assert
        assert_eq!(result, Status::NotFound);
    }

    #[test]
    fn a_write_reaches_the_stream_host_with_the_segments() {
        // Arrange
        let engine = engine();
        let (mut instance, root) = hosted(&engine, GUEST, RecordingStream::new());

        // Act
        let result = set(&mut instance, &[b"route", b"name"], b"main");

        // Assert
        assert_eq!(result, Status::Ok);
        let stream = RecordingStream::take(instance.state_mut());
        let (call, path, value) = &stream.property_writes()[0];
        assert_eq!(call.context, root);
        assert_eq!(path, &vec![b"route".to_vec(), b"name".to_vec()]);
        assert_eq!(value, b"main");
    }

    #[test]
    fn a_write_to_a_well_known_path_is_refused_without_the_stream_host() {
        // Arrange
        let engine = engine();
        let (mut instance, _) = hosted(&engine, GUEST, RecordingStream::new());

        // Act
        let result = set(&mut instance, &[b"plugin_name"], b"other");

        // Assert
        assert_eq!(result, Status::NotFound);
        assert!(
            RecordingStream::take(instance.state_mut())
                .property_writes()
                .is_empty()
        );
    }

    #[test]
    fn a_body_with_no_stream_host_or_no_context_is_not_found() {
        // Arrange
        let engine = engine();
        let (mut without_stream, _) = unhosted(&engine, GUEST);
        let mut without_context = bare(&engine, GUEST);

        // Act
        let results = [
            get(&mut without_stream, &[b"route"]),
            get(&mut without_context, &[b"route"]),
        ];

        // Assert
        assert_eq!(results, [Status::NotFound; 2]);
    }

    #[test]
    fn every_pointer_is_checked_before_the_stream_host_is_asked() {
        // Arrange
        let engine = engine();
        let (mut instance, _) = hosted(&engine, GUEST, RecordingStream::new());

        // Act
        let results = [
            outcome(proxy_get_property(
                instance.store_mut(),
                PAST_END,
                4,
                RETURN_DATA,
                RETURN_SIZE,
            )),
            outcome(proxy_get_property(
                instance.store_mut(),
                PATH,
                1,
                PAST_END,
                RETURN_SIZE,
            )),
            outcome(proxy_get_property(
                instance.store_mut(),
                PATH,
                1,
                RETURN_DATA,
                PAST_END,
            )),
            outcome(proxy_set_property(
                instance.store_mut(),
                PAST_END,
                4,
                VALUE,
                1,
            )),
            outcome(proxy_set_property(
                instance.store_mut(),
                PATH,
                1,
                PAST_END,
                4,
            )),
        ];

        // Assert
        assert_eq!(results, [Status::InvalidMemoryAccess; 5]);
        assert!(
            RecordingStream::take(instance.state_mut())
                .property_writes()
                .is_empty()
        );
    }

    #[test]
    fn a_read_under_a_refused_root_is_not_found() {
        // Arrange
        let engine = engine();
        let stream = RecordingStream::new().with_property(&[b"route"], b"main");
        let (mut instance, root) = hosted(&engine, GUEST, stream);
        instance.state_mut().abi_mut().contexts_mut().reject(root);

        // Act
        let result = get(&mut instance, &[b"route"]);

        // Assert
        assert_eq!(result, Status::NotFound);
    }

    #[test]
    fn the_plugin_name_of_a_refused_root_is_not_found() {
        // Arrange
        let engine = engine();
        let mut instance = with_plugin(&engine);
        let root = instance.state().abi().contexts().effective().unwrap();
        instance.state_mut().abi_mut().contexts_mut().reject(root);

        // Act
        let result = get(&mut instance, &[b"plugin_name"]);

        // Assert
        assert_eq!(result, Status::NotFound);
    }

    #[test]
    fn a_root_with_no_plugin_never_asks_the_stream_host_for_a_well_known_name() {
        // Arrange
        let engine = engine();
        let stream = RecordingStream::new().with_property(&[b"plugin_name"], b"wrong");
        let (mut instance, _) = hosted(&engine, GUEST, stream);

        // Act
        let result = get(&mut instance, &[b"plugin_name"]);

        // Assert
        assert_eq!(result, Status::NotFound);
        assert!(
            RecordingStream::take(instance.state_mut())
                .property_reads()
                .is_empty()
        );
    }

    #[test]
    fn a_well_known_property_is_never_asked_of_the_stream_host() {
        // Arrange
        // The crate answers the three plugin properties itself, so an
        // embedder must never see them.
        let engine = engine();
        let mut instance = with_plugin(&engine);
        instance
            .state_mut()
            .abi_mut()
            .set_stream_host(Box::new(RecordingStream::new()));

        // Act
        let results = [
            get(&mut instance, &[b"plugin_name"]),
            get(&mut instance, &[b"plugin_root_id"]),
            get(&mut instance, &[b"plugin_vm_id"]),
        ];

        // Assert
        assert_eq!(results, [Status::Ok; 3]);
        assert!(
            RecordingStream::take(instance.state_mut())
                .property_reads()
                .is_empty()
        );
    }
}
