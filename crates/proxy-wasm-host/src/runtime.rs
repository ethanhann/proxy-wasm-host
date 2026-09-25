//! The runtime below the ABI.
//!
//! An [`Engine`] compiles a [`Module`], and a guest runs it inside a store.
//! [`Limits`] bound the CPU time, the fuel, and the memory of each guest.
//! The ABI layer supplies the linker and an opaque state for each instance,
//! so nothing here refers to an ABI version.

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
mod opt_level;

#[cfg(test)]
pub(crate) mod test_support;

pub use engine::{Engine, EngineConfig};
pub(crate) use instance::Instance;
pub use limits::Limits;
pub(crate) use memory::{GuestMemory, GuestPtr, GuestSlice};
pub use module::Module;
pub use opt_level::OptLevel;

pub(crate) use alloc::{write_optional_return, write_return};
pub(crate) use host_state::HostState;
pub(crate) use memory::split;

#[cfg(test)]
pub(crate) use guest_call::map_guest_error;
