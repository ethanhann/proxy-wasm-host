//! `proxy_set_tick_period_milliseconds`.

use crate::abi::v0_2_1::AbiAccess;
use std::time::Duration;

use wasmtime::AsContextMut;

use crate::abi::v0_2_1::host_functions::Failure;
use crate::abi::v0_2_1::host_functions::call::context;
use crate::abi::v0_2_1::types::Status;
use crate::runtime::HostState;

pub(super) fn proxy_set_tick_period_milliseconds(
    ctx: &mut impl AsContextMut<Data = HostState>,
    tick_period: i32,
) -> Result<(), Failure> {
    let milliseconds = tick_period.cast_unsigned();
    let mut ctx = ctx.as_context_mut();
    let state = ctx.data_mut();
    let effective = context(state, Status::BadArgument)?;
    let period = (milliseconds > 0).then(|| Duration::from_millis(u64::from(milliseconds)));
    if state
        .abi_mut()
        .contexts_mut()
        .set_tick_period(effective, period)
    {
        Ok(())
    } else {
        Err(Status::BadArgument.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::v0_2_1::test_support::{bare, outcome, status, unhosted};
    use crate::runtime::test_support::engine;

    const GUEST: &str = r#"(module
        (import "env" "proxy_set_tick_period_milliseconds" (func $tick (param i32) (result i32)))
        (memory (export "memory") 1)
        (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 1024)
        (func (export "tick") (param i32) (result i32) local.get 0 call $tick))"#;

    #[test]
    fn a_period_is_recorded_on_the_root_of_the_effective_context() {
        // Arrange
        let engine = engine();
        let (mut instance, root) = unhosted(&engine, GUEST);

        // Act
        let result = instance.call::<i32, i32>("tick", 250).map(status);

        // Assert
        assert_eq!(result.unwrap(), Status::Ok);
        assert_eq!(
            instance.state().abi().contexts().tick_period(root),
            Some(Duration::from_millis(250))
        );
    }

    #[test]
    fn a_period_of_zero_clears_a_recorded_one() {
        // Arrange
        let engine = engine();
        let (mut instance, root) = unhosted(&engine, GUEST);
        instance.call::<i32, i32>("tick", 250).unwrap();

        // Act
        let result = instance.call::<i32, i32>("tick", 0).map(status);

        // Assert
        assert_eq!(result.unwrap(), Status::Ok);
        assert_eq!(instance.state().abi().contexts().tick_period(root), None);
    }

    #[test]
    fn the_largest_period_is_read_as_unsigned() {
        // Arrange
        let engine = engine();
        let (mut instance, root) = unhosted(&engine, GUEST);

        // Act
        let result = instance.call::<i32, i32>("tick", -1).map(status);

        // Assert
        assert_eq!(result.unwrap(), Status::Ok);
        assert_eq!(
            instance.state().abi().contexts().tick_period(root),
            Some(Duration::from_millis(u64::from(u32::MAX)))
        );
    }

    #[test]
    fn a_period_without_an_effective_context_is_a_bad_argument() {
        // Arrange
        let engine = engine();
        let mut instance = bare(&engine, GUEST);

        // Act
        let result = outcome(proxy_set_tick_period_milliseconds(
            instance.store_mut(),
            250,
        ));

        // Assert
        assert_eq!(result, Status::BadArgument);
    }

    #[test]
    fn a_period_under_a_refused_root_is_a_bad_argument() {
        // Arrange
        let engine = engine();
        let (mut instance, root) = unhosted(&engine, GUEST);
        instance.state_mut().abi_mut().contexts_mut().reject(root);

        // Act
        let result = outcome(proxy_set_tick_period_milliseconds(
            instance.store_mut(),
            250,
        ));

        // Assert
        assert_eq!(result, Status::BadArgument);
        assert_eq!(instance.state().abi().contexts().tick_period(root), None);
    }
}
