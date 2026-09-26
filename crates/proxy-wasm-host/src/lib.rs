//! A Proxy-Wasm ABI v0.2.1 host library built on wasmtime.
//!
//! This crate lets a proxy written in Rust load and run Proxy-Wasm guests.
//! It is a port of the Go host at
//! <https://github.com/mosn/proxy-wasm-go-host>.
//!
//! The ABI enumerations live under [`abi::v0_2_1::types`].
//! You lend your own header maps and buffers to the crate through the
//! [`HeaderMap`] and [`Buffer`] traits.
//! [`Engine`], [`Module`], and [`Limits`] compile a guest and bound its
//! resources.
//! [`abi::v0_2_1::Host`] links the host functions of the ABI on an engine.
//! [`abi::v0_2_1::Guest`] binds a guest to the ABI and drives its callbacks.
//!
//! # Example
//!
//! A guest with no callbacks starts and serves one request with the default
//! answers:
//!
//! ```
//! use std::sync::Arc;
//!
//! use proxy_wasm_host::abi::v0_2_1::types::{Action, LogLevel, MapType, Status};
//! use proxy_wasm_host::abi::v0_2_1::{
//!     Access, Guest, GuestError, Host, Invocation, LogContext, LogSink, PluginConfig, Started,
//!     StreamKind, StreamState, VmServices,
//! };
//! use proxy_wasm_host::{Engine, HeaderMap, Limits, Module, VecHeaderMap};
//!
//! struct Stderr;
//! impl LogSink for Stderr {
//!     fn log(&self, _: LogContext<'_>, _: LogLevel, message: &[u8]) {
//!         eprintln!("{}", String::from_utf8_lossy(message));
//!     }
//! }
//!
//! struct Request {
//!     headers: VecHeaderMap,
//! }
//! impl StreamState for Request {
//!     fn header_map(&mut self, _: Invocation, _: Access, map: MapType) -> Result<&mut dyn HeaderMap, Status> {
//!         match map {
//!             MapType::HttpRequestHeaders => Ok(&mut self.headers),
//!             _ => Err(Status::NotFound),
//!         }
//!     }
//! }
//!
//! # fn main() -> Result<(), GuestError> {
//! let wat = r#"(module
//!     (memory (export "memory") 1)
//!     (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 1024)
//!     (func (export "proxy_abi_version_0_2_1")))"#;
//! let engine = Engine::new()?;
//! let module = Module::new(&engine, &wat::parse_str(wat).unwrap())?;
//! let host = Host::new(&engine)?;
//! let services = VmServices::new(Arc::new(Stderr));
//! let mut guest = Guest::new(&host, &module, services, &Limits::default())?;
//!
//! let Started::Serving(root) = guest.start(PluginConfig::new())? else {
//!     panic!("the plugin refused its start");
//! };
//! let request = Request { headers: VecHeaderMap::default() };
//! let (answer, request) = guest.with(request, |scope| {
//!     let stream = scope.on_context_create(Some(root))?;
//!     scope.expect_stream_kind(stream, StreamKind::Http)?;
//!     let action = scope.on_request_headers(stream, 0, true)?;
//!     scope.on_done(stream)?;
//!     scope.on_log(stream)?;
//!     scope.on_delete(stream)?;
//!     Ok::<_, GuestError>(action)
//! });
//! assert_eq!(answer?, Action::Continue);
//! assert!(request.headers.is_empty());
//! # Ok(())
//! # }
//! ```
//!
//! # The surface
//!
//! What no ABI version owns is exported here, at the crate root.
//! You build and configure with [`Engine`], [`EngineConfig`], [`Module`], and
//! [`Limits`].
//! You lend your own storage through [`Buffer`], [`HeaderMap`], and
//! [`VecHeaderMap`].
//! You walk the pairs of a map with a [`PairVisitor`], and [`HeaderMapExt`]
//! reads a map through the trait.
//! You refuse a write with [`NotAllowed`].
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
mod buffer;
mod codec;
mod error;
mod header_map;
mod runtime;

pub use abi::AbiVersion;
pub use buffer::Buffer;
pub use codec::pairs::PairVisitor;
pub use error::{Error, Limit, MemoryError};
pub use header_map::{HeaderMap, HeaderMapExt, VecHeaderMap};
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
