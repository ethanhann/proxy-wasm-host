//! The per stream state an embedder lends to a guest.

use std::any::Any;

use crate::abi::v0_2_1::types::{MapType, Status};
use crate::abi::v0_2_1::{Callback, ContextId};
use crate::header_map::HeaderMap;

/// Whether a host function reads or writes the value it asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Access {
    /// The guest reads.
    Read,
    /// The guest writes.
    Write,
}

/// What the guest is doing when it calls a host function.
///
/// The ABI allows each map only in named callbacks, and only the crate knows
/// which callback is running.
/// A [`StreamHost`] method receives this so that you can apply those rules.
/// Build one with [`HostCall::new`] when you test your own stream host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct HostCall {
    /// The effective context, which the guest may have changed with
    /// `proxy_set_effective_context`.
    pub context: ContextId,
    /// The callback that is running, or `None` when the guest called from
    /// its start sequence or from a raw call.
    pub callback: Option<Callback>,
    /// Whether the guest reads or writes.
    pub access: Access,
}

impl HostCall {
    /// A call on `context` from `callback` with the given access.
    pub fn new(context: ContextId, callback: Option<Callback>, access: Access) -> Self {
        Self {
            context,
            callback,
            access,
        }
    }
}

/// The state of one stream, lent to a guest for a group of callbacks.
///
/// You give an owned value to [`Guest::enter`](super::Guest::enter), the
/// guest's host functions reach it while the scope lives, and
/// [`CallScope::finish`](super::CallScope::finish) gives it back.
/// The value must be `'static`, because wasmtime requires that of store
/// data, so move your request state in and take it back out rather than
/// borrowing it.
/// Every method has a default body, so you implement only what your stream
/// serves.
pub trait StreamHost: Any + Send {
    /// The map the guest asked for, or the status to report instead.
    ///
    /// The ABI allows `HttpRequestHeaders` to be read in `proxy_on_log` and
    /// read and written in `proxy_on_request_headers` or while the request
    /// is paused from it, and the same shape holds for the trailer and
    /// response maps and their callbacks.
    /// The crate does not enforce those rules.
    /// You can apply them by matching on `call.callback` and `call.access`,
    /// and you can refuse a write by returning `Err(NotAllowed)` from the
    /// map's write method, which the guest sees as [`Status::BadArgument`].
    /// `call.context` may name a context this stream does not serve, and
    /// then you should return `Err(Status::NotFound)`.
    /// The default body reports [`Status::BadArgument`], the status for a
    /// map that is not available.
    ///
    /// For example, a stream that serves the request headers only during
    /// `proxy_on_request_headers` and read only during `proxy_on_log`:
    ///
    /// ```
    /// use proxy_wasm_host::abi::v0_2_1::types::{MapType, Status};
    /// use proxy_wasm_host::abi::v0_2_1::{Access, Callback, HostCall, StreamHost};
    /// use proxy_wasm_host::{HeaderMap, VecHeaderMap};
    ///
    /// struct Request {
    ///     headers: VecHeaderMap,
    /// }
    ///
    /// impl StreamHost for Request {
    ///     fn header_map(&mut self, call: HostCall, map: MapType) -> Result<&mut dyn HeaderMap, Status> {
    ///         match (map, call.callback, call.access) {
    ///             (MapType::HttpRequestHeaders, Some(Callback::RequestHeaders), _)
    ///             | (MapType::HttpRequestHeaders, Some(Callback::Log), Access::Read) => {
    ///                 Ok(&mut self.headers)
    ///             }
    ///             _ => Err(Status::BadArgument),
    ///         }
    ///     }
    /// }
    /// ```
    ///
    /// # Errors
    ///
    /// The status you return goes to the guest unchanged, except that
    /// `Err(Status::Ok)` is reported as [`Status::InternalFailure`], because
    /// no map was touched.
    fn header_map(&mut self, call: HostCall, map: MapType) -> Result<&mut dyn HeaderMap, Status> {
        let _ = (call, map);
        Err(Status::BadArgument)
    }
}

/// A stream host that serves nothing, for the callbacks of a root context.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct NoStream;

impl StreamHost for NoStream {}

#[cfg(test)]
mod tests {
    use super::*;

    struct Empty;

    impl StreamHost for Empty {}

    fn call() -> HostCall {
        HostCall::new(
            ContextId::try_from(1).unwrap(),
            Some(Callback::RequestHeaders),
            Access::Read,
        )
    }

    #[test]
    fn the_default_body_and_no_stream_report_bad_argument() {
        // Arrange
        let mut empty = Empty;
        let mut none = NoStream;

        // Act
        let results = (
            empty.header_map(call(), MapType::HttpRequestHeaders).err(),
            none.header_map(call(), MapType::HttpResponseHeaders).err(),
        );

        // Assert
        assert_eq!(
            results,
            (Some(Status::BadArgument), Some(Status::BadArgument))
        );
        assert_eq!(call().callback, Some(Callback::RequestHeaders));
    }
}
