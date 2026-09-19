//! The result of an HTTP call.

use std::borrow::Cow;

use super::owned_pairs;
use crate::abi::v0_2_1::HeaderPairs;

/// The result of an HTTP call, which you deliver to the guest.
///
/// A response is one of two cases.
/// [`HttpCallResponse::received`] is a response that arrived.
/// [`HttpCallResponse::failed`] is a call that got none, for a timeout, a
/// connection that closed, or a request you abandoned.
///
/// The ABI tells the guest that a call failed with a header count of zero.
/// A received response with no header would therefore read as a failure in
/// the guest, so the delivery refuses one.
/// Keep the `:status` header of the response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpCallResponse<'a> {
    received: bool,
    headers: HeaderPairs<'a>,
    body: Cow<'a, [u8]>,
    trailers: HeaderPairs<'a>,
}

impl<'a> HttpCallResponse<'a> {
    /// A response that arrived, with its headers.
    pub fn received(headers: HeaderPairs<'a>) -> Self {
        Self {
            received: true,
            headers,
            body: Cow::Borrowed(&[]),
            trailers: Vec::new(),
        }
    }

    /// A call that got no response.
    pub fn failed() -> Self {
        Self {
            received: false,
            headers: Vec::new(),
            body: Cow::Borrowed(&[]),
            trailers: Vec::new(),
        }
    }

    /// Sets the body of a received response.
    /// A failed response keeps none.
    #[must_use]
    pub fn with_body(mut self, body: Cow<'a, [u8]>) -> Self {
        if self.received {
            self.body = body;
        }
        self
    }

    /// Sets the trailers of a received response.
    /// A failed response keeps none.
    #[must_use]
    pub fn with_trailers(mut self, trailers: HeaderPairs<'a>) -> Self {
        if self.received {
            self.trailers = trailers;
        }
        self
    }

    /// Whether this is a call that got no response.
    pub fn is_failed(&self) -> bool {
        !self.received
    }

    /// A copy that borrows nothing.
    #[must_use]
    pub fn into_owned(self) -> HttpCallResponse<'static> {
        HttpCallResponse {
            received: self.received,
            headers: owned_pairs(self.headers),
            body: Cow::Owned(self.body.into_owned()),
            trailers: owned_pairs(self.trailers),
        }
    }

    pub(crate) fn into_parts(self) -> (HeaderPairs<'a>, Cow<'a, [u8]>, HeaderPairs<'a>) {
        (self.headers, self.body, self.trailers)
    }

    pub(crate) fn headers(&self) -> &HeaderPairs<'a> {
        &self.headers
    }

    pub(crate) fn body(&self) -> &[u8] {
        &self.body
    }

    pub(crate) fn trailers(&self) -> &HeaderPairs<'a> {
        &self.trailers
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(list: &[(&'static [u8], &'static [u8])]) -> HeaderPairs<'static> {
        list.iter()
            .map(|(key, value)| (Cow::Borrowed(*key), Cow::Borrowed(*value)))
            .collect()
    }

    #[test]
    fn a_received_response_keeps_what_it_was_given() {
        // Arrange
        let headers = pairs(&[(b":status", b"200")]);

        // Act
        let response = HttpCallResponse::received(headers)
            .with_body(Cow::Borrowed(b"ok"))
            .with_trailers(pairs(&[(b"t", b"v")]));

        // Assert
        assert!(!response.is_failed());
        assert_eq!(response.headers(), &pairs(&[(b":status", b"200")]));
        assert_eq!(response.body(), b"ok");
        assert_eq!(response.trailers(), &pairs(&[(b"t", b"v")]));
    }

    #[test]
    fn a_failed_response_keeps_no_body_and_no_trailer() {
        // Arrange
        let failed = HttpCallResponse::failed();

        // Act
        let response = failed
            .with_body(Cow::Borrowed(b"ignored"))
            .with_trailers(pairs(&[(b"t", b"v")]));

        // Assert
        assert!(response.is_failed());
        assert!(response.headers().is_empty());
        assert!(response.body().is_empty());
        assert!(response.trailers().is_empty());
    }

    #[test]
    fn an_owned_response_is_equal_and_borrows_nothing() {
        // Arrange
        let body = b"ok".to_vec();
        let response = HttpCallResponse::received(pairs(&[(b":status", b"200")]))
            .with_body(Cow::Borrowed(&body));
        let expected = response.clone();

        // Act
        let owned: HttpCallResponse<'static> = response.into_owned();

        // Assert
        assert_eq!(owned, expected);
    }
}
