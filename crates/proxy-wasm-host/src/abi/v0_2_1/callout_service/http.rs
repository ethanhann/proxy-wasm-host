//! The HTTP call a guest asks the embedder to send.

use std::borrow::Cow;
use std::time::Duration;

use crate::abi::v0_2_1::HeaderPairs;
use crate::abi::v0_2_1::types::Status;

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

pub(super) fn owned_pairs(pairs: HeaderPairs<'_>) -> HeaderPairs<'static> {
    pairs
        .into_iter()
        .map(|(key, value)| (Cow::Owned(key.into_owned()), Cow::Owned(value.into_owned())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::borrow::Cow;

    fn pairs(list: &[(&'static [u8], &'static [u8])]) -> HeaderPairs<'static> {
        list.iter()
            .map(|(key, value)| (Cow::Borrowed(*key), Cow::Borrowed(*value)))
            .collect()
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
        assert!(matches!(owned.body, Cow::Owned(_)));
        assert_eq!(owned.timeout, Duration::from_millis(250));
    }
}
