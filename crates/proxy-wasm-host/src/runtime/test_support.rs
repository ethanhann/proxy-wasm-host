//! Guests and aids that the tests of this crate share.

use wasmtime::Linker;

use crate::Error;
use crate::runtime::{Engine, EngineConfig, HostState, Instance, Limits, Module};

/// One memory page, a stub allocator that returns 1024, and a `_start`.
pub(crate) const MINIMAL_GUEST: &str = r#"(module
    (memory (export "memory") 1)
    (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 1024)
    (func (export "_start")))"#;

/// An engine whose epoch advances only when a test asks.
pub(crate) fn engine() -> Engine {
    EngineConfig::new()
        .with_external_ticks(true)
        .build()
        .unwrap()
}

/// A linker that offers no import.
pub(crate) fn linker(engine: &Engine) -> Linker<HostState> {
    Linker::new(engine.wasmtime())
}

/// The binary form of a guest written in the text format.
pub(crate) fn wat_bytes(wat: &str) -> Vec<u8> {
    wat::parse_str(wat).unwrap()
}

/// An instance of `wat` with no import, an empty state slot, and the default
/// limits.
pub(crate) fn instance(engine: &Engine, wat: &str) -> Result<Instance, Error> {
    let module = Module::new(engine, &wat_bytes(wat))?;
    instance_from(engine, &module)
}

/// An instance of `module` with no import, an empty state slot, and the
/// default limits.
pub(crate) fn instance_from(engine: &Engine, module: &Module) -> Result<Instance, Error> {
    Instance::new(
        engine,
        &linker(engine),
        module,
        Box::new(()),
        &Limits::default(),
    )
}
