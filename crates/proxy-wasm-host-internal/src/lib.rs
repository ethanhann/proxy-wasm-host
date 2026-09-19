//! The wasmtime runtime of the `proxy-wasm-host` crate.
//!
//! This crate is internal.
//! You depend on `proxy-wasm-host`, which exports what you need from here.
//! This crate makes no compatibility promise between any two versions.
//!
//! An [`Engine`] compiles a [`Module`], and an [`Instance`] runs it inside a
//! store.
//! [`Limits`] bound the CPU time, the fuel, and the memory of each instance.
//! A host function reads and writes guest memory through [`GuestPtr`],
//! [`GuestSlice`], and [`GuestMemory`], so every access is bounds checked.
//! The layer above supplies the linker and an opaque state for each instance.
//!
//! # The `test-support` feature
//!
//! The feature is for the tests of `proxy-wasm-host`, and it makes no
//! promise.
//! It exports the items that drive a host function with no guest call.

mod alloc;
mod engine;
pub mod error;
mod guest_call;
mod host_state;
mod instance;
mod limits;
mod memory;
mod module;

#[cfg(test)]
pub(crate) mod test_support;

pub use alloc::write_return;
pub use engine::{Engine, EngineConfig};
pub use error::{Error, Limit, MemoryError};
#[cfg(any(test, feature = "test-support"))]
pub use guest_call::map_guest_error;
pub use host_state::HostState;
pub use instance::Instance;
pub use limits::Limits;
pub use memory::{GuestMemory, GuestPtr, GuestSlice, split};
pub use module::Module;
