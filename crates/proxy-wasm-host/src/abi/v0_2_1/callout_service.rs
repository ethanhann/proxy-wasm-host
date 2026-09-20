//! The service that receives the callouts of a guest, and the values it
//! exchanges with the crate.

mod grpc;
mod http;
mod response;

pub use grpc::{GrpcCall, GrpcOpenRefusal, GrpcStatus, GrpcStream};
pub use http::{HttpCall, HttpCallRefusal};
pub use response::HttpCallResponse;

use crate::abi::v0_2_1::unserved::unserved;
use crate::abi::v0_2_1::{CalloutId, Invocation};

/// The service that receives the callouts of a guest.
///
/// A guest that acts on a callout of its own which already ended reads
/// `OK`, and the crate asks you nothing, because a guest of the Rust SDK
/// stops on any other answer.
/// A guest can therefore learn that a number was given out in this guest,
/// and it learns nothing else about the callout.
///
/// Sometimes a guest needs a second server, for example to check a token.
/// A guest cannot open a connection, so it asks the host with
/// `proxy_http_call`, and the crate gives the request to this service.
/// You send the request with your own client, and you give the response to
/// the guest with
/// [`CallScope::on_http_call_response`](crate::abi::v0_2_1::CallScope::on_http_call_response).
///
/// You give the service to
/// [`VmServices::with_callouts`](crate::abi::v0_2_1::VmServices::with_callouts),
/// so one value serves every callback of a guest, which includes the
/// callbacks of a root context such as `proxy_on_tick`.
///
/// A guest asks a gRPC server in the same way, with `proxy_grpc_call` for
/// one question and `proxy_grpc_stream` for a conversation, and the five
/// gRPC methods below belong together.
/// A service that opens a gRPC callout and ignores [`Callouts::grpc_cancel`]
/// leaves its own request running after the guest gave up.
///
/// The guest waits inside the call, so a method must return at once.
/// Keep what you need with [`HttpCall::into_owned`], start the request
/// elsewhere, and return.
///
/// The identifiers of a guest start at one, so key your own record by
/// [`Invocation::guest`](crate::abi::v0_2_1::Invocation) and the callout, and
/// never by the context alone.
/// A guest may send, cancel, or close inside a delivery you are running, so
/// the crate can call this service from your own thread while you deliver.
/// Hold no lock of your own on that callout while you deliver.
///
/// For example, a service that accepts every call to one upstream:
///
/// ```
/// use std::sync::Mutex;
/// use proxy_wasm_host::abi::v0_2_1::{
///     CalloutId, Callouts, HttpCall, HttpCallRefusal, Invocation,
/// };
///
/// #[derive(Default)]
/// struct Outbox(Mutex<Vec<(CalloutId, HttpCall<'static>)>>);
///
/// impl Callouts for Outbox {
///     fn http_call(
///         &self,
///         _: Invocation,
///         callout: CalloutId,
///         request: HttpCall<'_>,
///     ) -> Result<(), HttpCallRefusal> {
///         if request.upstream.as_ref() != b"authz" {
///             return Err(HttpCallRefusal::UnknownUpstream);
///         }
///         let mut outbox = self.0.lock().map_err(|_| HttpCallRefusal::Failed)?;
///         outbox.push((callout, request.into_owned()));
///         Ok(())
///     }
/// }
/// ```
pub trait Callouts: Send + Sync {
    /// Accepts or refuses an HTTP call.
    ///
    /// `call.context` is the context that made the call, which you name when
    /// you deliver the response.
    /// The callout is open from the moment you return `Ok`, and it ends when
    /// you deliver a response or when its context is deleted.
    ///
    /// # Errors
    ///
    /// Return [`HttpCallRefusal::UnknownUpstream`] for an upstream you do not
    /// know, and [`HttpCallRefusal::Failed`] when you cannot send the request.
    /// The default body refuses with `Failed` and reports itself through
    /// `tracing` at the warn level.
    fn http_call(
        &self,
        call: Invocation,
        callout: CalloutId,
        request: HttpCall<'_>,
    ) -> Result<(), HttpCallRefusal> {
        let _ = (call, callout, request);
        unserved("http_call");
        Err(HttpCallRefusal::Failed)
    }
    /// Accepts or refuses a gRPC call, which sends one message and gets one
    /// answer.
    ///
    /// The callout is open from the moment you return `Ok`.
    /// It ends when you deliver
    /// [`CallScope::on_grpc_receive`](crate::abi::v0_2_1::CallScope::on_grpc_receive)
    /// or
    /// [`CallScope::on_grpc_close`](crate::abi::v0_2_1::CallScope::on_grpc_close),
    /// when the guest cancels or closes it, or when its context is deleted.
    ///
    /// # Errors
    ///
    /// Return [`GrpcOpenRefusal::UnknownUpstream`] for an upstream you do not
    /// know, and [`GrpcOpenRefusal::Failed`] when you cannot send the call.
    /// The default body refuses with `Failed` and reports itself through
    /// `tracing` at the warn level.
    ///
    /// `call.context` is the context that made the callout, which you name
    /// when you deliver a result.
    fn grpc_call(
        &self,
        call: Invocation,
        callout: CalloutId,
        request: GrpcCall<'_>,
    ) -> Result<(), GrpcOpenRefusal> {
        let _ = (call, callout, request);
        unserved("grpc_call");
        Err(GrpcOpenRefusal::Failed)
    }

    /// Accepts or refuses a gRPC stream, which stays open for many messages.
    ///
    /// The guest sends on the stream through [`Callouts::grpc_send`], and you
    /// give it messages and metadata through the deliveries of
    /// [`CallScope`](crate::abi::v0_2_1::CallScope).
    /// The callout ends when you deliver
    /// [`CallScope::on_grpc_close`](crate::abi::v0_2_1::CallScope::on_grpc_close),
    /// when the guest cancels it, or when its context is deleted.
    ///
    /// # Errors
    ///
    /// Return [`GrpcOpenRefusal::UnknownUpstream`] for an upstream you do not
    /// know, and [`GrpcOpenRefusal::Failed`] when you cannot open the stream.
    /// The default body refuses with `Failed` and reports itself through
    /// `tracing` at the warn level.
    ///
    /// `call.context` is the context that made the callout, which you name
    /// when you deliver a result.
    fn grpc_stream(
        &self,
        call: Invocation,
        callout: CalloutId,
        request: GrpcStream<'_>,
    ) -> Result<(), GrpcOpenRefusal> {
        let _ = (call, callout, request);
        unserved("grpc_stream");
        Err(GrpcOpenRefusal::Failed)
    }

    /// Sends one message on a gRPC stream that the guest opened.
    ///
    /// The guest reads `OK` whenever the crate reaches you, so a message you
    /// cannot send is a message you drop.
    /// You then deliver
    /// [`CallScope::on_grpc_close`](crate::abi::v0_2_1::CallScope::on_grpc_close),
    /// which ends the callout.
    /// The reason is that a guest of the Rust SDK stops with a panic on any
    /// other answer.
    ///
    /// `end_of_stream` says that the guest sends no more on this stream.
    /// The server may still send, so the callout stays open until you close
    /// it.
    /// A guest that ends the stream this way reaches [`Callouts::grpc_close`]
    /// no more, so treat this flag as the close of its side.
    ///
    /// `call.context` is the context that opened the callout, because the
    /// crate serves this function for that context alone.
    fn grpc_send(&self, call: Invocation, callout: CalloutId, message: &[u8], end_of_stream: bool) {
        let _ = (call, callout, message, end_of_stream);
        unserved("grpc_send");
    }

    /// Ends a gRPC call or stream that the guest gave up.
    ///
    /// The callout is closed when this returns, and the guest gets no
    /// callback for it, so stop your own request here.
    /// The crate calls this for `proxy_grpc_cancel` of either kind and for
    /// `proxy_grpc_close` of a call.
    ///
    /// A guest of the Rust SDK keeps its own record of a callout that ends
    /// with no delivery, and that record gets no callback.
    ///
    /// `call.context` is the context that opened the callout.
    fn grpc_cancel(&self, call: Invocation, callout: CalloutId) {
        let _ = (call, callout);
        unserved("grpc_cancel");
    }

    /// Reports that the guest sends no more on a gRPC stream.
    ///
    /// The callout stays open, because the server may still send.
    /// You end it when you deliver
    /// [`CallScope::on_grpc_close`](crate::abi::v0_2_1::CallScope::on_grpc_close).
    /// The crate calls this one time for one stream, and a second close
    /// reaches you no more.
    /// A guest that sends with `end_of_stream` and then closes reaches you
    /// through [`Callouts::grpc_send`] alone, because that send already told
    /// you that the guest closed its side.
    ///
    /// `call.context` is the context that opened the callout.
    fn grpc_close(&self, call: Invocation, callout: CalloutId) {
        let _ = (call, callout);
        unserved("grpc_close");
    }
}

/// The service of a guest whose embedder serves no callout.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct NoCallouts;

impl Callouts for NoCallouts {}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use super::*;
    use crate::abi::v0_2_1::test_support::events::warnings;
    use crate::abi::v0_2_1::types::Status;
    use crate::abi::v0_2_1::{ContextId, GuestId};

    fn call() -> Invocation {
        Invocation::new(GuestId::next(), ContextId::try_from(1).unwrap())
    }

    fn callout() -> CalloutId {
        CalloutId::try_from(1_u32).unwrap()
    }

    #[test]
    fn the_default_service_refuses_an_http_call_as_failed() {
        // Arrange
        let request = HttpCall::new(Cow::Borrowed(b"authz"));

        // Act
        let answer = NoCallouts.http_call(call(), callout(), request);

        // Assert
        assert_eq!(answer, Err(HttpCallRefusal::Failed));
    }

    #[test]
    fn the_two_grpc_openers_refuse_as_failed() {
        // Arrange
        let unary = GrpcCall::new(
            Cow::Borrowed(b"authz"),
            Cow::Borrowed(b"svc"),
            Cow::Borrowed(b"Check"),
        );
        let stream = GrpcStream::new(
            Cow::Borrowed(b"authz"),
            Cow::Borrowed(b"svc"),
            Cow::Borrowed(b"Watch"),
        );

        // Act
        let answers = (
            NoCallouts.grpc_call(call(), callout(), unary),
            NoCallouts.grpc_stream(call(), callout(), stream),
        );

        // Assert
        assert_eq!(answers.0, Err(GrpcOpenRefusal::Failed));
        assert_eq!(answers.1, Err(GrpcOpenRefusal::Failed));
    }

    #[test]
    fn every_default_body_warns_one_time() {
        // Arrange
        let service = NoCallouts;
        let message = b"m".as_slice();

        // Act
        let counted = warnings(|| {
            let _ = service.http_call(call(), callout(), HttpCall::new(Cow::Borrowed(b"a")));
            let _ = service.grpc_call(
                call(),
                callout(),
                GrpcCall::new(
                    Cow::Borrowed(b"a"),
                    Cow::Borrowed(b"s"),
                    Cow::Borrowed(b"m"),
                ),
            );
            let _ = service.grpc_stream(
                call(),
                callout(),
                GrpcStream::new(
                    Cow::Borrowed(b"a"),
                    Cow::Borrowed(b"s"),
                    Cow::Borrowed(b"m"),
                ),
            );
            service.grpc_send(call(), callout(), message, true);
            service.grpc_cancel(call(), callout());
            service.grpc_close(call(), callout());
        });

        // Assert
        assert_eq!(counted, 6);
    }

    #[test]
    fn each_grpc_refusal_is_a_status_the_sdk_accepts_from_an_opener() {
        // Arrange
        let refusals = [GrpcOpenRefusal::UnknownUpstream, GrpcOpenRefusal::Failed];

        // Act
        let statuses = refusals.map(Status::from);

        // Assert
        assert_eq!(statuses, [Status::ParseFailure, Status::InternalFailure]);
    }
}
