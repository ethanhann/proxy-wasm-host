//! `proxy_set_shared_data` and `proxy_get_shared_data`.
//!
//! The shared data is separated by the VM id, so one plugin cannot read a key
//! of another through a name it guesses.

use wasmtime::AsContextMut;

use crate::abi::v0_2_1::AbiAccess;
use crate::abi::v0_2_1::host_functions::Failure;
use crate::abi::v0_2_1::host_functions::bounds::within_name_bytes;
use crate::abi::v0_2_1::host_functions::call::{from_embedder, with_shared};
use crate::abi::v0_2_1::types::Status;
use crate::runtime::{GuestPtr, GuestSlice, HostState, split, write_return};

pub(super) fn proxy_set_shared_data(
    ctx: &mut impl AsContextMut<Data = HostState>,
    key_data: i32,
    key_size: i32,
    value_data: i32,
    value_size: i32,
    cas: i32,
) -> Result<(), Failure> {
    let key = GuestSlice::try_from((key_data, key_size))?;
    let value = GuestSlice::try_from((value_data, value_size))?;
    let cas = match cas.cast_unsigned() {
        0 => None,
        given => Some(given),
    };
    let (memory, state) = split(ctx)?;
    let key = memory.read(key)?;
    let value = memory.read(value)?;
    let (call, shared) = with_shared(state, Status::NotFound)?;
    within_name_bytes(state, key, Status::InternalFailure)?;
    let vm_id = state.abi().services().vm_id();
    from_embedder(
        "set_shared_data",
        shared.set_shared_data(call, vm_id, key, value, cas),
    )
}

pub(super) fn proxy_get_shared_data(
    ctx: &mut impl AsContextMut<Data = HostState>,
    key_data: i32,
    key_size: i32,
    return_value_data: i32,
    return_value_size: i32,
    return_cas: i32,
) -> Result<(), Failure> {
    let key = GuestSlice::try_from((key_data, key_size))?;
    let data_ptr = GuestPtr::try_from(return_value_data)?;
    let size_ptr = GuestPtr::try_from(return_value_size)?;
    let cas_ptr = GuestPtr::try_from(return_cas)?;
    let (memory, state) = split(ctx)?;
    memory.read_u32(data_ptr)?;
    memory.read_u32(size_ptr)?;
    memory.read_u32(cas_ptr)?;
    let key = memory.read(key)?;
    let (call, shared) = with_shared(state, Status::NotFound)?;
    within_name_bytes(state, key, Status::NotFound)?;
    let vm_id = state.abi().services().vm_id();
    let value = from_embedder("get_shared_data", shared.get_shared_data(call, vm_id, key))?;
    let cas = value.cas.get();
    write_return(ctx, &value.bytes, data_ptr, size_ptr)?;
    let (mut memory, _) = split(ctx)?;
    memory.write_u32(cas_ptr, cas)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::abi::v0_2_1::AbiAccess;
    use crate::abi::v0_2_1::test_support::services::{RecordingServices, SharedCall};
    use crate::abi::v0_2_1::test_support::{
        VM_ID, bare, engine, outcome, returned, shared_hosted, shared_hosted_with_limits, status,
        write,
    };
    use crate::abi::v0_2_1::{ContextId, GuestId, InMemoryStore, Invocation, SharedServices};
    use crate::runtime::{GuestPtr, Instance, Limits};

    const KEY: i32 = 1024;
    const VALUE: i32 = 1100;
    const RETURN_DATA: i32 = 2000;
    const RETURN_SIZE: i32 = 2004;
    const RETURN_CAS: i32 = 2008;
    const SENTINEL: u32 = 0x7f7f_7f7f;
    const PAST_END: i32 = 65_534;

    const GUEST: &str = r#"(module
        (import "env" "proxy_set_shared_data" (func $set (param i32 i32 i32 i32 i32) (result i32)))
        (import "env" "proxy_get_shared_data" (func $get (param i32 i32 i32 i32 i32) (result i32)))
        (memory (export "memory") 1)
        (func (export "proxy_on_memory_allocate") (param i32) (result i32)
            (i32.store8 (i32.const 0) (i32.add (i32.load8_u (i32.const 0)) (i32.const 1)))
            i32.const 4096)
        (func (export "set") (param i32 i32 i32 i32 i32) (result i32)
            local.get 0 local.get 1 local.get 2 local.get 3 local.get 4 call $set)
        (func (export "get") (param i32 i32 i32 i32 i32) (result i32)
            local.get 0 local.get 1 local.get 2 local.get 3 local.get 4 call $get))"#;

    fn set(instance: &mut Instance, key: &[u8], value: &[u8], cas: i32) -> Status {
        let (_, key_len) = write(instance, KEY, key);
        let (_, value_len) = write(instance, VALUE, value);
        status(
            instance
                .call::<(i32, i32, i32, i32, i32), i32>(
                    "set",
                    (KEY, key_len, VALUE, value_len, cas),
                )
                .unwrap(),
        )
    }

    fn get(instance: &mut Instance, key: &[u8]) -> Status {
        let (_, key_len) = write(instance, KEY, key);
        status(
            instance
                .call::<(i32, i32, i32, i32, i32), i32>(
                    "get",
                    (KEY, key_len, RETURN_DATA, RETURN_SIZE, RETURN_CAS),
                )
                .unwrap(),
        )
    }

    fn value_and_cas(instance: &mut Instance) -> (Vec<u8>, u32) {
        let bytes = returned(instance, 2000, 2004);
        let cas = instance
            .memory()
            .unwrap()
            .read_u32(GuestPtr::from_address(2008))
            .unwrap();
        (bytes, cas)
    }

    fn seed(instance: &mut Instance) {
        let mut memory = instance.memory().unwrap();
        for address in [2000, 2004, 2008] {
            memory
                .write_u32(GuestPtr::from_address(address), SENTINEL)
                .unwrap();
        }
    }

    fn allocator_calls(instance: &mut Instance) -> u8 {
        let slice = crate::runtime::GuestSlice::try_from((0, 1)).unwrap();
        instance.memory().unwrap().read(slice).unwrap()[0]
    }

    #[test]
    fn a_value_written_by_the_guest_reads_back_with_its_number() {
        // Arrange
        let engine = engine();
        let shared: Arc<dyn SharedServices> = Arc::new(InMemoryStore::new());
        let (mut instance, _) = shared_hosted(&engine, GUEST, shared);
        assert_eq!(set(&mut instance, b"k", b"v", 0), Status::Ok);

        // Act
        let result = get(&mut instance, b"k");

        // Assert
        assert_eq!(result, Status::Ok);
        let (value, cas) = value_and_cas(&mut instance);
        assert_eq!(value, b"v");
        assert_ne!(cas, 0);
    }

    #[test]
    fn a_key_that_is_not_there_is_not_found() {
        // Arrange
        let engine = engine();
        let shared: Arc<dyn SharedServices> = Arc::new(InMemoryStore::new());
        let (mut instance, _) = shared_hosted(&engine, GUEST, shared);

        // Act
        let result = get(&mut instance, b"missing");

        // Assert
        assert_eq!(result, Status::NotFound);
    }

    #[test]
    fn the_body_passes_the_vm_id_of_this_instance() {
        // Arrange
        let engine = engine();
        let recording = Arc::new(RecordingServices::new());
        let (mut instance, root) = shared_hosted(&engine, GUEST, recording.clone());

        // Act
        let result = set(&mut instance, b"k", b"v", 0);

        // Assert
        assert_eq!(result, Status::Ok);
        let calls = recording.calls();
        assert_eq!(
            calls[0].1,
            SharedCall::Set(VM_ID.to_vec(), b"k".to_vec(), b"v".to_vec(), None)
        );
        assert_eq!(calls[0].0.context, root);
    }

    #[test]
    fn a_number_that_is_not_zero_is_passed_as_a_comparison() {
        // Arrange
        let engine = engine();
        let recording = Arc::new(RecordingServices::new());
        let (mut instance, _) = shared_hosted(&engine, GUEST, recording.clone());
        set(&mut instance, b"k", b"first", 0);

        // Act
        let result = set(&mut instance, b"k", b"v", 7);

        // Assert
        assert_eq!(result, Status::CasMismatch);
        assert_eq!(
            recording.calls()[1].1,
            SharedCall::Set(VM_ID.to_vec(), b"k".to_vec(), b"v".to_vec(), Some(7))
        );
    }

    #[test]
    fn a_number_above_the_signed_limit_is_read_as_unsigned() {
        // Arrange
        let engine = engine();
        let recording = Arc::new(RecordingServices::new());
        let (mut instance, _) = shared_hosted(&engine, GUEST, recording.clone());
        set(&mut instance, b"k", b"first", 0);

        // Act
        let result = set(&mut instance, b"k", b"v", -1);

        // Assert
        assert_eq!(result, Status::CasMismatch);
        assert_eq!(
            recording.calls()[1].1,
            SharedCall::Set(VM_ID.to_vec(), b"k".to_vec(), b"v".to_vec(), Some(u32::MAX))
        );
    }

    #[test]
    fn a_status_the_services_return_passes_through() {
        // Arrange
        let engine = engine();
        let refusing = Arc::new(RecordingServices::new().refusing(Status::CasMismatch));
        let (mut instance, _) = shared_hosted(&engine, GUEST, refusing);

        // Act
        let result = set(&mut instance, b"k", b"v", 0);

        // Assert
        assert_eq!(result, Status::CasMismatch);
    }

    #[test]
    fn a_body_with_no_effective_context_is_not_found() {
        // Arrange
        let engine = engine();
        let mut instance = bare(&engine, GUEST);

        // Act
        let result = outcome(proxy_get_shared_data(
            instance.store_mut(),
            KEY,
            1,
            RETURN_DATA,
            RETURN_SIZE,
            RETURN_CAS,
        ));

        // Assert
        assert_eq!(result, Status::NotFound);
    }

    #[test]
    fn every_pointer_is_checked_before_the_services_are_asked() {
        // Arrange
        let engine = engine();
        let recording = Arc::new(RecordingServices::new());
        let (mut instance, _) = shared_hosted(&engine, GUEST, recording.clone());
        seed(&mut instance);

        // Act
        let results = [
            outcome(proxy_get_shared_data(
                instance.store_mut(),
                PAST_END,
                4,
                RETURN_DATA,
                RETURN_SIZE,
                RETURN_CAS,
            )),
            outcome(proxy_get_shared_data(
                instance.store_mut(),
                KEY,
                1,
                PAST_END,
                RETURN_SIZE,
                RETURN_CAS,
            )),
            outcome(proxy_get_shared_data(
                instance.store_mut(),
                KEY,
                1,
                RETURN_DATA,
                PAST_END,
                RETURN_CAS,
            )),
            outcome(proxy_get_shared_data(
                instance.store_mut(),
                KEY,
                1,
                RETURN_DATA,
                RETURN_SIZE,
                PAST_END,
            )),
            outcome(proxy_set_shared_data(
                instance.store_mut(),
                PAST_END,
                4,
                VALUE,
                1,
                0,
            )),
            outcome(proxy_set_shared_data(
                instance.store_mut(),
                KEY,
                1,
                PAST_END,
                4,
                0,
            )),
        ];

        // Assert
        assert_eq!(results, [Status::InvalidMemoryAccess; 6]);
        assert!(recording.calls().is_empty());
        assert_eq!(allocator_calls(&mut instance), 0);
        let cas = instance
            .memory()
            .unwrap()
            .read_u32(GuestPtr::from_address(2008))
            .unwrap();
        assert_eq!(cas, SENTINEL);
    }

    #[test]
    fn a_body_under_a_refused_root_is_not_found() {
        // Arrange
        let engine = engine();
        let shared: Arc<dyn SharedServices> = Arc::new(InMemoryStore::new());
        let (mut instance, root) = shared_hosted(&engine, GUEST, shared);
        instance.state_mut().abi_mut().contexts_mut().reject(root);

        // Act
        let result = set(&mut instance, b"k", b"v", 0);

        // Assert
        assert_eq!(result, Status::NotFound);
    }

    #[test]
    fn the_shared_services_replaced_after_construction_are_the_ones_a_guest_reads() {
        // Arrange
        let engine = engine();
        let replacement = Arc::new(InMemoryStore::new());
        let call = Invocation::new(GuestId::next(), ContextId::try_from(1).unwrap());
        replacement
            .set_shared_data(call, VM_ID, b"k", b"from the replacement", None)
            .unwrap();
        let (mut instance, _) = shared_hosted(&engine, GUEST, Arc::new(InMemoryStore::new()));
        let services = instance
            .state()
            .abi()
            .services()
            .clone()
            .with_shared(replacement);
        *instance.state_mut().abi_mut().services_mut() = services;

        // Act
        let found = get(&mut instance, b"k");

        // Assert
        assert_eq!(found, Status::Ok);
        assert_eq!(value_and_cas(&mut instance).0, b"from the replacement");
    }

    #[test]
    fn a_key_of_one_vm_is_not_visible_to_a_guest_of_another() {
        // Arrange
        // The existing test drives the store directly. This one drives two
        // guests, so it dies if the VM id is dropped from the key.
        let engine = engine();
        let store: Arc<dyn SharedServices> = Arc::new(InMemoryStore::new());
        let (mut mine, _) = shared_hosted(&engine, GUEST, Arc::clone(&store));
        let (mut theirs, _) = shared_hosted(&engine, GUEST, Arc::clone(&store));
        *theirs.state_mut().abi_mut().services_mut() = theirs
            .state()
            .abi()
            .services()
            .clone()
            .with_vm_id(b"other-vm".to_vec());
        set(&mut mine, b"k", b"mine", 0);

        // Act
        let found = get(&mut theirs, b"k");

        // Assert
        assert_eq!(found, Status::NotFound);
    }

    #[test]
    fn a_shared_data_key_above_the_byte_bound_is_refused() {
        // Arrange
        let engine = engine();
        let recording = Arc::new(RecordingServices::new());
        let limits = Limits::default().with_max_name_bytes(4);
        let (mut instance, _) =
            shared_hosted_with_limits(&engine, GUEST, recording.clone(), &limits);

        // Act
        let result = set(&mut instance, b"fives", b"v", 0);

        // Assert
        assert_eq!(result, Status::InternalFailure);
        assert!(
            recording.calls().is_empty(),
            "the service must not see a key the crate refuses"
        );
    }

    #[test]
    fn a_read_of_a_key_above_the_byte_bound_finds_nothing() {
        // Arrange
        let engine = engine();
        let recording = Arc::new(RecordingServices::new());
        let limits = Limits::default().with_max_name_bytes(4);
        let (mut instance, _) =
            shared_hosted_with_limits(&engine, GUEST, recording.clone(), &limits);

        // Act
        let result = get(&mut instance, b"fives");

        // Assert
        assert_eq!(
            result,
            Status::NotFound,
            "a guest of the Rust SDK reads this as a miss and does not stop"
        );
        assert!(
            recording.calls().is_empty(),
            "the service must not see a key the crate refuses"
        );
    }

    #[test]
    fn a_key_at_the_byte_bound_is_written() {
        // Arrange
        let engine = engine();
        let recording = Arc::new(RecordingServices::new());
        let limits = Limits::default().with_max_name_bytes(4);
        let (mut instance, _) =
            shared_hosted_with_limits(&engine, GUEST, recording.clone(), &limits);

        // Act
        let result = set(&mut instance, b"four", b"v", 0);

        // Assert
        assert_eq!(result, Status::Ok);
        assert_eq!(recording.calls().len(), 1);
    }
}
