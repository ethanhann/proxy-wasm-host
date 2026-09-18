//! The four metric functions.
//!
//! A metric is separated by the VM id, so two plugins that define one name
//! get two metrics.
//! A guest reaches only a metric it defined, because an identifier is a small
//! number that another VM can guess and the ABI gives the guest no VM id on
//! the three calls that name one.
//! The value is carried at its full width in every direction, which the Go
//! host does not do.

use crate::abi::v0_2_1::AbiAccess;
use wasmtime::AsContextMut;

use crate::abi::v0_2_1::MetricId;
use crate::abi::v0_2_1::host_functions::Failure;
use crate::abi::v0_2_1::host_functions::call::{from_embedder, settle, with_shared};
use crate::abi::v0_2_1::types::{MetricType, Status};
use crate::runtime::{GuestPtr, GuestSlice, HostState, split};

pub(super) fn proxy_define_metric(
    ctx: &mut impl AsContextMut<Data = HostState>,
    metric_type: i32,
    name_data: i32,
    name_size: i32,
    return_metric_id: i32,
) -> Result<(), Failure> {
    let kind = MetricType::try_from(metric_type)?;
    let name = GuestSlice::try_from((name_data, name_size))?;
    let return_metric_id = GuestPtr::try_from(return_metric_id)?;
    let (memory, state) = split(ctx)?;
    memory.read_u32(return_metric_id)?;
    let name = memory.read(name)?;
    let (call, shared) = with_shared(state, Status::NotFound)?;
    let vm_id = state.services().vm_id();
    let metric = from_embedder(
        "define_metric",
        shared.define_metric(call, vm_id, kind, name),
    )?;
    state.abi_mut().grant_metric(metric);
    let (mut memory, _) = split(ctx)?;
    memory.write_u32(return_metric_id, metric.get())?;
    Ok(())
}

pub(super) fn proxy_record_metric(
    ctx: &mut impl AsContextMut<Data = HostState>,
    metric_id: i32,
    value: i64,
) -> Result<(), Failure> {
    let metric = MetricId::try_from(metric_id).map_err(|_| Status::NotFound)?;
    let (_, state) = split(ctx)?;
    settle(state);
    if !state.abi().holds_metric(metric) {
        return Err(Status::NotFound.into());
    }
    let (call, shared) = with_shared(state, Status::NotFound)?;
    from_embedder(
        "record_metric",
        shared.record_metric(call, metric, value.cast_unsigned()),
    )
}

pub(super) fn proxy_increment_metric(
    ctx: &mut impl AsContextMut<Data = HostState>,
    metric_id: i32,
    delta: i64,
) -> Result<(), Failure> {
    let metric = MetricId::try_from(metric_id).map_err(|_| Status::NotFound)?;
    let (_, state) = split(ctx)?;
    settle(state);
    if !state.abi().holds_metric(metric) {
        return Err(Status::NotFound.into());
    }
    let (call, shared) = with_shared(state, Status::NotFound)?;
    from_embedder(
        "increment_metric",
        shared.increment_metric(call, metric, delta),
    )
}

pub(super) fn proxy_get_metric(
    ctx: &mut impl AsContextMut<Data = HostState>,
    metric_id: i32,
    return_value: i32,
) -> Result<(), Failure> {
    let metric = MetricId::try_from(metric_id).map_err(|_| Status::NotFound)?;
    let return_value = GuestPtr::try_from(return_value)?;
    let (memory, state) = split(ctx)?;
    memory.read_u64(return_value)?;
    settle(state);
    if !state.abi().holds_metric(metric) {
        return Err(Status::NotFound.into());
    }
    let (call, shared) = with_shared(state, Status::NotFound)?;
    let value = from_embedder("get_metric", shared.get_metric(call, metric))?;
    let (mut memory, _) = split(ctx)?;
    memory.write_u64(return_value, value)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::abi::v0_2_1::test_support::services::{RecordingServices, SharedCall};
    use crate::abi::v0_2_1::test_support::{VM_ID, bare, outcome, shared_hosted, status, write};
    use crate::abi::v0_2_1::{MemoryServices, SharedServices};
    use crate::runtime::Instance;
    use crate::runtime::test_support::engine;

    const NAME: i32 = 1024;
    const RETURN_ID: i32 = 2000;
    const RETURN_VALUE: i32 = 2008;
    const PAST_END: i32 = 65_534;

    const GUEST: &str = r#"(module
        (import "env" "proxy_define_metric" (func $define (param i32 i32 i32 i32) (result i32)))
        (import "env" "proxy_record_metric" (func $record (param i32 i64) (result i32)))
        (import "env" "proxy_increment_metric" (func $increment (param i32 i64) (result i32)))
        (import "env" "proxy_get_metric" (func $get (param i32 i32) (result i32)))
        (memory (export "memory") 1)
        (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 4096)
        (func (export "define") (param i32 i32 i32 i32) (result i32)
            local.get 0 local.get 1 local.get 2 local.get 3 call $define)
        (func (export "record") (param i32 i64) (result i32) local.get 0 local.get 1 call $record)
        (func (export "increment") (param i32 i64) (result i32)
            local.get 0 local.get 1 call $increment)
        (func (export "get") (param i32 i32) (result i32) local.get 0 local.get 1 call $get))"#;

    fn define(instance: &mut Instance, kind: i32, name: &[u8]) -> Status {
        let (_, len) = write(instance, NAME, name);
        status(
            instance
                .call::<(i32, i32, i32, i32), i32>("define", (kind, NAME, len, RETURN_ID))
                .unwrap(),
        )
    }

    fn defined_id(instance: &mut Instance) -> i32 {
        instance
            .memory()
            .unwrap()
            .read_u32(GuestPtr::from_address(2000))
            .unwrap()
            .cast_signed()
    }

    fn read_value(instance: &mut Instance) -> u64 {
        instance
            .memory()
            .unwrap()
            .read_u64(GuestPtr::from_address(2008))
            .unwrap()
    }

    fn counter(instance: &mut Instance) -> i32 {
        define(instance, i32::from(MetricType::Counter), b"requests");
        defined_id(instance)
    }

    #[test]
    fn a_metric_is_defined_under_this_vm_id() {
        // Arrange
        let engine = engine();
        let recording = Arc::new(RecordingServices::new());
        let (mut instance, _) = shared_hosted(&engine, GUEST, recording.clone());

        // Act
        let result = define(&mut instance, i32::from(MetricType::Counter), b"requests");

        // Assert
        assert_eq!(result, Status::Ok);
        assert_eq!(defined_id(&mut instance), 1);
        assert_eq!(
            recording.calls()[0].1,
            SharedCall::Define(VM_ID.to_vec(), MetricType::Counter, b"requests".to_vec())
        );
    }

    #[test]
    fn an_unknown_metric_type_is_a_bad_argument() {
        // Arrange
        let engine = engine();
        let recording = Arc::new(RecordingServices::new());
        let (mut instance, _) = shared_hosted(&engine, GUEST, recording.clone());

        // Act
        let result = define(&mut instance, 3, b"requests");

        // Assert
        assert_eq!(result, Status::BadArgument);
        assert!(recording.calls().is_empty());
    }

    #[test]
    fn a_value_above_the_thirty_two_bit_limit_survives_both_directions() {
        // Arrange
        let engine = engine();
        let shared: Arc<dyn SharedServices> = Arc::new(MemoryServices::new());
        let (mut instance, _) = shared_hosted(&engine, GUEST, shared);
        define(&mut instance, i32::from(MetricType::Gauge), b"bytes");
        let id = defined_id(&mut instance);
        let large = u64::from(u32::MAX) + 7;

        status(
            instance
                .call::<(i32, i64), i32>("record", (id, large.cast_signed()))
                .unwrap(),
        );

        // Act
        let read = status(
            instance
                .call::<(i32, i32), i32>("get", (id, RETURN_VALUE))
                .unwrap(),
        );

        // Assert
        assert_eq!(read, Status::Ok);
        assert_eq!(read_value(&mut instance), large);
    }

    #[test]
    fn a_counter_refuses_a_negative_delta() {
        // Arrange
        let engine = engine();
        let shared: Arc<dyn SharedServices> = Arc::new(MemoryServices::new());
        let (mut instance, _) = shared_hosted(&engine, GUEST, shared);
        let id = counter(&mut instance);

        // Act
        let result = status(
            instance
                .call::<(i32, i64), i32>("increment", (id, -1))
                .unwrap(),
        );

        // Assert
        assert_eq!(result, Status::BadArgument);
    }

    #[test]
    fn a_metric_this_guest_never_defined_is_not_found() {
        // Arrange
        let engine = engine();
        let shared: Arc<dyn SharedServices> = Arc::new(MemoryServices::new());
        let other = crate::abi::v0_2_1::HostCall::new(
            crate::abi::v0_2_1::ContextId::try_from(1).unwrap(),
            None,
        );
        let theirs = shared
            .define_metric(other, b"vm-2", MetricType::Gauge, b"secret")
            .unwrap();
        shared.record_metric(other, theirs, 4242).unwrap();
        let (mut instance, _) = shared_hosted(&engine, GUEST, shared);
        let id = theirs.get().cast_signed();

        // Act
        let results = [
            status(
                instance
                    .call::<(i32, i32), i32>("get", (id, RETURN_VALUE))
                    .unwrap(),
            ),
            status(instance.call::<(i32, i64), i32>("record", (id, 0)).unwrap()),
            status(
                instance
                    .call::<(i32, i64), i32>("increment", (id, 1))
                    .unwrap(),
            ),
        ];

        // Assert
        assert_eq!(results, [Status::NotFound; 3]);
    }

    #[test]
    fn a_metric_that_is_not_there_is_not_found() {
        // Arrange
        let engine = engine();
        let shared: Arc<dyn SharedServices> = Arc::new(MemoryServices::new());
        let (mut instance, _) = shared_hosted(&engine, GUEST, shared);

        // Act
        let results = [
            status(instance.call::<(i32, i64), i32>("record", (9, 1)).unwrap()),
            status(
                instance
                    .call::<(i32, i64), i32>("increment", (9, 1))
                    .unwrap(),
            ),
            status(
                instance
                    .call::<(i32, i32), i32>("get", (9, RETURN_VALUE))
                    .unwrap(),
            ),
        ];

        // Assert
        assert_eq!(results, [Status::NotFound; 3]);
    }

    #[test]
    fn a_metric_identifier_of_zero_is_not_found() {
        // Arrange
        let engine = engine();
        let shared: Arc<dyn SharedServices> = Arc::new(MemoryServices::new());
        let (mut instance, _) = shared_hosted(&engine, GUEST, shared);

        // Act
        let result = outcome(proxy_record_metric(instance.store_mut(), 0, 1));

        // Assert
        assert_eq!(result, Status::NotFound);
    }

    #[test]
    fn a_body_with_no_effective_context_is_not_found() {
        // Arrange
        let engine = engine();
        let mut instance = bare(&engine, GUEST);

        // Act
        let result = outcome(proxy_get_metric(instance.store_mut(), 1, RETURN_VALUE));

        // Assert
        assert_eq!(result, Status::NotFound);
    }

    #[test]
    fn every_pointer_is_checked_before_the_services_are_asked() {
        // Arrange
        let engine = engine();
        let recording = Arc::new(RecordingServices::new());
        let (mut instance, _) = shared_hosted(&engine, GUEST, recording.clone());

        // Act
        let results = [
            outcome(proxy_define_metric(
                instance.store_mut(),
                0,
                PAST_END,
                8,
                RETURN_ID,
            )),
            outcome(proxy_define_metric(
                instance.store_mut(),
                0,
                NAME,
                1,
                PAST_END,
            )),
            outcome(proxy_get_metric(instance.store_mut(), 1, PAST_END)),
        ];

        // Assert
        assert_eq!(results, [Status::InvalidMemoryAccess; 3]);
        assert!(recording.calls().is_empty());
    }

    #[test]
    fn a_body_under_a_refused_root_is_not_found() {
        // Arrange
        let engine = engine();
        let shared: Arc<dyn SharedServices> = Arc::new(MemoryServices::new());
        let (mut instance, root) = shared_hosted(&engine, GUEST, shared);
        instance.state_mut().abi_mut().contexts_mut().reject(root);

        // Act
        let result = define(&mut instance, i32::from(MetricType::Counter), b"requests");

        // Assert
        assert_eq!(result, Status::NotFound);
    }
}
