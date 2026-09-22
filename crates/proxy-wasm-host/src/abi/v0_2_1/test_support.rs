//! Test doubles that the ABI layer's tests share.

pub(crate) mod callouts;
pub(crate) mod doubles;
pub(crate) mod events;
pub(crate) mod services;
pub(crate) mod stream;

pub(crate) use stream::RecordingStream;

use std::sync::{Arc, Mutex, PoisonError};

use crate::Error;
use crate::abi::v0_2_1::AbiAccess;
use crate::abi::v0_2_1::Host;
use crate::abi::v0_2_1::LogSink;
use crate::abi::v0_2_1::VmServices;
use crate::abi::v0_2_1::host_functions::Failure;
use crate::abi::v0_2_1::types::LogLevel;
use crate::abi::v0_2_1::types::Status;
use crate::abi::v0_2_1::{Callback, ContextId};
use crate::runtime::{Engine, EngineConfig, GuestPtr, GuestSlice, Instance, Limits, Module};

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

/// The binary form of a guest written in the text format.
pub(crate) fn wat_bytes(wat: &str) -> Vec<u8> {
    wat::parse_str(wat).unwrap()
}

/// An instance of `module` with the host functions linked, the given
/// services, and the default limits.
pub(crate) fn instance_with(
    engine: &Engine,
    module: &Module,
    services: VmServices,
) -> Result<Instance, Error> {
    let host = Host::new(engine)?;
    Instance::new(
        engine,
        host.linker(),
        module,
        crate::abi::v0_2_1::state(services),
        &Limits::default(),
    )
}

/// An instance of `wat` with a recording sink and the default limits.
pub(crate) fn instance(engine: &Engine, wat: &str) -> Result<Instance, Error> {
    let module = Module::new(engine, &wat_bytes(wat))?;
    instance_from(engine, &module)
}

/// An instance of `module` with a recording sink and the default limits.
pub(crate) fn instance_from(engine: &Engine, module: &Module) -> Result<Instance, Error> {
    instance_with(engine, module, services())
}

/// An instance of `wat` whose sink the test keeps.
pub(crate) fn instance_with_sink(
    engine: &Engine,
    wat: &str,
    sink: Arc<RecordingSink>,
) -> Result<Instance, Error> {
    let module = Module::new(engine, &wat_bytes(wat))?;
    instance_with(engine, &module, VmServices::new(sink))
}

/// The status an `i32` from a host function wrapper stands for.
pub(crate) fn status(value: i32) -> Status {
    Status::try_from(value).unwrap()
}

/// The status a body's result stands for, when it did not unwind.
pub(crate) fn outcome(result: Result<(), Failure>) -> Status {
    match result {
        Ok(()) => Status::Ok,
        Err(Failure::Status(status)) => status,
        Err(Failure::Unwind(error)) => panic!("the body unwound: {error}"),
    }
}

/// An instance of `wat` with `stream` installed, a root context, and the
/// effective context a callback would have set.
pub(crate) fn hosted(engine: &Engine, wat: &str, stream: RecordingStream) -> (Instance, ContextId) {
    let (mut instance, root) = unhosted(engine, wat);
    instance
        .state_mut()
        .abi_mut()
        .set_stream_state(Box::new(stream));
    (instance, root)
}

/// An instance of `wat` with `stream` installed and the limits given, for a
/// test of a limit an embedder chose.
pub(crate) fn hosted_with_limits(
    engine: &Engine,
    wat: &str,
    stream: RecordingStream,
    limits: &Limits,
) -> (Instance, ContextId) {
    let module = Module::new(engine, &wat_bytes(wat)).unwrap();
    let host = Host::new(engine).unwrap();
    let mut instance = Instance::new(
        engine,
        host.linker(),
        &module,
        crate::abi::v0_2_1::state(services()),
        limits,
    )
    .unwrap();
    let state = instance.state_mut();
    let root = state.abi_mut().contexts_mut().create(None).unwrap();
    state.abi_mut().contexts_mut().set_effective(root);
    state
        .abi_mut()
        .set_current_callback(Some(Callback::RequestHeaders));
    state.abi_mut().set_stream_state(Box::new(stream));
    (instance, root)
}

/// An instance of `wat` with a root context, an effective context, and no
/// stream state.
pub(crate) fn unhosted(engine: &Engine, wat: &str) -> (Instance, ContextId) {
    let mut instance = instance(engine, wat).unwrap();
    let state = instance.state_mut();
    let root = state.abi_mut().contexts_mut().create(None).unwrap();
    state.abi_mut().contexts_mut().set_effective(root);
    state
        .abi_mut()
        .set_current_callback(Some(Callback::RequestHeaders));
    (instance, root)
}

/// An instance of `wat` with no context and no stream state, as a guest sees
/// before any callback has run.
pub(crate) fn bare(engine: &Engine, wat: &str) -> Instance {
    instance(engine, wat).unwrap()
}

/// The VM id that the shared tests separate their state by.
pub(crate) const VM_ID: &[u8] = b"vm-1";

/// An instance of `wat` with the VM id, the given shared services, a root
/// context, and the effective context a callback would have set.
pub(crate) fn shared_hosted(
    engine: &Engine,
    wat: &str,
    shared: std::sync::Arc<dyn crate::abi::v0_2_1::SharedServices>,
) -> (Instance, ContextId) {
    let module = Module::new(engine, &wat_bytes(wat)).unwrap();
    let services = VmServices::new(std::sync::Arc::new(RecordingSink::default()))
        .with_vm_id(VM_ID.to_vec())
        .with_shared(shared);
    let mut instance = instance_with(engine, &module, services).unwrap();
    let state = instance.state_mut();
    let root = state.abi_mut().contexts_mut().create(None).unwrap();
    state.abi_mut().contexts_mut().set_effective(root);
    state
        .abi_mut()
        .set_current_callback(Some(Callback::RequestHeaders));
    (instance, root)
}

/// A call from the request header callback on context one.
pub(crate) fn call() -> crate::abi::v0_2_1::Invocation {
    crate::abi::v0_2_1::Invocation::new(
        crate::abi::v0_2_1::GuestId::next(),
        ContextId::try_from(1).unwrap(),
    )
    .with_callback(Callback::RequestHeaders)
}

/// The bytes a host function returned through the two pointers at `data` and
/// `size`.
pub(crate) fn returned(instance: &mut Instance, data: u32, size: u32) -> Vec<u8> {
    let memory = instance.memory().unwrap();
    let address = memory.read_u32(GuestPtr::from_address(data)).unwrap();
    let length = memory.read_u32(GuestPtr::from_address(size)).unwrap();
    if length == 0 {
        return Vec::new();
    }
    let slice = GuestSlice::new(GuestPtr::from_address(address), length).unwrap();
    memory.read(slice).unwrap().to_vec()
}

/// Writes `bytes` into guest memory at `at` and reports the pair a host
/// function takes.
pub(crate) fn write(instance: &mut Instance, at: i32, bytes: &[u8]) -> (i32, i32) {
    let len = i32::try_from(bytes.len()).unwrap();
    let slice = GuestSlice::try_from((at, len)).unwrap();
    instance.memory().unwrap().write(slice, bytes).unwrap();
    (at, len)
}

/// A guest that imports every row of the host function table.
///
/// The module comes from the table, so a row added or removed reaches the
/// tests that instantiate it.
/// It exports no callable function, so a test that needs its own body writes
/// its own module.
pub(crate) fn import_everything() -> String {
    use std::fmt::Write as _;

    use crate::abi::v0_2_1::host_functions::table::{HOST_FUNCTIONS, WasmType};

    fn wat_type(ty: WasmType) -> &'static str {
        match ty {
            WasmType::I32 => "i32",
            WasmType::I64 => "i64",
        }
    }

    let mut wat = String::from("(module\n");
    for function in HOST_FUNCTIONS {
        let params: Vec<&str> = function.params.iter().copied().map(wat_type).collect();
        writeln!(
            wat,
            "  (import \"env\" \"{}\" (func (param {}) (result i32)))",
            function.name,
            params.join(" ")
        )
        .unwrap();
    }
    wat.push_str(
        r#"  (memory (export "memory") 1)
  (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 1024)
  (func (export "proxy_abi_version_0_2_1")))"#,
    );
    wat
}

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

/// Services with a fresh recording sink.
pub(crate) fn services() -> VmServices {
    VmServices::new(Arc::new(RecordingSink::default()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::v0_2_1::host_functions::table::HOST_FUNCTIONS;

    #[test]
    fn the_generated_guest_imports_one_function_per_table_row() {
        // Arrange
        let expected = HOST_FUNCTIONS.len();

        // Act
        let wat = import_everything();

        // Assert
        assert_eq!(wat.matches("(import \"env\"").count(), expected);
        for function in HOST_FUNCTIONS {
            assert!(wat.contains(function.name), "{} is missing", function.name);
        }
    }
}
