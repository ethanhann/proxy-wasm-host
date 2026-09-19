//! The service that receives the callouts of a guest, and the values it
//! exchanges with the crate.

mod response;

pub use response::HttpCallResponse;

use std::borrow::Cow;
use std::time::Duration;

use crate::abi::v0_2_1::types::Status;
use crate::abi::v0_2_1::unserved::unserved;
use crate::abi::v0_2_1::{CalloutId, HeaderPairs, Invocation};

/// The service that receives the callouts of a guest.
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
/// The guest waits inside the call, so a method must return at once.
/// Keep what you need with [`HttpCall::into_owned`], start the request
/// elsewhere, and return.
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
}

/// The service of a guest whose embedder serves no callout.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct NoCallouts;

impl Callouts for NoCallouts {}

/// Why you refused an HTTP call.
///
/// Each case is a status that every guest SDK accepts from
/// `proxy_http_call`, so a refusal is an error value in the guest and never
/// a trap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum HttpCallRefusal {
    /// You do not know the upstream, which the guest reads as `BAD_ARGUMENT`.
    UnknownUpstream,
    /// You cannot send the request, which the guest reads as
    /// `INTERNAL_FAILURE`.
    Failed,
}

impl From<HttpCallRefusal> for Status {
    fn from(refusal: HttpCallRefusal) -> Self {
        match refusal {
            HttpCallRefusal::UnknownUpstream => Self::BadArgument,
            HttpCallRefusal::Failed => Self::InternalFailure,
        }
    }
}

/// The HTTP request a guest asks you to send.
///
/// The crate hands you a value that borrows guest memory for the duration of
/// the call.
/// Call [`HttpCall::into_owned`] to keep it after you return.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct HttpCall<'a> {
    /// The name of the upstream, as the guest wrote it.
    pub upstream: Cow<'a, [u8]>,
    /// The request headers, which hold `:authority`, `:method`, and `:path`.
    pub headers: HeaderPairs<'a>,
    /// The request body, which may be empty.
    pub body: Cow<'a, [u8]>,
    /// The request trailers, which may be empty.
    pub trailers: HeaderPairs<'a>,
    /// How long the guest waits for the response.
    /// The ABI gives a timeout of zero no meaning.
    pub timeout: Duration,
}

impl<'a> HttpCall<'a> {
    /// A call to `upstream` with no header, no body, no trailer, and a
    /// timeout of zero, for a test of your own service.
    pub fn new(upstream: Cow<'a, [u8]>) -> Self {
        Self {
            upstream,
            headers: Vec::new(),
            body: Cow::Borrowed(&[]),
            trailers: Vec::new(),
            timeout: Duration::ZERO,
        }
    }

    /// Sets the headers.
    #[must_use]
    pub fn with_headers(mut self, headers: HeaderPairs<'a>) -> Self {
        self.headers = headers;
        self
    }

    /// Sets the body.
    #[must_use]
    pub fn with_body(mut self, body: Cow<'a, [u8]>) -> Self {
        self.body = body;
        self
    }

    /// Sets the trailers.
    #[must_use]
    pub fn with_trailers(mut self, trailers: HeaderPairs<'a>) -> Self {
        self.trailers = trailers;
        self
    }

    /// Sets the timeout.
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// A copy that borrows nothing, for a request you send after you return.
    #[must_use]
    pub fn into_owned(self) -> HttpCall<'static> {
        HttpCall {
            upstream: Cow::Owned(self.upstream.into_owned()),
            headers: owned_pairs(self.headers),
            body: Cow::Owned(self.body.into_owned()),
            trailers: owned_pairs(self.trailers),
            timeout: self.timeout,
        }
    }
}

fn owned_pairs(pairs: HeaderPairs<'_>) -> HeaderPairs<'static> {
    pairs
        .into_iter()
        .map(|(key, value)| (Cow::Owned(key.into_owned()), Cow::Owned(value.into_owned())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::v0_2_1::ContextId;

    fn pairs(list: &[(&'static [u8], &'static [u8])]) -> HeaderPairs<'static> {
        list.iter()
            .map(|(key, value)| (Cow::Borrowed(*key), Cow::Borrowed(*value)))
            .collect()
    }

    #[test]
    fn the_default_service_refuses_an_http_call_as_failed() {
        // Arrange
        let call = Invocation::new(ContextId::try_from(1).unwrap());
        let callout = CalloutId::try_from(1_u32).unwrap();
        let request = HttpCall::new(Cow::Borrowed(b"authz"));

        // Act
        let answer = NoCallouts.http_call(call, callout, request);

        // Assert
        assert_eq!(answer, Err(HttpCallRefusal::Failed));
    }

    #[test]
    fn each_refusal_is_a_status_every_sdk_accepts_from_an_http_call() {
        // Arrange
        let refusals = [HttpCallRefusal::UnknownUpstream, HttpCallRefusal::Failed];

        // Act
        let statuses = refusals.map(Status::from);

        // Assert
        assert_eq!(statuses, [Status::BadArgument, Status::InternalFailure]);
    }

    #[test]
    fn an_owned_call_keeps_every_field() {
        // Arrange
        let upstream = b"authz".to_vec();
        let call = HttpCall::new(Cow::Borrowed(&upstream))
            .with_headers(pairs(&[(b":path", b"/check")]))
            .with_body(Cow::Borrowed(b"body"))
            .with_trailers(pairs(&[(b"t", b"v")]))
            .with_timeout(Duration::from_millis(250));
        let expected = call.clone();

        // Act
        let owned = call.into_owned();

        // Assert
        assert_eq!(owned, expected);
        assert!(matches!(owned.upstream, Cow::Owned(_)));
    }
}
