//! A Proxy-Wasm ABI v0.2.1 host library built on wasmtime.
//!
//! This crate lets a proxy written in Rust load and run Proxy-Wasm guests.
//! It is a port of the Go host at
//! <https://github.com/mosn/proxy-wasm-go-host>.
//!
//! The ABI enumerations live under [`abi::v0_2_1::types`].
//! The serialization rules for maps and property paths live under [`codec`].
//! You lend your own header maps and buffers to the crate through the
//! [`HeaderMap`] and [`Buffer`] traits.
//! The [`runtime`] module compiles a guest and bounds its resources.
//! [`abi::v0_2_1::Host`] links the host functions of the ABI on an engine.
//! [`abi::v0_2_1::Guest`] binds a guest to the ABI and drives its callbacks.
//!
//! # The surface
//!
//! What no ABI version owns is exported here, at the crate root.
//! You build and configure with [`Engine`], [`EngineConfig`], [`Module`], and
//! [`Limits`].
//! You lend your own storage through [`Buffer`], [`HeaderMap`], and
//! [`VecHeaderMap`], and you refuse a write with [`NotAllowed`].
//! You read a failure of the runtime through [`Error`], [`Limit`], and
//! [`MemoryError`].
//! You ask which ABI a module speaks with [`AbiVersion`], and
//! [`abi::UnsupportedAbi`] is the refusal.
//!
//! Everything an ABI version defines is exported from that version's module, so a
//! later version can define its own without a rename here.
//! For v0.2.1 that is [`abi::v0_2_1`], which groups its own surface the same
//! way.

pub mod abi;
pub mod buffer;
pub mod codec;
pub mod error;
pub mod header_map;
pub mod runtime;

pub use abi::AbiVersion;
pub use buffer::Buffer;
pub use error::{Error, Limit, MemoryError};
pub use header_map::{HeaderMap, VecHeaderMap};
pub use runtime::{Engine, EngineConfig, Limits, Module};

/// The embedder refused a write to a header map or a buffer.
///
/// The ABI allows a write to each map and buffer only from the callbacks the ABI lists.
/// If you enforce that rule in your [`HeaderMap`] or [`Buffer`]
/// implementation, return this error from the write.
/// The host function then reports the status that the ABI section for that
/// resource lists when it is not available.
/// A refused map write is `BAD_ARGUMENT` and a refused buffer write is
/// `NOT_FOUND`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the embedder does not allow this write")]
pub struct NotAllowed;
