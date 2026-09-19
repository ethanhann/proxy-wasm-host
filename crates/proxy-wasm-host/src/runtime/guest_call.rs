//! The shared path for every call into the guest.
//!
//! A call from the host refills the CPU and fuel budgets, runs, and on
//! failure poisons the store data, maps the wasmtime error, and logs the
//! failure once.
//! The allocator runs while a call is already in the guest, so it spends the
//! budget of that call and refills nothing.
//! It shares [`fail`] with this path, so a guest re-entry has one failure
//! path.

use wasmtime::{Store, Trap, TypedFunc, WasmBacktrace, WasmParams, WasmResults};

use crate::Error;
use crate::error::Limit;
use crate::runtime::{Engine, HostState, Limits};

/// The budgets that every call refills.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Budget {
    pub(crate) epoch_ticks: u64,
    pub(crate) fuel: Option<u64>,
}

impl Budget {
    /// The budget that `limits` means on `engine`.
    ///
    /// The tick count is at least one and is clamped well below `u64::MAX`,
    /// because wasmtime adds it to the current epoch.
    pub(crate) fn new(limits: &Limits, engine: &Engine) -> Self {
        let period = engine.epoch_period().as_nanos().max(1);
        let ticks = limits.cpu_time().as_nanos().div_ceil(period).max(1);
        let epoch_ticks = u64::try_from(ticks).unwrap_or(u64::MAX).min(u64::MAX / 4);
        Self {
            epoch_ticks,
            fuel: limits.fuel(),
        }
    }

    /// Applies the budget to `store` before a call.
    pub(crate) fn refill(self, store: &mut Store<HostState>) -> Result<(), Error> {
        store.set_epoch_deadline(self.epoch_ticks);
        if let Some(fuel) = self.fuel {
            store.set_fuel(fuel).map_err(|source| Error::Config {
                message: format!("fuel could not be set: {source}"),
            })?;
        }
        Ok(())
    }
}

/// Refills the budget and calls `func` on `store`.
///
/// The store and the function are borrowed apart, so a callback cached next
/// to the store can be called without a clone.
pub(crate) fn call_on<P: WasmParams, R: WasmResults>(
    store: &mut Store<HostState>,
    budget: Budget,
    func: &TypedFunc<P, R>,
    params: P,
) -> Result<R, Error> {
    if store.data().is_poisoned() {
        return Err(Error::Poisoned);
    }
    budget.refill(store)?;
    func.call(&mut *store, params)
        .map_err(|error| fail(store.data_mut(), error))
}

/// Poisons `state`, maps `error`, and logs the failure once.
pub(crate) fn fail(state: &mut HostState, error: wasmtime::Error) -> Error {
    state.poison();
    let mapped = map_guest_error(error);
    tracing::warn!(error = %mapped, "guest call failed, instance poisoned");
    mapped
}

/// Turns a wasmtime error from a guest call into the crate's [`Error`].
///
/// An [`Error`] a host function returned comes back as itself.
/// The epoch, fuel, and stack traps become [`Error::LimitExceeded`].
/// Every other trap becomes [`Error::Trap`] with the reason and the backtrace.
pub(crate) fn map_guest_error(error: wasmtime::Error) -> Error {
    let error = match error.downcast::<Error>() {
        Ok(ours) => return ours,
        Err(other) => other,
    };
    let backtrace = error
        .downcast_ref::<WasmBacktrace>()
        .map(ToString::to_string);
    match error.downcast_ref::<Trap>() {
        Some(Trap::Interrupt) => Error::LimitExceeded {
            limit: Limit::Epoch,
        },
        Some(Trap::OutOfFuel) => Error::LimitExceeded { limit: Limit::Fuel },
        Some(Trap::StackOverflow) => Error::LimitExceeded {
            limit: Limit::Stack,
        },
        Some(trap) => Error::Trap {
            message: trap.to_string(),
            backtrace,
        },
        None => Error::Trap {
            message: error.to_string(),
            backtrace,
        },
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::runtime::test_support::{MINIMAL_GUEST, engine, instance};
    use crate::runtime::{EngineConfig, Limits};

    #[test]
    fn budget_rounds_ticks_up_and_never_below_one() {
        // Arrange
        let engine = EngineConfig::new()
            .with_external_ticks(true)
            .build()
            .unwrap();
        let limits = [
            Limits::new().with_cpu_time(Duration::from_millis(25)),
            Limits::new().with_cpu_time(Duration::ZERO),
            Limits::new().with_cpu_time(Duration::MAX),
        ];

        // Act
        let ticks: Vec<u64> = limits
            .iter()
            .map(|l| Budget::new(l, &engine).epoch_ticks)
            .collect();

        // Assert
        assert_eq!(ticks[0], 3);
        assert_eq!(ticks[1], 1);
        assert_eq!(ticks[2], u64::MAX / 4);
    }

    #[test]
    fn a_plain_wasmtime_error_maps_to_a_trap_without_backtrace() {
        // Arrange
        let error = wasmtime::Error::msg("something else");

        // Act
        let mapped = map_guest_error(error);

        // Assert
        assert!(
            matches!(mapped, Error::Trap { message, backtrace: None } if message == "something else")
        );
    }

    #[test]
    fn a_poisoned_store_is_refused_before_the_guest_runs() {
        // Arrange
        // Every public caller of `call_on` checks first, so this reaches the
        // check directly.
        let engine = engine();
        let mut instance = instance(&engine, MINIMAL_GUEST).unwrap();
        let allocator = instance.state().allocator().unwrap();
        let budget = Budget::new(&Limits::default(), &engine);
        instance.state_mut().poison();

        // Act
        let called = call_on::<i32, i32>(instance.store_mut(), budget, &allocator, 1);

        // Assert
        assert!(matches!(called, Err(Error::Poisoned)));
    }
}
