//! `proxy_log` and `proxy_get_log_level`.

use wasmtime::AsContextMut;

use crate::abi::v0_2_1::AbiAccess;
use crate::abi::v0_2_1::host_functions::Failure;
use crate::abi::v0_2_1::types::LogLevel;
use crate::runtime::{GuestPtr, GuestSlice, HostState, split};

pub(super) fn proxy_log(
    ctx: &mut impl AsContextMut<Data = HostState>,
    log_level: i32,
    message_data: i32,
    message_size: i32,
) -> Result<(), Failure> {
    let level = LogLevel::try_from(log_level)?;
    let slice = GuestSlice::try_from((message_data, message_size))?;
    let (memory, state) = split(ctx)?;
    let message = memory.read(slice)?;
    state.abi().services().log().log(level, message);
    Ok(())
}

pub(super) fn proxy_get_log_level(
    ctx: &mut impl AsContextMut<Data = HostState>,
    return_log_level: i32,
) -> Result<(), Failure> {
    let return_log_level = GuestPtr::try_from(return_log_level)?;
    let (mut memory, state) = split(ctx)?;
    let level = i32::from(state.abi().services().log_level()).cast_unsigned();
    memory.write_u32(return_log_level, level)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::abi::v0_2_1::test_support::{
        RecordingSink, engine, instance_with_sink, outcome, status,
    };
    use crate::abi::v0_2_1::types::Status;

    const LOGGER: &str = r#"(module
        (import "env" "proxy_log" (func $log (param i32 i32 i32) (result i32)))
        (memory (export "memory") 1)
        (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 1024)
        (func (export "log") (param i32 i32 i32) (result i32)
            local.get 0 local.get 1 local.get 2 call $log)
        (data (i32.const 16) "hello"))"#;

    #[test]
    fn a_message_is_logged_at_its_level() {
        // Arrange
        let engine = engine();
        let sink = Arc::new(RecordingSink::default());
        let mut instance = instance_with_sink(&engine, LOGGER, Arc::clone(&sink)).unwrap();

        // Act
        let result = instance
            .call::<(i32, i32, i32), i32>("log", (2, 16, 5))
            .map(status);

        // Assert
        assert_eq!(result.unwrap(), Status::Ok);
        assert_eq!(sink.entries(), vec![(LogLevel::Info, b"hello".to_vec())]);
    }

    #[test]
    fn an_empty_message_is_logged_as_an_empty_entry() {
        // Arrange
        let engine = engine();
        let sink = Arc::new(RecordingSink::default());
        let mut instance = instance_with_sink(&engine, LOGGER, Arc::clone(&sink)).unwrap();

        // Act
        let result = instance
            .call::<(i32, i32, i32), i32>("log", (4, 16, 0))
            .map(status);

        // Assert
        assert_eq!(result.unwrap(), Status::Ok);
        assert_eq!(sink.entries(), vec![(LogLevel::Error, Vec::new())]);
    }

    #[test]
    fn a_bad_level_and_a_bad_address_log_nothing() {
        // Arrange
        let engine = engine();
        let sink = Arc::new(RecordingSink::default());
        let mut instance = instance_with_sink(&engine, LOGGER, Arc::clone(&sink)).unwrap();

        // Act
        let results = (
            outcome(proxy_log(instance.store_mut(), 9, 16, 5)),
            outcome(proxy_log(instance.store_mut(), 2, 65_530, 10)),
            outcome(proxy_log(instance.store_mut(), 2, -1, 5)),
        );

        // Assert
        assert_eq!(
            results,
            (
                Status::BadArgument,
                Status::InvalidMemoryAccess,
                Status::InvalidMemoryAccess
            )
        );
        assert!(sink.entries().is_empty());
    }

    const LEVEL_GUEST: &str = r#"(module
        (import "env" "proxy_get_log_level" (func $level (param i32) (result i32)))
        (memory (export "memory") 1)
        (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 1024)
        (func (export "level") (param i32) (result i32) local.get 0 call $level))"#;

    #[test]
    fn the_level_in_the_services_is_written() {
        // Arrange
        let engine = engine();
        let sink = Arc::new(RecordingSink::default());
        let mut instance = instance_with_sink(&engine, LEVEL_GUEST, sink).unwrap();

        // Act
        let result = instance.call::<i32, i32>("level", 16).map(status);

        // Assert
        assert_eq!(result.unwrap(), Status::Ok);
        assert_eq!(read_level(&mut instance, 16), LogLevel::Info);
    }

    #[test]
    fn a_changed_level_is_written() {
        // Arrange
        let engine = engine();
        let sink = Arc::new(RecordingSink::default());
        let mut instance = instance_with_sink(&engine, LEVEL_GUEST, sink).unwrap();
        instance
            .state_mut()
            .abi_mut()
            .services_mut()
            .set_log_level(LogLevel::Critical);

        // Act
        let result = instance.call::<i32, i32>("level", 16).map(status);

        // Assert
        assert_eq!(result.unwrap(), Status::Ok);
        assert_eq!(read_level(&mut instance, 16), LogLevel::Critical);
    }

    #[test]
    fn a_level_return_pointer_past_memory_is_an_invalid_access() {
        // Arrange
        let engine = engine();
        let sink = Arc::new(RecordingSink::default());
        let mut instance = instance_with_sink(&engine, LEVEL_GUEST, sink).unwrap();

        // Act
        let result = outcome(proxy_get_log_level(instance.store_mut(), 65_534));

        // Assert
        assert_eq!(result, Status::InvalidMemoryAccess);
    }

    fn read_level(instance: &mut crate::runtime::Instance, at: u32) -> LogLevel {
        let value = instance
            .memory()
            .unwrap()
            .read_u32(crate::runtime::GuestPtr::from_address(at))
            .unwrap();
        LogLevel::try_from(value.cast_signed()).unwrap()
    }
}
