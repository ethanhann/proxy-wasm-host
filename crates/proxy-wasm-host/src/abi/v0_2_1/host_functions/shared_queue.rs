//! The four shared queue functions.
//!
//! A register and a resolve record the identifier the guest obtained, and an
//! enqueue or a dequeue of an identifier it never obtained reports
//! `NOT_FOUND`.
//! A queue identifier is a small number that another VM can guess, and the
//! ABI gives the guest no VM id on those two calls, so the crate checks it
//! here rather than asking every implementation to.

use wasmtime::AsContextMut;

use crate::abi::v0_2_1::AbiAccess;
use crate::abi::v0_2_1::QueueId;
use crate::abi::v0_2_1::host_functions::Failure;
use crate::abi::v0_2_1::host_functions::bounds::{within_name_bytes, within_shared_names};
use crate::abi::v0_2_1::host_functions::call::{from_embedder, settle, with_shared};
use crate::abi::v0_2_1::types::Status;
use crate::runtime::{GuestPtr, GuestSlice, HostState, split, write_return};

pub(super) fn proxy_register_shared_queue(
    ctx: &mut impl AsContextMut<Data = HostState>,
    name_data: i32,
    name_size: i32,
    return_queue_id: i32,
) -> Result<(), Failure> {
    let name = GuestSlice::try_from((name_data, name_size))?;
    let return_queue_id = GuestPtr::try_from(return_queue_id)?;
    let (memory, state) = split(ctx)?;
    memory.read_u32(return_queue_id)?;
    let name = memory.read(name)?;
    let (call, shared) = with_shared(state, Status::NotFound)?;
    within_name_bytes(state, name, Status::InternalFailure)?;
    within_shared_names(state)?;
    let vm_id = state.abi().services().vm_id();
    let queue = from_embedder(
        "register_shared_queue",
        shared.register_shared_queue(call, vm_id, name),
    )?;
    state.abi_mut().grant_queue(queue);
    if let Some(root) = state.abi().contexts().root_of(call.context) {
        state.abi_mut().register_queue(queue, root, name);
    }
    let (mut memory, _) = split(ctx)?;
    memory.write_u32(return_queue_id, queue.get())?;
    Ok(())
}

pub(super) fn proxy_resolve_shared_queue(
    ctx: &mut impl AsContextMut<Data = HostState>,
    vm_id_data: i32,
    vm_id_size: i32,
    name_data: i32,
    name_size: i32,
    return_queue_id: i32,
) -> Result<(), Failure> {
    let vm_id = GuestSlice::try_from((vm_id_data, vm_id_size))?;
    let name = GuestSlice::try_from((name_data, name_size))?;
    let return_queue_id = GuestPtr::try_from(return_queue_id)?;
    let (memory, state) = split(ctx)?;
    memory.read_u32(return_queue_id)?;
    let vm_id = memory.read(vm_id)?;
    let name = memory.read(name)?;
    let (call, shared) = with_shared(state, Status::NotFound)?;
    within_name_bytes(state, name, Status::InternalFailure)?;
    within_shared_names(state)?;
    // The C++ host reads an empty VM id as the VM of the caller.
    let vm_id = if vm_id.is_empty() {
        state.abi().services().vm_id()
    } else {
        vm_id
    };
    let queue = from_embedder(
        "resolve_shared_queue",
        shared.resolve_shared_queue(call, vm_id, name),
    )?;
    state.abi_mut().grant_queue(queue);
    let (mut memory, _) = split(ctx)?;
    memory.write_u32(return_queue_id, queue.get())?;
    Ok(())
}

pub(super) fn proxy_enqueue_shared_queue(
    ctx: &mut impl AsContextMut<Data = HostState>,
    queue_id: i32,
    value_data: i32,
    value_size: i32,
) -> Result<(), Failure> {
    let queue = QueueId::try_from(queue_id).map_err(|_| Status::NotFound)?;
    let value = GuestSlice::try_from((value_data, value_size))?;
    let (memory, state) = split(ctx)?;
    let value = memory.read(value)?;
    settle(state);
    if !state.abi().holds_queue(queue) {
        return Err(Status::NotFound.into());
    }
    let (call, shared) = with_shared(state, Status::NotFound)?;
    from_embedder(
        "enqueue_shared_queue",
        shared.enqueue_shared_queue(call, queue, value),
    )
}

pub(super) fn proxy_dequeue_shared_queue(
    ctx: &mut impl AsContextMut<Data = HostState>,
    queue_id: i32,
    return_value_data: i32,
    return_value_size: i32,
) -> Result<(), Failure> {
    let queue = QueueId::try_from(queue_id).map_err(|_| Status::NotFound)?;
    let data_ptr = GuestPtr::try_from(return_value_data)?;
    let size_ptr = GuestPtr::try_from(return_value_size)?;
    let (memory, state) = split(ctx)?;
    memory.read_u32(data_ptr)?;
    memory.read_u32(size_ptr)?;
    settle(state);
    if !state.abi().holds_queue(queue) {
        return Err(Status::NotFound.into());
    }
    let (call, shared) = with_shared(state, Status::NotFound)?;
    let value = from_embedder(
        "dequeue_shared_queue",
        shared.dequeue_shared_queue(call, queue),
    )?;
    write_return(ctx, &value, data_ptr, size_ptr)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::abi::v0_2_1::test_support::services::{RecordingServices, SharedCall};
    use crate::abi::v0_2_1::test_support::{
        VM_ID, bare, engine, outcome, returned, shared_hosted, shared_hosted_with_limits, status,
        write,
    };
    use crate::abi::v0_2_1::{ContextId, GuestId, InMemoryStore, Invocation, SharedServices};
    use crate::runtime::{GuestPtr, Instance, Limits};

    const NAME: i32 = 1024;
    const VM: i32 = 1100;
    const VALUE: i32 = 1200;
    const RETURN_ID: i32 = 2000;
    const RETURN_DATA: i32 = 2004;
    const RETURN_SIZE: i32 = 2008;
    const PAST_END: i32 = 65_534;

    const GUEST: &str = r#"(module
        (import "env" "proxy_register_shared_queue" (func $register (param i32 i32 i32) (result i32)))
        (import "env" "proxy_resolve_shared_queue" (func $resolve (param i32 i32 i32 i32 i32) (result i32)))
        (import "env" "proxy_enqueue_shared_queue" (func $enqueue (param i32 i32 i32) (result i32)))
        (import "env" "proxy_dequeue_shared_queue" (func $dequeue (param i32 i32 i32) (result i32)))
        (memory (export "memory") 1)
        (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 4096)
        (func (export "register") (param i32 i32 i32) (result i32)
            local.get 0 local.get 1 local.get 2 call $register)
        (func (export "resolve") (param i32 i32 i32 i32 i32) (result i32)
            local.get 0 local.get 1 local.get 2 local.get 3 local.get 4 call $resolve)
        (func (export "enqueue") (param i32 i32 i32) (result i32)
            local.get 0 local.get 1 local.get 2 call $enqueue)
        (func (export "dequeue") (param i32 i32 i32) (result i32)
            local.get 0 local.get 1 local.get 2 call $dequeue))"#;

    fn register(instance: &mut Instance, name: &[u8]) -> Status {
        let (_, len) = write(instance, NAME, name);
        status(
            instance
                .call::<(i32, i32, i32), i32>("register", (NAME, len, RETURN_ID))
                .unwrap(),
        )
    }

    fn returned_id(instance: &mut Instance) -> u32 {
        instance
            .memory()
            .unwrap()
            .read_u32(GuestPtr::from_address(2000))
            .unwrap()
    }

    fn dequeued(instance: &mut Instance) -> Vec<u8> {
        returned(instance, 2004, 2008)
    }

    #[test]
    fn a_registration_names_this_vm_and_returns_an_identifier() {
        // Arrange
        let engine = engine();
        let recording = Arc::new(RecordingServices::new());
        let (mut instance, _) = shared_hosted(&engine, GUEST, recording.clone());

        // Act
        let result = register(&mut instance, b"q");

        // Assert
        assert_eq!(result, Status::Ok);
        assert_eq!(returned_id(&mut instance), 1);
        assert_eq!(
            recording.calls()[0].1,
            SharedCall::Register(VM_ID.to_vec(), b"q".to_vec())
        );
    }

    #[test]
    fn a_registration_grants_the_identifier_to_this_guest() {
        // Arrange
        let engine = engine();
        let shared: Arc<dyn SharedServices> = Arc::new(InMemoryStore::new());
        let (mut instance, _) = shared_hosted(&engine, GUEST, shared);

        // Act
        let result = register(&mut instance, b"q");

        // Assert
        assert_eq!(result, Status::Ok);
        let queue = QueueId::try_from(returned_id(&mut instance)).unwrap();
        assert!(instance.state().abi().holds_queue(queue));
    }

    #[test]
    fn a_queue_this_guest_never_obtained_is_not_found() {
        // Arrange
        let engine = engine();
        let shared: Arc<dyn SharedServices> = Arc::new(InMemoryStore::new());
        let other = Invocation::new(GuestId::next(), ContextId::try_from(1).unwrap());
        let theirs = shared
            .register_shared_queue(other, b"vm-2", b"private")
            .unwrap();
        shared
            .enqueue_shared_queue(other, theirs, b"secret")
            .unwrap();
        let (mut instance, _) = shared_hosted(&engine, GUEST, shared);

        // Act
        let results = [
            status(
                instance
                    .call::<(i32, i32, i32), i32>(
                        "dequeue",
                        (theirs.get().cast_signed(), RETURN_DATA, RETURN_SIZE),
                    )
                    .unwrap(),
            ),
            status(
                instance
                    .call::<(i32, i32, i32), i32>("enqueue", (theirs.get().cast_signed(), VALUE, 1))
                    .unwrap(),
            ),
        ];

        // Assert
        assert_eq!(results, [Status::NotFound; 2]);
    }

    #[test]
    fn a_resolve_grants_the_identifier_as_well() {
        // Arrange
        let engine = engine();
        let shared: Arc<dyn SharedServices> = Arc::new(InMemoryStore::new());
        let other = Invocation::new(GuestId::next(), ContextId::try_from(1).unwrap());
        let theirs = shared
            .register_shared_queue(other, VM_ID, b"shared")
            .unwrap();
        let (mut instance, _) = shared_hosted(&engine, GUEST, shared);
        let (_, vm_len) = write(&mut instance, VM, VM_ID);
        let (_, name_len) = write(&mut instance, NAME, b"shared");

        // Act
        let result = status(
            instance
                .call::<(i32, i32, i32, i32, i32), i32>(
                    "resolve",
                    (VM, vm_len, NAME, name_len, RETURN_ID),
                )
                .unwrap(),
        );

        // Assert
        assert_eq!(result, Status::Ok);
        assert!(instance.state().abi().holds_queue(theirs));
    }

    #[test]
    fn a_resolve_finds_a_queue_of_the_named_vm() {
        // Arrange
        let engine = engine();
        let shared: Arc<dyn SharedServices> = Arc::new(InMemoryStore::new());
        let (mut instance, _) = shared_hosted(&engine, GUEST, shared);
        register(&mut instance, b"q");
        let (_, vm_len) = write(&mut instance, VM, VM_ID);
        let (_, name_len) = write(&mut instance, NAME, b"q");

        // Act
        let result = status(
            instance
                .call::<(i32, i32, i32, i32, i32), i32>(
                    "resolve",
                    (VM, vm_len, NAME, name_len, RETURN_ID),
                )
                .unwrap(),
        );

        // Assert
        assert_eq!(result, Status::Ok);
        assert_eq!(returned_id(&mut instance), 1);
    }

    #[test]
    fn a_resolve_with_an_empty_vm_id_finds_a_queue_of_the_callers_vm() {
        // Arrange
        let engine = engine();
        let recording = Arc::new(RecordingServices::new());
        let (mut instance, _) = shared_hosted(&engine, GUEST, recording.clone());
        register(&mut instance, b"q");
        let (_, name_len) = write(&mut instance, NAME, b"q");

        // Act
        let result = status(
            instance
                .call::<(i32, i32, i32, i32, i32), i32>(
                    "resolve",
                    (VM, 0, NAME, name_len, RETURN_ID),
                )
                .unwrap(),
        );

        // Assert
        assert_eq!(
            result,
            Status::Ok,
            "an empty VM id means the caller's own VM"
        );
        assert_eq!(returned_id(&mut instance), 1);
        assert_eq!(
            recording.calls()[1].1,
            SharedCall::Resolve(VM_ID.to_vec(), b"q".to_vec())
        );
    }

    #[test]
    fn a_resolve_of_a_name_no_vm_registered_is_not_found() {
        // Arrange
        let engine = engine();
        let shared: Arc<dyn SharedServices> = Arc::new(InMemoryStore::new());
        let (mut instance, _) = shared_hosted(&engine, GUEST, shared);
        let (_, vm_len) = write(&mut instance, VM, VM_ID);
        let (_, name_len) = write(&mut instance, NAME, b"q");

        // Act
        let result = status(
            instance
                .call::<(i32, i32, i32, i32, i32), i32>(
                    "resolve",
                    (VM, vm_len, NAME, name_len, RETURN_ID),
                )
                .unwrap(),
        );

        // Assert
        assert_eq!(result, Status::NotFound);
    }

    #[test]
    fn an_item_goes_in_and_comes_back_out() {
        // Arrange
        let engine = engine();
        let shared: Arc<dyn SharedServices> = Arc::new(InMemoryStore::new());
        let (mut instance, _) = shared_hosted(&engine, GUEST, shared);
        register(&mut instance, b"q");
        let id = returned_id(&mut instance).cast_signed();
        let (_, len) = write(&mut instance, VALUE, b"item");

        status(
            instance
                .call::<(i32, i32, i32), i32>("enqueue", (id, VALUE, len))
                .unwrap(),
        );

        // Act
        let taken = status(
            instance
                .call::<(i32, i32, i32), i32>("dequeue", (id, RETURN_DATA, RETURN_SIZE))
                .unwrap(),
        );

        // Assert
        assert_eq!(taken, Status::Ok);
        assert_eq!(dequeued(&mut instance), b"item");
    }

    #[test]
    fn a_dequeue_of_an_empty_queue_is_empty_and_of_an_unknown_one_is_not_found() {
        // Arrange
        let engine = engine();
        let shared: Arc<dyn SharedServices> = Arc::new(InMemoryStore::new());
        let (mut instance, _) = shared_hosted(&engine, GUEST, shared);
        register(&mut instance, b"q");
        let id = returned_id(&mut instance).cast_signed();

        // Act
        let results = [
            status(
                instance
                    .call::<(i32, i32, i32), i32>("dequeue", (id, RETURN_DATA, RETURN_SIZE))
                    .unwrap(),
            ),
            status(
                instance
                    .call::<(i32, i32, i32), i32>("dequeue", (9, RETURN_DATA, RETURN_SIZE))
                    .unwrap(),
            ),
        ];

        // Assert
        assert_eq!(results, [Status::Empty, Status::NotFound]);
    }

    #[test]
    fn a_queue_identifier_of_zero_is_not_found() {
        // Arrange
        let engine = engine();
        let shared: Arc<dyn SharedServices> = Arc::new(InMemoryStore::new());
        let (mut instance, _) = shared_hosted(&engine, GUEST, shared);

        // Act
        let result = outcome(proxy_enqueue_shared_queue(
            instance.store_mut(),
            0,
            VALUE,
            1,
        ));

        // Assert
        assert_eq!(result, Status::NotFound);
    }

    #[test]
    fn a_body_with_no_effective_context_is_not_found() {
        // Arrange
        let engine = engine();
        let mut instance = bare(&engine, GUEST);

        // Act
        let result = outcome(proxy_register_shared_queue(
            instance.store_mut(),
            NAME,
            1,
            RETURN_ID,
        ));

        // Assert
        assert_eq!(result, Status::NotFound);
    }

    #[test]
    fn a_long_name_with_no_effective_context_is_not_found() {
        // Arrange
        let engine = engine();
        let mut instance = bare(&engine, GUEST);

        // Act
        let result = outcome(proxy_register_shared_queue(
            instance.store_mut(),
            NAME,
            5000,
            RETURN_ID,
        ));

        // Assert
        assert_eq!(
            result,
            Status::NotFound,
            "the absence of a context answers before a bound"
        );
    }

    #[test]
    fn a_guest_at_its_share_registers_again_after_the_store_is_replaced() {
        // Arrange
        let engine = engine();
        let first: Arc<dyn SharedServices> = Arc::new(InMemoryStore::new());
        let second: Arc<dyn SharedServices> = Arc::new(InMemoryStore::new());
        let limits = Limits::default().with_max_shared_names(1);
        let (mut instance, _) = shared_hosted_with_limits(&engine, GUEST, first, &limits);
        assert_eq!(register(&mut instance, b"mine"), Status::Ok);
        let replacement = instance
            .state()
            .abi()
            .services()
            .clone()
            .with_shared(second);
        *instance.state_mut().abi_mut().services_mut() = replacement;

        // Act
        let result = register(&mut instance, b"mine");

        // Assert
        assert_eq!(
            result,
            Status::Ok,
            "a new store starts the guest with no grants"
        );
    }

    #[test]
    fn every_pointer_is_checked_before_the_services_are_asked() {
        // Arrange
        let engine = engine();
        let recording = Arc::new(RecordingServices::new());
        let (mut instance, _) = shared_hosted(&engine, GUEST, recording.clone());

        // Act
        let results = [
            outcome(proxy_register_shared_queue(
                instance.store_mut(),
                PAST_END,
                4,
                RETURN_ID,
            )),
            outcome(proxy_register_shared_queue(
                instance.store_mut(),
                NAME,
                1,
                PAST_END,
            )),
            outcome(proxy_resolve_shared_queue(
                instance.store_mut(),
                PAST_END,
                4,
                NAME,
                1,
                RETURN_ID,
            )),
            outcome(proxy_enqueue_shared_queue(
                instance.store_mut(),
                1,
                PAST_END,
                4,
            )),
            outcome(proxy_dequeue_shared_queue(
                instance.store_mut(),
                1,
                PAST_END,
                RETURN_SIZE,
            )),
        ];

        // Assert
        assert_eq!(results, [Status::InvalidMemoryAccess; 5]);
        assert!(recording.calls().is_empty());
    }

    #[test]
    fn a_body_under_a_refused_root_is_not_found() {
        // Arrange
        let engine = engine();
        let shared: Arc<dyn SharedServices> = Arc::new(InMemoryStore::new());
        let (mut instance, root) = shared_hosted(&engine, GUEST, shared);
        instance.state_mut().abi_mut().contexts_mut().reject(root);

        // Act
        let result = register(&mut instance, b"q");

        // Assert
        assert_eq!(result, Status::NotFound);
    }

    #[test]
    fn a_queue_one_guest_registered_is_not_granted_to_another_of_the_same_vm() {
        // Arrange
        // The grant is per instance, so a second guest of the same VM that
        // never registered the queue cannot reach it by passing its identifier. This dies
        // if the grant set is shared between instances.
        let engine = engine();
        let shared: Arc<dyn SharedServices> = Arc::new(InMemoryStore::new());
        let (mut mine, _) = shared_hosted(&engine, GUEST, Arc::clone(&shared));
        let (mut theirs, _) = shared_hosted(&engine, GUEST, Arc::clone(&shared));
        register(&mut mine, b"shared-name");
        let queue = returned_id(&mut mine).cast_signed();

        // Act
        let found = status(
            theirs
                .call::<(i32, i32, i32), i32>("dequeue", (queue, RETURN_DATA, RETURN_SIZE))
                .unwrap(),
        );

        // Assert
        assert_eq!(found, Status::NotFound);
        assert!(
            mine.state()
                .abi()
                .holds_queue(QueueId::try_from(queue).unwrap())
        );
    }

    #[test]
    fn a_grant_does_not_survive_a_replacement_of_the_shared_services() {
        // Arrange
        // Both stores hand out small numbers from their own counter, so the
        // identifier this guest was granted refers to a queue of another VM
        // inside the replacement.
        let engine = engine();
        let first: Arc<dyn SharedServices> = Arc::new(InMemoryStore::new());
        let second: Arc<dyn SharedServices> = Arc::new(InMemoryStore::new());
        let other = Invocation::new(GuestId::next(), ContextId::try_from(1).unwrap());
        let theirs = second
            .register_shared_queue(other, b"other-vm", b"private")
            .unwrap();
        second
            .enqueue_shared_queue(other, theirs, b"a secret of the other vm")
            .unwrap();
        let (mut instance, _) = shared_hosted(&engine, GUEST, Arc::clone(&first));
        register(&mut instance, b"mine");
        let granted = returned_id(&mut instance);
        assert_eq!(
            granted,
            theirs.get(),
            "both stores start their counter at one"
        );
        let replacement = instance
            .state()
            .abi()
            .services()
            .clone()
            .with_shared(second);
        *instance.state_mut().abi_mut().services_mut() = replacement;

        // Act
        let found = status(
            instance
                .call::<(i32, i32, i32), i32>(
                    "dequeue",
                    (granted.cast_signed(), RETURN_DATA, RETURN_SIZE),
                )
                .unwrap(),
        );

        // Assert
        assert_eq!(found, Status::NotFound);
    }

    #[test]
    fn a_guest_at_its_share_of_names_is_refused_a_new_queue() {
        // Arrange
        let engine = engine();
        let recording = Arc::new(RecordingServices::new());
        let limits = Limits::default().with_max_shared_names(2);
        let (mut instance, _) =
            shared_hosted_with_limits(&engine, GUEST, recording.clone(), &limits);
        assert_eq!(register(&mut instance, b"first"), Status::Ok);
        assert_eq!(register(&mut instance, b"second"), Status::Ok);

        // Act
        let result = register(&mut instance, b"third");

        // Assert
        assert_eq!(result, Status::InternalFailure);
        assert_eq!(
            recording.calls().len(),
            2,
            "the store must not create a queue the crate refuses"
        );
    }

    #[test]
    fn a_queue_a_guest_already_holds_does_not_raise_its_count() {
        // Arrange
        let engine = engine();
        let shared: Arc<dyn SharedServices> = Arc::new(InMemoryStore::new());
        let limits = Limits::default().with_max_shared_names(2);
        let (mut instance, _) = shared_hosted_with_limits(&engine, GUEST, shared, &limits);
        assert_eq!(register(&mut instance, b"first"), Status::Ok);
        assert_eq!(register(&mut instance, b"first"), Status::Ok);

        // Act
        let result = register(&mut instance, b"second");

        // Assert
        assert_eq!(
            result,
            Status::Ok,
            "the count is of identifiers, so the repeat spent nothing"
        );
    }

    #[test]
    fn a_resolved_queue_spends_a_share_of_the_names() {
        // Arrange
        let engine = engine();
        let shared: Arc<dyn SharedServices> = Arc::new(InMemoryStore::new());
        let other = Invocation::new(GuestId::next(), ContextId::try_from(1).unwrap());
        shared
            .register_shared_queue(other, VM_ID, b"theirs")
            .unwrap();
        let limits = Limits::default().with_max_shared_names(1);
        let (mut instance, _) = shared_hosted_with_limits(&engine, GUEST, shared, &limits);
        let (_, vm_len) = write(&mut instance, VM, VM_ID);
        let (_, name_len) = write(&mut instance, NAME, b"theirs");
        assert_eq!(
            status(
                instance
                    .call::<(i32, i32, i32, i32, i32), i32>(
                        "resolve",
                        (VM, vm_len, NAME, name_len, RETURN_ID)
                    )
                    .unwrap()
            ),
            Status::Ok
        );

        // Act
        let result = register(&mut instance, b"mine");

        // Assert
        assert_eq!(result, Status::InternalFailure);
    }

    #[test]
    fn a_queue_name_above_the_byte_bound_is_refused() {
        // Arrange
        let engine = engine();
        let recording = Arc::new(RecordingServices::new());
        let limits = Limits::default().with_max_name_bytes(4);
        let (mut instance, _) =
            shared_hosted_with_limits(&engine, GUEST, recording.clone(), &limits);

        // Act
        let result = register(&mut instance, b"fives");

        // Assert
        assert_eq!(result, Status::InternalFailure);
        assert!(
            recording.calls().is_empty(),
            "the service must not see a name the crate refuses"
        );
    }

    #[test]
    fn a_queue_name_at_the_byte_bound_is_registered() {
        // Arrange
        let engine = engine();
        let recording = Arc::new(RecordingServices::new());
        let limits = Limits::default().with_max_name_bytes(4);
        let (mut instance, _) =
            shared_hosted_with_limits(&engine, GUEST, recording.clone(), &limits);

        // Act
        let result = register(&mut instance, b"four");

        // Assert
        assert_eq!(result, Status::Ok);
        assert_eq!(
            recording.calls()[0].1,
            SharedCall::Register(VM_ID.to_vec(), b"four".to_vec())
        );
    }
}
