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
//!
//! The runtime, the host functions, and the callbacks are under construction.

pub mod abi;
pub mod buffer;
pub mod codec;
pub mod header_map;

pub use buffer::Buffer;
pub use header_map::{HeaderMap, VecHeaderMap};

/// The embedder refused a write to a header map or a buffer.
///
/// The ABI allows a write to each map and buffer only from named callbacks.
/// If you enforce that rule in your [`HeaderMap`] or [`Buffer`]
/// implementation, return this error from the write.
/// The host function then reports a status to the guest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the embedder does not allow this write")]
pub struct NotAllowed;
