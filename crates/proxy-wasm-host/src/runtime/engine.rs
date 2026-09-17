//! The engine that compiles modules and drives the epoch clock.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use wasmtime::{Config, Linker};

use crate::Error;
use crate::runtime::HostState;
use crate::runtime::wasi;

const DEFAULT_EPOCH_PERIOD: Duration = Duration::from_millis(10);

/// The settings of an [`Engine`].
///
/// Fuel metering and the wasm stack size are engine properties in wasmtime.
/// They live here and not in [`crate::runtime::Limits`].
/// The struct is non exhaustive, so build it with [`EngineConfig::new`] and
/// the `with_*` methods.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct EngineConfig {
    epoch_period: Duration,
    external_ticks: bool,
    fuel_enabled: bool,
    max_wasm_stack: Option<usize>,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            epoch_period: DEFAULT_EPOCH_PERIOD,
            external_ticks: false,
            fuel_enabled: false,
            max_wasm_stack: None,
        }
    }
}

impl EngineConfig {
    /// A ten millisecond epoch period, the built in ticker, fuel off, and
    /// the default wasm stack.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets how often the epoch advances.
    ///
    /// A guest's CPU time limit is measured in these periods.
    /// A zero period is rejected by [`EngineConfig::build`].
    #[must_use]
    pub fn with_epoch_period(mut self, period: Duration) -> Self {
        self.epoch_period = period;
        self
    }

    /// Chooses whether you advance the epoch yourself.
    ///
    /// With `true` the engine starts no ticker thread.
    /// You then call [`Engine::increment_epoch`] every epoch period from your
    /// own event loop, or no CPU time limit is enforced.
    #[must_use]
    pub fn with_external_ticks(mut self, external: bool) -> Self {
        self.external_ticks = external;
        self
    }

    /// Enables fuel metering, so that [`crate::runtime::Limits::with_fuel`]
    /// can bound a guest call.
    #[must_use]
    pub fn with_fuel_enabled(mut self, enabled: bool) -> Self {
        self.fuel_enabled = enabled;
        self
    }

    /// Sets the wasm stack size in bytes, or restores the default with
    /// `None`.
    #[must_use]
    pub fn with_max_wasm_stack(mut self, bytes: impl Into<Option<usize>>) -> Self {
        self.max_wasm_stack = bytes.into();
        self
    }

    /// Builds the engine, its linker with the WASI functions and the ABI
    /// v0.2.1 host functions, and unless disabled its ticker thread.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Config`] for a zero epoch period, and
    /// [`Error::Instantiate`] when wasmtime rejects the configuration or a
    /// WASI function cannot be registered.
    pub fn build(self) -> Result<Engine, Error> {
        self.build_with(crate::abi::v0_2_1::host_functions::register)
    }

    /// Builds the engine and lets `register` add imports to the linker after
    /// the WASI functions.
    pub(crate) fn build_with(
        self,
        register: impl FnOnce(&mut Linker<HostState>) -> Result<(), Error>,
    ) -> Result<Engine, Error> {
        if self.epoch_period.is_zero() {
            return Err(Error::Config {
                message: "the epoch period must not be zero".to_owned(),
            });
        }
        let mut config = Config::new();
        config.epoch_interruption(true);
        config.consume_fuel(self.fuel_enabled);
        if let Some(bytes) = self.max_wasm_stack {
            config.max_wasm_stack(bytes);
        }
        let engine = wasmtime::Engine::new(&config).map_err(|source| Error::Instantiate {
            source: source.into(),
        })?;
        let mut linker = Linker::new(&engine);
        wasi::add_to_linker(&mut linker)?;
        register(&mut linker)?;
        let ticks = Arc::new(AtomicU64::new(0));
        let ticker = if self.external_ticks {
            None
        } else {
            Some(Ticker::start(
                engine.clone(),
                Arc::clone(&ticks),
                self.epoch_period,
            ))
        };
        Ok(Engine {
            inner: Arc::new(EngineInner {
                engine,
                linker,
                config: self,
                ticks,
                ticker,
            }),
        })
    }
}

/// A compiler, a linker, and an epoch clock, shared by every module and
/// instance of a process.
///
/// A clone is one reference count increment.
#[derive(Clone)]
pub struct Engine {
    inner: Arc<EngineInner>,
}

struct EngineInner {
    engine: wasmtime::Engine,
    linker: Linker<HostState>,
    config: EngineConfig,
    ticks: Arc<AtomicU64>,
    ticker: Option<Ticker>,
}

impl Engine {
    /// An engine with the default [`EngineConfig`].
    ///
    /// # Errors
    ///
    /// Returns [`Error::Instantiate`] when wasmtime rejects the configuration.
    pub fn new() -> Result<Self, Error> {
        EngineConfig::new().build()
    }

    /// How often the epoch advances.
    pub fn epoch_period(&self) -> Duration {
        self.inner.config.epoch_period
    }

    /// Whether fuel metering is on.
    pub fn fuel_enabled(&self) -> bool {
        self.inner.config.fuel_enabled
    }

    /// Whether the engine runs its own ticker thread.
    pub fn has_ticker(&self) -> bool {
        self.inner.ticker.is_some()
    }

    /// Advances the epoch by one tick.
    ///
    /// The built in ticker calls this every epoch period.
    /// Call it yourself only when you built the engine with external ticks.
    pub fn increment_epoch(&self) {
        self.inner.engine.increment_epoch();
        self.inner.ticks.fetch_add(1, Ordering::Relaxed);
    }

    /// How many ticks the epoch has advanced since the engine was built.
    pub fn ticks(&self) -> u64 {
        self.inner.ticks.load(Ordering::Relaxed)
    }

    pub(crate) fn wasmtime(&self) -> &wasmtime::Engine {
        &self.inner.engine
    }

    pub(crate) fn linker(&self) -> &Linker<HostState> {
        &self.inner.linker
    }
}

/// The thread that advances the epoch on a fixed period.
///
/// Dropping it stops the thread at once and joins it.
struct Ticker {
    stop: Arc<(Mutex<bool>, Condvar)>,
    handle: Option<JoinHandle<()>>,
}

impl Ticker {
    fn start(engine: wasmtime::Engine, ticks: Arc<AtomicU64>, period: Duration) -> Self {
        let stop = Arc::new((Mutex::new(false), Condvar::new()));
        let shared = Arc::clone(&stop);
        let handle = std::thread::spawn(move || {
            let (lock, wake) = &*shared;
            let mut stopped = lock.lock().unwrap_or_else(PoisonError::into_inner);
            loop {
                let (guard, waited) = wake
                    .wait_timeout_while(stopped, period, |stopped| !*stopped)
                    .unwrap_or_else(PoisonError::into_inner);
                stopped = guard;
                if *stopped {
                    break;
                }
                if waited.timed_out() {
                    engine.increment_epoch();
                    ticks.fetch_add(1, Ordering::Relaxed);
                }
            }
        });
        Self {
            stop,
            handle: Some(handle),
        }
    }
}

impl Drop for Ticker {
    fn drop(&mut self) {
        let (lock, wake) = &*self.stop;
        *lock.lock().unwrap_or_else(PoisonError::into_inner) = true;
        wake.notify_all();
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    fn assert_send_sync<T: Send + Sync>() {}

    const _: () = {
        let _ = assert_send_sync::<Engine>;
    };

    fn wait_for_ticks(engine: &Engine, count: u64) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while engine.ticks() < count && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn a_default_engine_has_a_ticker_and_the_default_period() {
        // Arrange
        let config = EngineConfig::new();

        // Act
        let engine = Engine::new().unwrap();

        // Assert
        assert!(engine.has_ticker());
        assert_eq!(engine.epoch_period(), Duration::from_millis(10));
        assert!(!engine.fuel_enabled());
        assert_eq!(config, EngineConfig::default());
    }

    #[test]
    fn the_ticker_advances_the_epoch_on_its_own() {
        // Arrange
        let engine = EngineConfig::new()
            .with_epoch_period(Duration::from_millis(1))
            .build()
            .unwrap();
        wait_for_ticks(&engine, 3);

        // Act
        let ticks = engine.ticks();

        // Assert
        assert!(ticks >= 3);
    }

    #[test]
    fn external_ticks_advance_only_when_asked() {
        // Arrange
        let engine = EngineConfig::new()
            .with_external_ticks(true)
            .build()
            .unwrap();
        let before = engine.ticks();

        // Act
        engine.increment_epoch();

        // Assert
        assert_eq!((before, engine.ticks()), (0, 1));
        assert!(!engine.has_ticker());
    }

    #[test]
    fn a_clone_shares_the_tick_counter() {
        // Arrange
        let engine = EngineConfig::new()
            .with_external_ticks(true)
            .build()
            .unwrap();
        let clone = engine.clone();

        // Act
        clone.increment_epoch();

        // Assert
        assert_eq!(engine.ticks(), 1);
        assert_eq!(engine.epoch_period(), clone.epoch_period());
    }

    #[test]
    fn a_zero_epoch_period_is_rejected() {
        // Arrange
        let config = EngineConfig::new().with_epoch_period(Duration::ZERO);

        // Act
        let result = config.build();

        // Assert
        assert!(matches!(result, Err(Error::Config { .. })));
    }

    #[test]
    fn the_stack_setter_accepts_a_size_and_none() {
        // Arrange
        let base = EngineConfig::new();

        // Act
        let configs = [
            base.clone().with_max_wasm_stack(1 << 20),
            base.clone().with_max_wasm_stack(None),
        ];

        // Assert
        assert_eq!(configs[0].max_wasm_stack, Some(1 << 20));
        assert_eq!(configs[1], base);
        assert!(configs[0].clone().build().is_ok());
    }

    #[test]
    fn dropping_the_engine_stops_the_ticker_within_one_period() {
        // Arrange
        let engine = EngineConfig::new()
            .with_epoch_period(Duration::from_secs(5))
            .build()
            .unwrap();
        let started = Instant::now();

        // Act
        drop(engine);

        // Assert
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn the_linker_defines_the_abi_host_functions() {
        // Arrange
        let engine = EngineConfig::new()
            .with_external_ticks(true)
            .build()
            .unwrap();
        let wat = r#"(module
            (import "env" "proxy_log" (func (param i32 i32 i32) (result i32)))
            (import "env" "proxy_call_foreign_function" (func (param i32 i32 i32 i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 1024))"#;

        // Act
        let result = crate::runtime::test_support::instance(&engine, wat);

        // Assert
        assert!(result.is_ok());
    }
}
