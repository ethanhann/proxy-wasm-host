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
//! The [`runtime`] module compiles and runs a guest, bounds its resources,
//! and gives host functions safe access to guest memory.
//! [`abi::v0_2_1::Guest`] binds an instance to the ABI and drives its
//! callbacks.

pub mod abi;
pub mod buffer;
pub mod codec;
pub mod error;
pub mod header_map;
pub mod runtime;

pub use buffer::Buffer;
pub use error::{Error, Limit, MemoryError};
pub use header_map::{HeaderMap, VecHeaderMap};

/// The embedder refused a write to a header map or a buffer.
///
/// The ABI allows a write to each map and buffer only from named callbacks.
/// If you enforce that rule in your [`HeaderMap`] or [`Buffer`]
/// implementation, return this error from the write.
/// The host function then reports the status that the ABI section for that
/// resource lists when it is not available.
/// A refused map write is `BAD_ARGUMENT` and a refused buffer write is
/// `NOT_FOUND`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the embedder does not allow this write")]
pub struct NotAllowed;
