//! Test doubles that the ABI layer's tests share.

pub(crate) mod doubles;
pub(crate) mod services;
pub(crate) mod stream;

pub(crate) use stream::RecordingStream;

use crate::abi::v0_2_1::host_functions::Failure;
use crate::abi::v0_2_1::types::Status;
use crate::abi::v0_2_1::{Callback, ContextId};
use crate::runtime::test_support::{RecordingSink, instance, wat_bytes};
use crate::runtime::{Engine, GuestPtr, GuestSlice, HostServices, Instance, Limits, Module};

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
        .set_stream_host(Box::new(stream));
    (instance, root)
}

/// An instance of `wat` with a root context, an effective context, and no
/// stream host.
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

/// An instance of `wat` with no context and no stream host, as a guest sees
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
    let services = HostServices::new(std::sync::Arc::new(RecordingSink::default()))
        .with_vm_id(VM_ID.to_vec())
        .with_shared(shared);
    let mut instance = Instance::new(engine, &module, services, &Limits::default()).unwrap();
    let state = instance.state_mut();
    let root = state.abi_mut().contexts_mut().create(None).unwrap();
    state.abi_mut().contexts_mut().set_effective(root);
    state
        .abi_mut()
        .set_current_callback(Some(Callback::RequestHeaders));
    (instance, root)
}

/// A call from the request header callback on context one.
pub(crate) fn call() -> crate::abi::v0_2_1::HostCall {
    crate::abi::v0_2_1::HostCall::new(
        ContextId::try_from(1).unwrap(),
        Some(Callback::RequestHeaders),
    )
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
