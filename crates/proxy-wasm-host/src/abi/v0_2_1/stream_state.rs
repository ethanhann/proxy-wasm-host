//! The per stream state an embedder lends to a guest.

pub(crate) mod invocation;
pub(crate) mod values;

use std::any::Any;

use crate::Buffer;
use crate::abi::v0_2_1::types::{BufferType, MapType, Status, StreamType};
use crate::abi::v0_2_1::unserved::unserved;
use crate::header_map::HeaderMap;
pub use invocation::{Access, Invocation, NoStream};
use values::{ForeignCall, LocalResponse};

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
///
/// Each default body reports the status that the ABI section for that family
/// lists for a resource that is not available.
/// The crate reports the same status when it cannot reach you, which
/// happens when no callback has run, when the guest refused the root context,
/// and when no stream state is installed.
///
/// | Method | Default status | A refusal from the value you return |
/// |---|---|---|
/// | [`header_map`](StreamState::header_map) | [`Status::BadArgument`] | [`Status::BadArgument`] |
/// | [`buffer`](StreamState::buffer) | [`Status::NotFound`] | [`Status::NotFound`] |
/// | [`continue_stream`](StreamState::continue_stream) | [`Status::Unimplemented`] | none |
/// | [`close_stream`](StreamState::close_stream) | [`Status::Unimplemented`] | none |
/// | [`send_local_response`](StreamState::send_local_response) | [`Status::Unimplemented`] | none |
/// | [`property`](StreamState::property) | [`Status::NotFound`] | none |
/// | [`set_property`](StreamState::set_property) | [`Status::NotFound`] | none |
/// | [`call_foreign_function`](StreamState::call_foreign_function) | [`Status::NotFound`] | none |
///
/// A guest built with the Rust SDK ends its stream on any status other than
/// `OK` from most of these functions, and reads `NOT_FOUND` from a buffer or
/// from a property read as an absent value.
/// Every default body reports itself through `tracing` at the warn level,
/// so a method you forgot reaches your log before it reaches a guest.
pub trait StreamState: Any + Send {
    // `Any` lets the scope give your own value back without a downcast of
    // your own, and it requires `Self: 'static`, which wasmtime requires of
    // store data.

    /// The map the guest asked for, or the status to report instead.
    ///
    /// The ABI allows `HttpRequestHeaders` to be read in `proxy_on_log` and
    /// read and written in `proxy_on_request_headers` or while the request
    /// is paused from it, and the same shape holds for the trailer and
    /// response maps and their callbacks.
    /// The crate does not enforce those rules.
    /// You can apply them by matching on `call.callback` and `access`,
    /// and you can refuse a write by returning `Err(NotAllowed)` from the
    /// map's write method, which the guest sees as [`Status::BadArgument`].
    /// `call.context` may name a context this stream does not serve, and
    /// then you should return `Err(Status::NotFound)`.
    /// The default body reports [`Status::BadArgument`], the status for a
    /// map that is not available.
    ///
    /// The crate never asks for [`MapType::HttpCallResponseHeaders`],
    /// [`MapType::HttpCallResponseTrailers`],
    /// [`MapType::GrpcCallInitialMetadata`], or
    /// [`MapType::GrpcCallTrailingMetadata`].
    /// It serves each one from the value you gave to the delivery that
    /// holds it, such as
    /// [`CallScope::on_http_call_response`](crate::abi::v0_2_1::CallScope::on_http_call_response).
    ///
    /// For example, a stream that serves the request headers only during
    /// `proxy_on_request_headers` and read only during `proxy_on_log`:
    ///
    /// ```
    /// use proxy_wasm_host::abi::v0_2_1::types::{MapType, Status};
    /// use proxy_wasm_host::abi::v0_2_1::{Access, Callback, Invocation, StreamState};
    /// use proxy_wasm_host::{HeaderMap, VecHeaderMap};
    ///
    /// struct Request {
    ///     headers: VecHeaderMap,
    /// }
    ///
    /// impl StreamState for Request {
    ///     fn header_map(&mut self, call: Invocation, access: Access, map: MapType) -> Result<&mut dyn HeaderMap, Status> {
    ///         match (map, call.callback, access) {
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
    fn header_map(
        &mut self,
        call: Invocation,
        access: Access,
        map: MapType,
    ) -> Result<&mut dyn HeaderMap, Status> {
        unserved("header_map");
        let _ = (call, access, map);
        Err(Status::BadArgument)
    }

    /// The buffer the guest asked for, or the status to report instead.
    ///
    /// The ABI allows each buffer only in named callbacks.
    /// `HttpRequestBody` and `HttpResponseBody` are read and written in the
    /// body callback of their own direction, or while that direction is
    /// paused from it.
    /// `DownstreamData` and `UpstreamData` are read and written in the data
    /// callbacks.
    /// The crate does not enforce those rules, and you can apply them by
    /// matching on `call.callback` and `access`.
    ///
    /// The crate never asks you for `VmConfiguration`, `PluginConfiguration`,
    /// `HttpCallResponseBody`, `GrpcCallMessage`, or
    /// `ForeignFunctionArguments`, because it serves each one itself from
    /// the value you gave it.
    /// The crate clamps `start` and the length against
    /// [`Buffer::len`](crate::Buffer::len) before it calls your buffer, so a
    /// range you receive is inside it.
    /// You can refuse a write by returning `Err(NotAllowed)` from
    /// [`Buffer::replace`](crate::Buffer::replace), which the guest sees as
    /// [`Status::NotFound`].
    /// The default body reports [`Status::NotFound`], the status for a buffer
    /// that is not available.
    ///
    /// # Errors
    ///
    /// The status you return goes to the guest unchanged, except that
    /// `Err(Status::Ok)` is reported as [`Status::InternalFailure`], because
    /// no buffer was touched.
    fn buffer(
        &mut self,
        call: Invocation,
        access: Access,
        buffer: BufferType,
    ) -> Result<&mut dyn Buffer, Status> {
        unserved("buffer");
        let _ = (call, access, buffer);
        Err(Status::NotFound)
    }

    /// Resumes the stream the guest paused.
    ///
    /// The stream type names which half of the exchange the guest means, and
    /// a guest resumes the half it paused from a callback that returned
    /// [`Action::Pause`](crate::abi::v0_2_1::types::Action::Pause).
    /// The default body reports [`Status::Unimplemented`], which is the
    /// status the ABI names for a stream a host cannot resume.
    ///
    /// # Errors
    ///
    /// The status you return goes to the guest unchanged, except that
    /// `Err(Status::Ok)` is reported as [`Status::InternalFailure`].
    fn continue_stream(&mut self, call: Invocation, stream: StreamType) -> Result<(), Status> {
        unserved("continue_stream");
        let _ = (call, stream);
        Err(Status::Unimplemented)
    }

    /// Closes or resets the stream.
    ///
    /// The stream type names which half of the exchange the guest means.
    /// The default body reports [`Status::Unimplemented`].
    ///
    /// # Errors
    ///
    /// The status you return goes to the guest unchanged, except that
    /// `Err(Status::Ok)` is reported as [`Status::InternalFailure`].
    fn close_stream(&mut self, call: Invocation, stream: StreamType) -> Result<(), Status> {
        unserved("close_stream");
        let _ = (call, stream);
        Err(Status::Unimplemented)
    }

    /// Sends a response in place of the upstream one.
    ///
    /// The ABI allows the call while the response headers have not gone
    /// downstream, and it says nothing about a second call, so you decide
    /// what one means.
    /// The response borrows guest memory, so call
    /// [`LocalResponse::into_owned`] if you send it after the callback
    /// returns.
    /// The default body reports [`Status::Unimplemented`].
    ///
    /// # Errors
    ///
    /// The status you return goes to the guest unchanged, except that
    /// `Err(Status::Ok)` is reported as [`Status::InternalFailure`].
    fn send_local_response(
        &mut self,
        call: Invocation,
        response: LocalResponse<'_>,
    ) -> Result<(), Status> {
        unserved("send_local_response");
        let _ = (call, response);
        Err(Status::Unimplemented)
    }

    /// The value of a property, or the status to report instead.
    ///
    /// The path arrives as the segments the guest serialized, so the path
    /// `route.name` arrives as two slices.
    /// The crate answers `plugin_name`, `plugin_root_id`, and
    /// `plugin_vm_id` itself from the plugin of the root and from the host
    /// services, so you never see those three.
    /// The ABI says that properties are particular to a host, so you decide
    /// which ones you serve.
    ///
    /// # Errors
    ///
    /// Report [`Status::NotFound`] for a path you do not serve, which the
    /// default body does, and [`Status::SerializationFailure`] for a value
    /// you hold and cannot serialize.
    /// A guest built with the Rust SDK reads `NOT_FOUND` as an absent value
    /// and stops on anything else.
    fn property(&mut self, call: Invocation, path: &[&[u8]]) -> Result<Vec<u8>, Status> {
        unserved("property");
        let _ = (call, path);
        Err(Status::NotFound)
    }

    /// Writes a property.
    ///
    /// The crate refuses a write to the three properties it answers itself,
    /// so you never see those.
    ///
    /// # Errors
    ///
    /// Report [`Status::NotFound`] for a path you do not serve, which the
    /// default body does.
    /// A guest built with the Rust SDK stops on any status but `OK`, so
    /// serve this if your guests write properties.
    fn set_property(
        &mut self,
        call: Invocation,
        path: &[&[u8]],
        value: &[u8],
    ) -> Result<(), Status> {
        unserved("set_property");
        let _ = (call, path, value);
        Err(Status::NotFound)
    }

    /// Runs a function of yours that the ABI does not name.
    ///
    /// The result may be empty, and the guest reads an empty result as a
    /// present value of no bytes.
    ///
    /// # Errors
    ///
    /// Report [`Status::NotFound`] for a name you do not serve, which the
    /// default body does.
    fn call_foreign_function(
        &mut self,
        call: Invocation,
        request: ForeignCall<'_>,
    ) -> Result<Vec<u8>, Status> {
        unserved("call_foreign_function");
        let _ = (call, request);
        Err(Status::NotFound)
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use super::*;
    use crate::abi::v0_2_1::{Callback, ContextId, GuestId};

    struct Empty;

    impl StreamState for Empty {}

    fn call() -> Invocation {
        Invocation::new(GuestId::next(), ContextId::try_from(1).unwrap())
            .with_callback(Callback::RequestHeaders)
    }

    #[test]
    fn the_default_body_and_no_stream_report_bad_argument() {
        // Arrange
        let mut empty = Empty;
        let mut none = NoStream;

        // Act
        let results = (
            empty
                .header_map(call(), Access::Read, MapType::HttpRequestHeaders)
                .err(),
            none.header_map(call(), Access::Read, MapType::HttpResponseHeaders)
                .err(),
        );

        // Assert
        assert_eq!(
            results,
            (Some(Status::BadArgument), Some(Status::BadArgument))
        );
        assert_eq!(call().callback, Some(Callback::RequestHeaders));
    }

    #[test]
    fn the_default_buffer_reports_not_found() {
        // Arrange
        let mut empty = Empty;
        let mut none = NoStream;

        // Act
        let results = (
            empty
                .buffer(call(), Access::Read, BufferType::HttpRequestBody)
                .err(),
            none.buffer(call(), Access::Read, BufferType::HttpResponseBody)
                .err(),
        );

        // Assert
        assert_eq!(results, (Some(Status::NotFound), Some(Status::NotFound)));
    }

    fn unimplemented_answers(stream: &mut dyn StreamState) -> [Option<Status>; 3] {
        [
            stream
                .continue_stream(call(), StreamType::HttpRequest)
                .err(),
            stream.close_stream(call(), StreamType::HttpRequest).err(),
            stream
                .send_local_response(call(), LocalResponse::new(200))
                .err(),
        ]
    }

    fn not_found_answers(stream: &mut dyn StreamState) -> [Option<Status>; 3] {
        [
            stream.property(call(), &[b"route"]).err(),
            stream.set_property(call(), &[b"route"], b"main").err(),
            stream
                .call_foreign_function(
                    call(),
                    ForeignCall::new(Cow::Borrowed(b"echo"), Cow::Borrowed(b"")),
                )
                .err(),
        ]
    }

    #[test]
    fn the_three_property_and_foreign_defaults_report_not_found() {
        // Arrange
        let mut empty = Empty;
        let mut none = NoStream;

        // Act
        let results = [not_found_answers(&mut empty), not_found_answers(&mut none)];

        // Assert
        assert_eq!(results, [[Some(Status::NotFound); 3]; 2]);
    }

    #[test]
    fn the_other_three_defaults_report_unimplemented() {
        // Arrange
        let mut empty = Empty;
        let mut none = NoStream;

        // Act
        let results = [
            unimplemented_answers(&mut empty),
            unimplemented_answers(&mut none),
        ];

        // Assert
        assert_eq!(results, [[Some(Status::Unimplemented); 3]; 2]);
    }
}
