//! `proxy_log`.

use wasmtime::AsContextMut;

use crate::abi::v0_2_1::host_functions::Failure;
use crate::abi::v0_2_1::types::LogLevel;
use crate::runtime::{GuestSlice, HostState, split};

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
    state.services().log().log(level, message);
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::abi::v0_2_1::test_support::{outcome, status};
    use crate::abi::v0_2_1::types::Status;
    use crate::runtime::test_support::{RecordingSink, engine, instance_with_sink};

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
}
