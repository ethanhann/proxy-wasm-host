//! The runtime below the ABI.
//!
//! An [`Engine`] compiles a [`Module`], and an [`Instance`] runs it inside a
//! store.
//! You supply the services a guest needs through [`HostServices`], and
//! [`Limits`] bound the CPU time, the fuel, and the memory of each instance.
//! Host functions read and write guest memory through [`GuestPtr`],
//! [`GuestSlice`], and [`GuestMemory`], so every access is bounds checked.

mod alloc;
mod engine;
mod guest_call;
mod host_state;
mod instance;
mod limits;
mod memory;
mod module;
mod services;
mod wasi;

#[cfg(test)]
pub(crate) mod test_support;

pub use engine::{Engine, EngineConfig};
pub use instance::Instance;
pub use limits::Limits;
pub use memory::{GuestMemory, GuestPtr, GuestSlice};
pub use module::Module;
pub use services::{Clock, HostServices, LogSink, SystemClock};

pub(crate) use alloc::write_return;
pub(crate) use host_state::HostState;
pub(crate) use memory::split;

#[cfg(test)]
pub(crate) use guest_call::map_guest_error;
#[cfg(test)]
pub(crate) use wasi::WASI_FUNCTIONS;
