//! Guests and services that the runtime tests share.

use std::sync::{Arc, Mutex, PoisonError};

use crate::Error;
use crate::abi::v0_2_1::types::LogLevel;
use crate::runtime::{Engine, EngineConfig, Instance, Limits, LogSink, Module, VmServices};

/// One memory page, a stub allocator that returns 1024, and a `_start`.
pub(crate) const MINIMAL_GUEST: &str = r#"(module
    (memory (export "memory") 1)
    (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 1024)
    (func (export "_start")))"#;

/// A log sink that records every message.
#[derive(Default)]
pub(crate) struct RecordingSink {
    entries: Mutex<Vec<(LogLevel, Vec<u8>)>>,
}

impl RecordingSink {
    /// Every message logged so far, in order.
    pub(crate) fn entries(&self) -> Vec<(LogLevel, Vec<u8>)> {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl LogSink for RecordingSink {
    fn log(&self, level: LogLevel, message: &[u8]) {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((level, message.to_vec()));
    }
}

/// An engine whose epoch advances only when a test asks.
pub(crate) fn engine() -> Engine {
    EngineConfig::new()
        .with_external_ticks(true)
        .build()
        .unwrap()
}

/// The binary form of a guest written in the text format.
pub(crate) fn wat_bytes(wat: &str) -> Vec<u8> {
    wat::parse_str(wat).unwrap()
}

/// Services with a fresh recording sink.
pub(crate) fn services() -> VmServices {
    VmServices::new(Arc::new(RecordingSink::default()))
}

/// An instance of `wat` with a recording sink and the default limits.
pub(crate) fn instance(engine: &Engine, wat: &str) -> Result<Instance, Error> {
    let module = Module::new(engine, &wat_bytes(wat))?;
    instance_from(engine, &module)
}

/// An instance of `module` with a recording sink and the default limits.
pub(crate) fn instance_from(engine: &Engine, module: &Module) -> Result<Instance, Error> {
    Instance::new(engine, module, services(), &Limits::default())
}

/// An instance of `wat` whose sink the test keeps.
pub(crate) fn instance_with_sink(
    engine: &Engine,
    wat: &str,
    sink: Arc<RecordingSink>,
) -> Result<Instance, Error> {
    let module = Module::new(engine, &wat_bytes(wat))?;
    Instance::new(engine, &module, VmServices::new(sink), &Limits::default())
}
