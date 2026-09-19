//! The runtime below the ABI.
//!
//! An [`Engine`] compiles a [`Module`], and a guest runs it inside a store.
//! [`Limits`] bound the CPU time, the fuel, and the memory of each guest.

#[doc(inline)]
pub use proxy_wasm_host_internal::{Engine, EngineConfig, Limits, Module};

#[cfg(test)]
pub(crate) use proxy_wasm_host_internal::map_guest_error;
pub(crate) use proxy_wasm_host_internal::{
    GuestMemory, GuestPtr, GuestSlice, HostState, Instance, split, write_return,
};
