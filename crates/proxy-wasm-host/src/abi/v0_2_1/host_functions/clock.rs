//! `proxy_get_current_time_nanoseconds`.

use wasmtime::AsContextMut;

use crate::abi::v0_2_1::AbiAccess;
use crate::abi::v0_2_1::host_functions::Failure;
use crate::runtime::{GuestPtr, HostState, split};

pub(super) fn proxy_get_current_time_nanoseconds(
    ctx: &mut impl AsContextMut<Data = HostState>,
    return_time: i32,
) -> Result<(), Failure> {
    let return_time = GuestPtr::try_from(return_time)?;
    let (mut memory, state) = split(ctx)?;
    let now = state.abi().services().clock().realtime_nanos();
    memory.write_u64(return_time, now)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::abi::v0_2_1::test_support::{RecordingSink, engine, wat_bytes};
    use crate::abi::v0_2_1::test_support::{outcome, status};
    use crate::abi::v0_2_1::types::Status;
    use crate::abi::v0_2_1::{Clock, VmServices};
    use crate::runtime::{GuestPtr, Instance, Module};

    const GUEST: &str = r#"(module
        (import "env" "proxy_get_current_time_nanoseconds" (func $now (param i32) (result i32)))
        (memory (export "memory") 1)
        (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 1024)
        (func (export "now") (param i32) (result i32) local.get 0 call $now))"#;

    struct Fixed;

    impl Clock for Fixed {
        fn realtime_nanos(&self) -> u64 {
            0x0102_0304_0506_0708
        }

        fn monotonic_nanos(&self) -> u64 {
            0
        }
    }

    fn instance() -> Instance {
        let engine = engine();
        let module = Module::new(&engine, &wat_bytes(GUEST)).unwrap();
        let services =
            VmServices::new(Arc::new(RecordingSink::default())).with_clock(Arc::new(Fixed));
        crate::abi::v0_2_1::test_support::instance_with(&engine, &module, services).unwrap()
    }

    #[test]
    fn the_clock_is_written_as_eight_little_endian_bytes() {
        // Arrange
        let mut instance = instance();

        // Act
        let result = instance.call::<i32, i32>("now", 16).map(status);

        // Assert
        assert_eq!(result.unwrap(), Status::Ok);
        assert_eq!(
            instance
                .memory()
                .unwrap()
                .read_u64(GuestPtr::from_address(16))
                .unwrap(),
            0x0102_0304_0506_0708
        );
    }

    #[test]
    fn a_return_pointer_past_memory_is_an_invalid_access() {
        // Arrange
        let mut instance = instance();

        // Act
        let result = outcome(proxy_get_current_time_nanoseconds(
            instance.store_mut(),
            65_534,
        ));

        // Assert
        assert_eq!(result, Status::InvalidMemoryAccess);
    }
}
