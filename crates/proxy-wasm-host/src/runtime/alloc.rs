//! Allocation inside the guest and the writes that follow it.
//!
//! The ABI passes host data to a guest through memory the guest allocates.
//! A host function therefore allocates, then writes, then hands the guest the
//! address and the length.
//! The allocator call re-enters the guest and may grow its memory, so no
//! memory slice is held across it.

use wasmtime::AsContextMut;

use crate::Error;
use crate::runtime::HostState;
use crate::runtime::guest_call::fail;
use crate::runtime::memory::{GuestPtr, GuestSlice, split};

/// Asks the guest allocator for `size` bytes.
///
/// A failure inside the allocator poisons the state, like any guest call.
pub(crate) fn allocate(
    ctx: &mut impl AsContextMut<Data = HostState>,
    size: u32,
) -> Result<GuestPtr, Error> {
    if ctx.as_context().data().is_poisoned() {
        return Err(Error::Poisoned);
    }
    let allocator = ctx
        .as_context()
        .data()
        .allocator()
        .ok_or(Error::MissingAllocator)?;
    let requested = i32::try_from(size).map_err(|_| Error::ValueTooLarge {
        size: size as usize,
    })?;
    match allocator.call(&mut *ctx, requested) {
        Ok(0) => Err(Error::AllocationFailed { size }),
        Ok(address) => Ok(GuestPtr::try_from(address)?),
        Err(error) => Err(fail(ctx.as_context_mut().data_mut(), error)),
    }
}

/// Allocates room for `bytes` in the guest and copies them there.
///
/// An empty value needs no allocation and is reported as an empty range at
/// address zero, because a conforming allocator may return null for zero
/// bytes.
pub(crate) fn write_to_guest(
    ctx: &mut impl AsContextMut<Data = HostState>,
    bytes: &[u8],
) -> Result<GuestSlice, Error> {
    let len = u32::try_from(bytes.len()).map_err(|_| Error::ValueTooLarge { size: bytes.len() })?;
    if len == 0 {
        return Ok(GuestSlice::new(GuestPtr::from_address(0), 0)?);
    }
    let address = allocate(ctx, len)?;
    let slice = GuestSlice::new(address, len)?;
    let (mut memory, _) = split(ctx)?;
    memory.write(slice, bytes)?;
    Ok(slice)
}

/// Writes `bytes` into the guest and stores the address and the size at the
/// two return pointers.
///
/// Both return pointers are checked before the allocator runs, so a bad
/// pointer costs the guest no allocation.
pub(crate) fn write_return(
    ctx: &mut impl AsContextMut<Data = HostState>,
    bytes: &[u8],
    return_data: GuestPtr,
    return_size: GuestPtr,
) -> Result<(), Error> {
    {
        let (memory, _) = split(ctx)?;
        memory.read_u32(return_data)?;
        memory.read_u32(return_size)?;
    }
    let slice = write_to_guest(ctx, bytes)?;
    let (mut memory, _) = split(ctx)?;
    memory.write_u32(return_data, slice.ptr().address())?;
    memory.write_u32(return_size, slice.len())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{Limit, MemoryError};
    use crate::runtime::guest_call::Budget;
    use crate::runtime::test_support::{engine, instance, services, wat_bytes};
    use crate::runtime::{Engine, EngineConfig, Instance, Limits, Module};

    const FIXED: &str = r#"(module
        (memory (export "memory") 1)
        (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 1024)
        (func (export "_start")))"#;

    #[test]
    fn proxy_on_memory_allocate_is_used_and_the_bytes_land_there() {
        // Arrange
        let engine = engine();
        let mut instance = instance(&engine, FIXED).unwrap();

        // Act
        let slice = write_to_guest(instance.store_mut(), b"hello").unwrap();

        // Assert
        assert_eq!((slice.ptr().address(), slice.len()), (1024, 5));
        assert_eq!(
            instance.memory().unwrap().read(slice),
            Ok(b"hello".as_slice())
        );
    }

    #[test]
    fn malloc_is_used_when_it_is_the_only_allocator() {
        // Arrange
        let engine = engine();
        let wat = r#"(module
            (memory (export "memory") 1)
            (func (export "malloc") (param i32) (result i32) i32.const 2048)
            (func (export "_start")))"#;
        let mut instance = instance(&engine, wat).unwrap();

        // Act
        let address = allocate(instance.store_mut(), 4).unwrap();

        // Assert
        assert_eq!(address.address(), 2048);
    }

    #[test]
    fn proxy_on_memory_allocate_wins_over_malloc() {
        // Arrange
        let engine = engine();
        let wat = r#"(module
            (memory (export "memory") 1)
            (func (export "proxy_on_memory_allocate") (param i32) (result i32) (i32.store8 (i32.const 0) (i32.const 1)) i32.const 1024)
            (func (export "malloc") (param i32) (result i32) (i32.store8 (i32.const 0) (i32.const 2)) i32.const 1024)
            (func (export "_start")))"#;
        let mut instance = instance(&engine, wat).unwrap();

        // Act
        let address = allocate(instance.store_mut(), 4).unwrap();

        // Assert
        assert_eq!(address.address(), 1024);
        assert_eq!(
            instance
                .memory()
                .unwrap()
                .read(GuestSlice::new(GuestPtr::from_address(0), 1).unwrap()),
            Ok([1].as_slice())
        );
    }

    #[test]
    fn a_null_allocation_is_reported() {
        // Arrange
        let engine = engine();
        let wat = r#"(module
            (memory (export "memory") 1)
            (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 0)
            (func (export "_start")))"#;
        let mut instance = instance(&engine, wat).unwrap();

        // Act
        let result = allocate(instance.store_mut(), 16);

        // Assert
        assert!(matches!(result, Err(Error::AllocationFailed { size: 16 })));
        assert!(!instance.is_poisoned());
    }

    #[test]
    fn an_empty_write_does_not_enter_the_guest() {
        // Arrange
        let engine = engine();
        let wat = r#"(module
            (memory (export "memory") 1)
            (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 0)
            (func (export "_start")))"#;
        let mut instance = instance(&engine, wat).unwrap();

        // Act
        let slice = write_to_guest(instance.store_mut(), b"").unwrap();

        // Assert
        assert_eq!((slice.ptr().address(), slice.len()), (0, 0));
    }

    #[test]
    fn a_size_the_abi_cannot_carry_is_rejected() {
        // Arrange
        let engine = engine();
        let mut instance = instance(&engine, FIXED).unwrap();

        // Act
        let result = allocate(instance.store_mut(), u32::MAX);

        // Assert
        assert!(matches!(result, Err(Error::ValueTooLarge { size }) if size == u32::MAX as usize));
    }

    #[test]
    fn an_allocation_outside_memory_is_out_of_bounds() {
        // Arrange
        let engine = engine();
        let wat = r#"(module
            (memory (export "memory") 1)
            (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 65530)
            (func (export "_start")))"#;
        let mut instance = instance(&engine, wat).unwrap();

        // Act
        let result = write_to_guest(instance.store_mut(), b"12345678");

        // Assert
        assert!(matches!(
            result,
            Err(Error::Memory(MemoryError::OutOfBounds {
                ptr: 65_530,
                len: 8,
                ..
            }))
        ));
    }

    #[test]
    fn a_trapping_allocator_poisons_the_instance() {
        // Arrange
        let engine = engine();
        let wat = r#"(module
            (memory (export "memory") 1)
            (func (export "proxy_on_memory_allocate") (param i32) (result i32) unreachable)
            (func (export "_start")))"#;
        let mut instance = instance(&engine, wat).unwrap();

        // Act
        let result = allocate(instance.store_mut(), 1);

        // Assert
        assert!(matches!(result, Err(Error::Trap { .. })));
        assert!(instance.is_poisoned());
        assert!(matches!(
            allocate(instance.store_mut(), 1),
            Err(Error::Poisoned)
        ));
    }

    #[test]
    fn a_growing_allocator_does_not_invalidate_the_write() {
        // Arrange
        let engine = engine();
        let wat = r#"(module
            (memory (export "memory") 1)
            (func (export "proxy_on_memory_allocate") (param i32) (result i32)
                (drop (memory.grow (i32.const 1)))
                i32.const 70000)
            (func (export "_start")))"#;
        let mut instance = instance(&engine, wat).unwrap();

        // Act
        let slice = write_to_guest(instance.store_mut(), b"grown").unwrap();

        // Assert
        assert_eq!(slice.ptr().address(), 70_000);
        assert_eq!(
            instance.memory().unwrap().read(slice),
            Ok(b"grown".as_slice())
        );
        assert_eq!(instance.memory().unwrap().size(), 2 * 65_536);
    }

    const COUNTING: &str = r#"(module
        (memory (export "memory") 1)
        (func (export "proxy_on_memory_allocate") (param i32) (result i32)
            (i32.store8 (i32.const 0) (i32.add (i32.load8_u (i32.const 0)) (i32.const 1)))
            i32.const 1024)
        (func (export "_start")))"#;

    fn allocator_calls(instance: &mut Instance) -> u8 {
        instance
            .memory()
            .unwrap()
            .read(GuestSlice::new(GuestPtr::from_address(0), 1).unwrap())
            .unwrap()[0]
    }

    #[test]
    fn write_return_stores_the_address_and_the_size() {
        // Arrange
        let engine = engine();
        let mut instance = instance(&engine, COUNTING).unwrap();
        let (data_ptr, size_ptr) = (GuestPtr::from_address(100), GuestPtr::from_address(104));

        // Act
        let result = write_return(instance.store_mut(), b"abc", data_ptr, size_ptr);

        // Assert
        assert!(result.is_ok());
        let memory = instance.memory().unwrap();
        assert_eq!(memory.read_u32(data_ptr), Ok(1024));
        assert_eq!(memory.read_u32(size_ptr), Ok(3));
        assert_eq!(
            memory.read(GuestSlice::new(GuestPtr::from_address(1024), 3).unwrap()),
            Ok(b"abc".as_slice())
        );
    }

    #[test]
    fn write_return_of_an_empty_value_writes_two_zeros_without_allocating() {
        // Arrange
        let engine = engine();
        let mut instance = instance(&engine, COUNTING).unwrap();
        let (data_ptr, size_ptr) = (GuestPtr::from_address(100), GuestPtr::from_address(104));
        instance.memory().unwrap().write_u32(data_ptr, 7).unwrap();

        // Act
        let result = write_return(instance.store_mut(), b"", data_ptr, size_ptr);

        // Assert
        assert!(result.is_ok());
        assert_eq!(allocator_calls(&mut instance), 0);
        let memory = instance.memory().unwrap();
        assert_eq!(memory.read_u32(data_ptr), Ok(0));
        assert_eq!(memory.read_u32(size_ptr), Ok(0));
    }

    #[test]
    fn write_return_with_a_bad_return_pointer_does_not_allocate() {
        // Arrange
        let engine = engine();
        let mut instance = instance(&engine, COUNTING).unwrap();
        let (data_ptr, size_ptr) = (GuestPtr::from_address(100), GuestPtr::from_address(65_534));

        // Act
        let result = write_return(instance.store_mut(), b"abc", data_ptr, size_ptr);

        // Assert
        assert!(matches!(
            result,
            Err(Error::Memory(MemoryError::OutOfBounds { .. }))
        ));
        assert_eq!(allocator_calls(&mut instance), 0);
    }

    #[test]
    fn a_poisoned_state_is_refused_before_the_allocator_runs() {
        // Arrange
        let engine = engine();
        let mut instance = instance(&engine, FIXED).unwrap();
        instance.state_mut().poison();

        // Act
        let allocated = allocate(instance.store_mut(), 16);

        // Assert
        assert!(matches!(allocated, Err(Error::Poisoned)));
    }

    /// A guest that works, takes an allocation from the host, and works
    /// again, all inside one call.
    ///
    /// `fuel_twice` spends the same fuel on each side of the allocation.
    /// `tick_twice` moves the epoch `n` times on each side of it.
    const ALLOCATES_MID_CALL: &str = r#"(module
        (import "env" "take" (func $take))
        (import "env" "tick" (func $tick))
        (memory (export "memory") 1)
        (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 1024)
        (func $spend (result i32)
            (local $i i32)
            (loop $l
                (local.set $i (i32.add (local.get $i) (i32.const 1)))
                (br_if $l (i32.lt_u (local.get $i) (i32.const 1000))))
            (local.get $i))
        (func $ticks (param $n i32)
            (local $i i32)
            (loop $l
                call $tick
                (local.set $i (i32.add (local.get $i) (i32.const 1)))
                (br_if $l (i32.lt_u (local.get $i) (local.get $n)))))
        (func (export "fuel_once") (result i32) call $spend)
        (func (export "fuel_twice") (result i32)
            call $spend
            drop
            call $take
            call $spend)
        (func (export "tick_twice") (param $n i32)
            (call $ticks (local.get $n))
            call $take
            (call $ticks (local.get $n))))"#;

    /// An engine whose guests can ask the host for an allocation through
    /// `env.take` and can move the epoch through `env.tick`.
    fn mid_call_engine() -> Engine {
        EngineConfig::new()
            .with_external_ticks(true)
            .with_fuel_enabled(true)
            .build_with(|linker| {
                linker
                    .func_wrap(
                        "env",
                        "take",
                        |mut caller: wasmtime::Caller<'_, HostState>| -> wasmtime::Result<()> {
                            allocate(&mut caller, 16)?;
                            Ok(())
                        },
                    )
                    .and_then(|linker| {
                        linker.func_wrap(
                            "env",
                            "tick",
                            |caller: wasmtime::Caller<'_, HostState>| {
                                caller.engine().increment_epoch();
                            },
                        )
                    })
                    .map_err(|source| Error::Config {
                        message: format!("the test imports could not be registered: {source}"),
                    })?;
                Ok(())
            })
            .unwrap()
    }

    fn mid_call_instance(engine: &Engine, limits: &Limits) -> Instance {
        let module = Module::new(engine, &wat_bytes(ALLOCATES_MID_CALL)).unwrap();
        Instance::new(engine, &module, crate::abi::state(services()), limits).unwrap()
    }

    /// The fuel one call of `fuel_once` costs on this engine, measured so a
    /// change to wasmtime's cost model cannot make the budget cover two.
    fn fuel_once_cost(engine: &Engine) -> u64 {
        let limits = Limits::new().with_fuel(u64::from(u32::MAX));
        let mut instance = mid_call_instance(engine, &limits);
        let before = instance.store_mut().get_fuel().unwrap();
        instance.call::<(), i32>("fuel_once", ()).unwrap();
        before - instance.store_mut().get_fuel().unwrap()
    }

    #[test]
    fn an_allocation_inside_a_call_does_not_restore_the_fuel() {
        // Arrange
        let engine = mid_call_engine();
        let cost = fuel_once_cost(&engine);
        let limits = Limits::new().with_fuel(cost + cost / 2);
        let mut control = mid_call_instance(&engine, &limits);
        assert!(
            matches!(control.call::<(), i32>("fuel_once", ()), Ok(1000)),
            "the budget must cover one loop, or the second loop proves nothing"
        );
        let mut instance = mid_call_instance(&engine, &limits);

        // Act
        let result = instance.call::<(), i32>("fuel_twice", ());

        // Assert
        assert!(
            matches!(result, Err(Error::LimitExceeded { limit: Limit::Fuel })),
            "the budget covers one loop, so the second loop passes only on restored fuel: {result:?}"
        );
        assert!(instance.is_poisoned());
    }

    #[test]
    fn an_allocation_inside_a_call_does_not_move_the_epoch_deadline() {
        // Arrange
        let engine = mid_call_engine();
        let limits = Limits::new()
            .with_cpu_time(engine.epoch_period() * 8)
            .with_fuel(u64::from(u32::MAX));
        let ticks = Budget::new(&limits, &engine).epoch_ticks;
        let each_side = i32::try_from(ticks - 1).unwrap();
        let mut instance = mid_call_instance(&engine, &limits);

        // Act
        let result = instance.call::<(i32,), ()>("tick_twice", (each_side,));

        // Assert
        assert_eq!(ticks, 8);
        assert!(
            matches!(
                result,
                Err(Error::LimitExceeded {
                    limit: Limit::Epoch
                })
            ),
            "each side stays below the deadline alone, so both pass only on a moved deadline: {result:?}"
        );
        assert!(instance.is_poisoned());
    }
}
