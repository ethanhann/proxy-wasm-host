//! A running guest and the calls into it.

use wasmtime::{Store, StoreLimitsBuilder, TypedFunc, WasmParams, WasmResults};

use crate::Error;
use crate::runtime::guest_call::{Budget, call_on};
#[cfg(test)]
use crate::runtime::memory::{GuestMemory, GuestPtr, GuestSlice, split};
use crate::runtime::{Engine, HostState, Limits, Module};

const MEMORY_EXPORT: &str = "memory";

/// One instantiated guest with its store.
///
/// Every call into the guest refills the CPU and fuel budgets first.
/// Any error that unwinds a guest call poisons the instance.
/// Every later call then returns [`Error::Poisoned`], and you create a new
/// instance from the same module to recover.
/// Dropping the instance releases the guest memory.
pub(crate) struct Instance {
    store: Store<HostState>,
    inner: wasmtime::Instance,
    module: Module,
    budget: Budget,
}

impl Instance {
    /// Instantiates `module`, resolves its memory and allocator, and runs its
    /// start sequence.
    ///
    /// The start sequence follows the ABI document.
    /// When the guest exports `_initialize`, it is called, and then `main`
    /// with two zero arguments when `main` is exported.
    /// When the guest does not export `_initialize`, `_start` is called when
    /// it is exported.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Config`] when `limits` asks for fuel on an engine
    /// without it, [`Error::Instantiate`] when an import is missing,
    /// [`Error::MissingMemory`] and [`Error::MissingAllocator`] for the two
    /// required exports, and the mapped error when a start function fails.
    pub(crate) fn new(
        engine: &Engine,
        module: &Module,
        abi: Box<dyn std::any::Any + Send>,
        limits: &Limits,
    ) -> Result<Self, Error> {
        if limits.fuel().is_some() && !engine.fuel_enabled() {
            return Err(Error::Config {
                message: "fuel is not enabled on this engine".to_owned(),
            });
        }
        let budget = Budget::new(limits, engine);
        let mut store = Store::new(engine.wasmtime(), HostState::new(abi));
        budget.refill(&mut store)?;
        let mut builder = StoreLimitsBuilder::new();
        if let Some(bytes) = limits.memory_bytes() {
            builder = builder.memory_size(bytes);
        }
        store.data_mut().set_store_limits(builder.build());
        store.limiter(
            |state: &mut HostState| -> &mut dyn wasmtime::ResourceLimiter { state.store_limits() },
        );

        let inner = engine
            .linker()
            .instantiate(&mut store, module.wasmtime())
            .map_err(|source| Error::Instantiate {
                source: source.into(),
            })?;
        let memory = inner
            .get_memory(&mut store, MEMORY_EXPORT)
            .ok_or(Error::MissingMemory)?;
        store.data_mut().set_memory(memory);
        let allocator = ["proxy_on_memory_allocate", "malloc"]
            .into_iter()
            .find_map(|name| inner.get_typed_func::<i32, i32>(&mut store, name).ok())
            .ok_or(Error::MissingAllocator)?;
        store.data_mut().set_allocator(allocator);

        let mut instance = Self {
            store,
            inner,
            module: module.clone(),
            budget,
        };
        instance.start()?;
        Ok(instance)
    }

    fn start(&mut self) -> Result<(), Error> {
        if let Some(initialize) = self.typed_func::<(), ()>("_initialize")? {
            self.call_typed(&initialize, ())?;
            if let Some(main) = self.typed_func::<(i32, i32), i32>("main")? {
                self.call_typed(&main, (0, 0))?;
            }
        } else if let Some(start) = self.typed_func::<(), ()>("_start")? {
            self.call_typed(&start, ())?;
        }
        Ok(())
    }

    /// Whether an earlier failure unwound a guest call.
    pub(crate) fn is_poisoned(&self) -> bool {
        self.store.data().is_poisoned()
    }

    pub(crate) fn state(&self) -> &HostState {
        self.store.data()
    }

    pub(crate) fn state_mut(&mut self) -> &mut HostState {
        self.store.data_mut()
    }

    #[cfg(test)]
    pub(crate) fn store_mut(&mut self) -> &mut Store<HostState> {
        &mut self.store
    }

    /// Whether the module exports `name`.
    pub(crate) fn has_export(&self, name: &str) -> bool {
        self.module.has_export(name)
    }

    fn ensure_live(&self) -> Result<(), Error> {
        if self.is_poisoned() {
            Err(Error::Poisoned)
        } else {
            Ok(())
        }
    }

    /// The guest memory.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Poisoned`] after an earlier failure, and
    /// [`Error::MissingMemory`] when the cached memory handle is absent, which
    /// cannot happen after a successful construction.
    #[cfg(test)]
    pub(crate) fn memory(&mut self) -> Result<GuestMemory<'_>, Error> {
        self.ensure_live()?;
        split(&mut self.store).map(|(memory, _)| memory)
    }

    /// Calls the exported function `name` with `params`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Poisoned`] after an earlier failure,
    /// [`Error::MissingExport`] when there is no such export,
    /// [`Error::ExportTypeMismatch`] when it is not a function of that type,
    /// and the mapped error when the call fails, which poisons the instance.
    pub(crate) fn call<P: WasmParams, R: WasmResults>(
        &mut self,
        name: &str,
        params: P,
    ) -> Result<R, Error> {
        let func = self
            .typed_func::<P, R>(name)?
            .ok_or_else(|| Error::MissingExport {
                name: name.to_owned(),
            })?;
        self.call_typed(&func, params)
    }

    /// The exported function `name`, or `None` when the module has no such
    /// export.
    pub(crate) fn typed_func<P: WasmParams, R: WasmResults>(
        &mut self,
        name: &str,
    ) -> Result<Option<TypedFunc<P, R>>, Error> {
        self.ensure_live()?;
        let Some(func) = self.inner.get_func(&mut self.store, name) else {
            return if self.module.has_export(name) {
                Err(Error::ExportTypeMismatch {
                    name: name.to_owned(),
                })
            } else {
                Ok(None)
            };
        };
        func.typed::<P, R>(&self.store)
            .map(Some)
            .map_err(|_| Error::ExportTypeMismatch {
                name: name.to_owned(),
            })
    }

    /// Refills the budget and calls `func`, which must belong to this
    /// instance.
    pub(crate) fn call_typed<P: WasmParams, R: WasmResults>(
        &mut self,
        func: &TypedFunc<P, R>,
        params: P,
    ) -> Result<R, Error> {
        call_on(&mut self.store, self.budget, func, params)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::error::Limit;
    use crate::runtime::EngineConfig;
    use crate::runtime::test_support::{
        MINIMAL_GUEST, engine, instance, instance_from, services, wat_bytes,
    };

    /// A guest whose loop ends on its own, so a test cannot hang when a limit
    /// is not enforced.
    const BOUNDED: &str = r#"(module
        (memory (export "memory") 1)
        (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 1024)
        (func (export "burn") (result i32)
            (local $i i32)
            (loop $l
                (local.set $i (i32.add (local.get $i) (i32.const 1)))
                (br_if $l (i32.lt_u (local.get $i) (i32.const 1000))))
            (local.get $i)))"#;

    /// A bounded guest that moves the epoch itself, so the deadline can pass
    /// during a call with no second thread.
    const TICKING: &str = r#"(module
        (import "env" "tick" (func $tick))
        (memory (export "memory") 1)
        (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 1024)
        (func (export "burn") (result i32)
            (local $i i32)
            (loop $l
                call $tick
                (local.set $i (i32.add (local.get $i) (i32.const 1)))
                (br_if $l (i32.lt_u (local.get $i) (i32.const 1000))))
            (local.get $i)))"#;

    /// The fuel one call of `burn` costs on this engine.
    ///
    /// The cost is measured rather than recorded, so a change to the loop or
    /// to wasmtime's cost model cannot leave a budget that covers two calls
    /// and a refill test that passes for the wrong reason.
    fn burn_cost(engine: &Engine, wat: &str) -> u64 {
        let module = Module::new(engine, &wat_bytes(wat)).unwrap();
        let limits = Limits::new().with_fuel(u64::from(u32::MAX));
        let mut instance =
            Instance::new(engine, &module, crate::abi::state(services()), &limits).unwrap();
        let before = instance.store_mut().get_fuel().unwrap();
        instance.call::<(), i32>("burn", ()).unwrap();
        before - instance.store_mut().get_fuel().unwrap()
    }

    /// A budget that serves one call of `burn` and never two.
    fn one_call_of(engine: &Engine, wat: &str) -> u64 {
        let cost = burn_cost(engine, wat);
        cost + cost / 2
    }

    /// An engine that meters fuel, which [`Limits::with_fuel`] requires.
    fn metered_engine() -> Engine {
        EngineConfig::new()
            .with_external_ticks(true)
            .with_fuel_enabled(true)
            .build()
            .unwrap()
    }

    /// An engine whose guests can move the epoch through an `env.tick` import.
    ///
    /// The guest moves the clock itself, so a deadline can pass during a call
    /// with no second thread and no sleep.
    fn ticking_engine() -> Engine {
        EngineConfig::new()
            .with_external_ticks(true)
            .build_with(|linker| {
                linker
                    .func_wrap("env", "tick", |caller: wasmtime::Caller<'_, HostState>| {
                        caller.engine().increment_epoch();
                    })
                    .map_err(|source| Error::Config {
                        message: format!("the tick import could not be registered: {source}"),
                    })?;
                Ok(())
            })
            .unwrap()
    }

    fn assert_send<T: Send>() {}

    const _: () = {
        let _ = assert_send::<Instance>;
    };

    #[test]
    fn the_minimal_guest_instantiates_with_one_page() {
        // Arrange
        let engine = engine();

        // Act
        let mut instance = instance(&engine, MINIMAL_GUEST).unwrap();

        // Assert
        assert_eq!(instance.memory().unwrap().size(), 65_536);
        assert!(!instance.is_poisoned());
    }

    #[test]
    fn missing_memory_and_allocator_are_rejected() {
        // Arrange
        let engine = engine();
        let no_memory = r#"(module (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 0))"#;
        let no_allocator = r#"(module (memory (export "memory") 1))"#;

        // Act
        let results = [
            instance(&engine, no_memory).err(),
            instance(&engine, no_allocator).err(),
        ];

        // Assert
        assert!(matches!(results[0], Some(Error::MissingMemory)));
        assert!(matches!(results[1], Some(Error::MissingAllocator)));
    }

    #[test]
    fn an_unknown_wasi_import_does_not_instantiate() {
        // Arrange
        let engine = engine();
        let wat = r#"(module
            (import "wasi_snapshot_preview1" "fd_read" (func (param i32 i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 0))"#;

        // Act
        let result = instance(&engine, wat);

        // Assert
        assert!(matches!(result, Err(Error::Instantiate { .. })));
    }

    #[test]
    fn initialize_then_main_runs_and_start_does_not() {
        // Arrange
        let engine = engine();
        let wat = r#"(module
            (memory (export "memory") 1)
            (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 1024)
            (func (export "_initialize") (i32.store8 (i32.const 0) (i32.const 1)))
            (func (export "main") (param i32 i32) (result i32) (i32.store8 (i32.const 1) (i32.const 2)) i32.const 0)
            (func (export "_start") (i32.store8 (i32.const 2) (i32.const 3))))"#;

        // Act
        let mut instance = instance(&engine, wat).unwrap();

        // Assert
        let memory = instance.memory().unwrap();
        assert_eq!(
            memory.read(GuestSlice::new(GuestPtr::from_address(0), 3).unwrap()),
            Ok([1, 2, 0].as_slice())
        );
    }

    #[test]
    fn start_runs_when_initialize_is_absent() {
        // Arrange
        let engine = engine();
        let wat = r#"(module
            (memory (export "memory") 1)
            (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 1024)
            (func (export "main") (param i32 i32) (result i32) (i32.store8 (i32.const 1) (i32.const 2)) i32.const 0)
            (func (export "_start") (i32.store8 (i32.const 2) (i32.const 3))))"#;

        // Act
        let mut instance = instance(&engine, wat).unwrap();

        // Assert
        let memory = instance.memory().unwrap();
        assert_eq!(
            memory.read(GuestSlice::new(GuestPtr::from_address(0), 3).unwrap()),
            Ok([0, 0, 3].as_slice())
        );
    }

    #[test]
    fn a_trap_in_a_start_function_yields_no_instance() {
        // Arrange
        let engine = engine();
        let in_start = r#"(module
            (memory (export "memory") 1)
            (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 1024)
            (func (export "_start") unreachable))"#;
        let in_main = r#"(module
            (memory (export "memory") 1)
            (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 1024)
            (func (export "_initialize"))
            (func (export "main") (param i32 i32) (result i32) unreachable))"#;

        // Act
        let results = [
            instance(&engine, in_start).err(),
            instance(&engine, in_main).err(),
        ];

        // Assert
        assert!(matches!(results[0], Some(Error::Trap { .. })));
        assert!(matches!(results[1], Some(Error::Trap { .. })));
    }

    #[test]
    fn export_lookups_distinguish_absent_mistyped_and_non_function_exports() {
        // Arrange
        let engine = engine();
        let mut instance = instance(&engine, MINIMAL_GUEST).unwrap();

        // Act
        let observed = (
            instance.has_export("_start"),
            instance.has_export("absent"),
            instance.call::<(), ()>("absent", ()).err(),
            instance.call::<(i32,), i32>("_start", (1,)).err(),
            instance.call::<(), ()>("memory", ()).err(),
            instance.typed_func::<(), ()>("absent").map(|f| f.is_none()),
        );

        // Assert
        assert_eq!((observed.0, observed.1), (true, false));
        assert!(matches!(observed.2, Some(Error::MissingExport { name }) if name == "absent"));
        assert!(matches!(observed.3, Some(Error::ExportTypeMismatch { name }) if name == "_start"));
        assert!(matches!(observed.4, Some(Error::ExportTypeMismatch { name }) if name == "memory"));
        assert!(matches!(observed.5, Ok(true)));
    }

    #[test]
    fn a_trap_poisons_the_instance() {
        // Arrange
        let engine = engine();
        let wat = r#"(module
            (memory (export "memory") 1)
            (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 1024)
            (func (export "crash") unreachable)
            (func (export "ok")))"#;
        let mut instance = instance(&engine, wat).unwrap();

        // Act
        let result = instance.call::<(), ()>("crash", ());

        // Assert
        assert!(
            matches!(&result, Err(Error::Trap { message, backtrace: Some(_) }) if message.contains("unreachable"))
        );
        assert!(instance.is_poisoned());
        assert!(matches!(
            instance.call::<(), ()>("ok", ()),
            Err(Error::Poisoned)
        ));
        assert!(matches!(instance.memory(), Err(Error::Poisoned)));
    }

    #[test]
    fn a_new_instance_from_the_same_module_runs_after_a_trap() {
        // Arrange
        let engine = engine();
        let wat = r#"(module
            (memory (export "memory") 1)
            (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 1024)
            (func (export "crash") unreachable)
            (func (export "ok")))"#;
        let module = Module::new(&engine, &wat_bytes(wat)).unwrap();
        let mut crashed = instance_from(&engine, &module).unwrap();
        let _ = crashed.call::<(), ()>("crash", ());
        let mut fresh = instance_from(&engine, &module).unwrap();

        // Act
        let result = fresh.call::<(), ()>("ok", ());

        // Assert
        assert!(result.is_ok());
        assert!(crashed.is_poisoned());
        assert!(!fresh.is_poisoned());
    }

    #[test]
    fn a_host_error_round_trips_through_the_guest() {
        // Arrange
        let engine = engine();
        let wat = r#"(module
            (import "wasi_snapshot_preview1" "proc_exit" (func $exit (param i32)))
            (memory (export "memory") 1)
            (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 1024)
            (func (export "leave") (call $exit (i32.const 7))))"#;
        let mut instance = instance(&engine, wat).unwrap();

        // Act
        let result = instance.call::<(), ()>("leave", ());

        // Assert
        assert!(matches!(result, Err(Error::GuestExit { code: 7 })));
        assert!(instance.is_poisoned());
    }

    #[test]
    fn a_guest_that_outlives_its_epoch_deadline_is_stopped() {
        // Arrange
        let engine = ticking_engine();
        let module = Module::new(&engine, &wat_bytes(TICKING)).unwrap();
        let limits = Limits::new().with_cpu_time(Duration::from_millis(10));
        let mut instance =
            Instance::new(&engine, &module, crate::abi::state(services()), &limits).unwrap();

        // Act
        let result = instance.call::<(), i32>("burn", ());

        // Assert
        assert!(
            matches!(
                result,
                Err(Error::LimitExceeded {
                    limit: Limit::Epoch
                })
            ),
            "the guest moved the epoch past its own deadline, so it must be stopped"
        );
        assert!(instance.is_poisoned());
    }

    #[test]
    fn a_call_gets_a_fresh_epoch_deadline_when_the_epoch_moved_since_construction() {
        // Arrange
        let engine = engine();
        let module = Module::new(&engine, &wat_bytes(MINIMAL_GUEST)).unwrap();
        let limits = Limits::new().with_cpu_time(Duration::from_millis(10));
        let mut instance =
            Instance::new(&engine, &module, crate::abi::state(services()), &limits).unwrap();
        for _ in 0..5 {
            engine.increment_epoch();
        }

        // Act
        let result = instance.call::<(i32,), i32>("proxy_on_memory_allocate", (1,));

        // Assert
        assert!(
            matches!(result, Ok(1024)),
            "the call refills the deadline, so an epoch that moved since construction is harmless"
        );
    }

    #[test]
    fn a_fresh_instance_gets_a_full_budget_after_the_epoch_moved() {
        // Arrange
        let engine = engine();
        let module = Module::new(&engine, &wat_bytes(MINIMAL_GUEST)).unwrap();
        let limits = Limits::new().with_cpu_time(Duration::from_millis(20));
        for _ in 0..100 {
            engine.increment_epoch();
        }
        let mut fresh =
            Instance::new(&engine, &module, crate::abi::state(services()), &limits).unwrap();

        // Act
        let result = fresh.call::<(i32,), i32>("proxy_on_memory_allocate", (1,));

        // Assert
        assert!(matches!(result, Ok(1024)));
    }

    #[test]
    fn a_bounded_guest_is_stopped_by_fuel() {
        // Arrange
        let engine = metered_engine();
        let module = Module::new(&engine, &wat_bytes(BOUNDED)).unwrap();
        let limits = Limits::new().with_fuel(burn_cost(&engine, BOUNDED) / 4);
        let mut instance =
            Instance::new(&engine, &module, crate::abi::state(services()), &limits).unwrap();

        // Act
        let result = instance.call::<(), i32>("burn", ());

        // Assert
        assert!(matches!(
            result,
            Err(Error::LimitExceeded { limit: Limit::Fuel })
        ));
    }

    #[test]
    fn a_second_call_gets_a_fresh_fuel_budget() {
        // Arrange
        let engine = metered_engine();
        let module = Module::new(&engine, &wat_bytes(BOUNDED)).unwrap();
        let limits = Limits::new().with_fuel(one_call_of(&engine, BOUNDED));
        let mut instance =
            Instance::new(&engine, &module, crate::abi::state(services()), &limits).unwrap();
        instance.call::<(), i32>("burn", ()).unwrap();

        // Act
        let second = instance.call::<(), i32>("burn", ());

        // Assert
        assert!(
            matches!(second, Ok(1000)),
            "the budget the constructor set covers one call, so a second call proves the refill"
        );
    }

    #[test]
    fn fuel_on_an_engine_without_fuel_is_rejected() {
        // Arrange
        let engine = engine();
        let module = Module::new(&engine, &wat_bytes(MINIMAL_GUEST)).unwrap();
        let limits = Limits::new().with_fuel(1);

        // Act
        let result = Instance::new(&engine, &module, crate::abi::state(services()), &limits);

        // Assert
        assert!(matches!(result, Err(Error::Config { .. })));
    }

    #[test]
    fn unbounded_recursion_is_a_stack_limit() {
        // Arrange
        let engine = engine();
        let wat = r#"(module
            (memory (export "memory") 1)
            (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 1024)
            (func $down (call $down))
            (func (export "recurse") (call $down)))"#;
        let mut instance = instance(&engine, wat).unwrap();

        // Act
        let result = instance.call::<(), ()>("recurse", ());

        // Assert
        assert!(matches!(
            result,
            Err(Error::LimitExceeded {
                limit: Limit::Stack
            })
        ));
    }

    #[test]
    fn memory_growth_past_the_ceiling_fails_in_the_guest() {
        // Arrange
        let engine = engine();
        let wat = r#"(module
            (memory (export "memory") 1)
            (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 1024)
            (func (export "grow") (result i32) (memory.grow (i32.const 1))))"#;
        let module = Module::new(&engine, &wat_bytes(wat)).unwrap();
        let limits = Limits::new().with_memory_bytes(65_536);
        let mut instance =
            Instance::new(&engine, &module, crate::abi::state(services()), &limits).unwrap();

        // Act
        let grown = instance.call::<(), i32>("grow", ());

        // Assert
        assert!(matches!(grown, Ok(-1)));
    }
}
