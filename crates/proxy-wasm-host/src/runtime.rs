//! The runtime below the ABI.
//!
//! An [`Engine`] compiles a [`Module`], and an instance runs it inside a
//! store.
//! The ABI layer supplies the services a guest needs, and
//! [`Limits`] bound the CPU time, the fuel, and the memory of each instance.
//! Host functions read and write guest memory through `GuestPtr`,
//! `GuestSlice`, and `GuestMemory`, so every access is bounds checked.

mod alloc;
mod engine;
mod guest_call;
mod host_state;
mod instance;
#[cfg(test)]
mod layering;
mod limits;
mod memory;
mod module;

#[cfg(test)]
pub(crate) mod test_support;

pub use engine::{Engine, EngineConfig};
pub(crate) use instance::Instance;
pub use limits::Limits;
pub(crate) use memory::{GuestMemory, GuestPtr, GuestSlice};
pub use module::Module;

pub(crate) use alloc::write_return;
pub(crate) use host_state::HostState;
pub(crate) use memory::split;

#[cfg(test)]
pub(crate) use guest_call::map_guest_error;
