//! Test doubles that the ABI layer's tests share.

pub(crate) mod doubles;
pub(crate) mod stream;

pub(crate) use stream::RecordingStream;

use crate::abi::v0_2_1::host_functions::Failure;
use crate::abi::v0_2_1::types::Status;
use crate::abi::v0_2_1::{Callback, ContextId};
use crate::runtime::test_support::instance;
use crate::runtime::{Engine, GuestSlice, Instance};

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

/// Writes `bytes` into guest memory at `at` and reports the pair a host
/// function takes.
pub(crate) fn write(instance: &mut Instance, at: i32, bytes: &[u8]) -> (i32, i32) {
    let len = i32::try_from(bytes.len()).unwrap();
    let slice = GuestSlice::try_from((at, len)).unwrap();
    instance.memory().unwrap().write(slice, bytes).unwrap();
    (at, len)
}
