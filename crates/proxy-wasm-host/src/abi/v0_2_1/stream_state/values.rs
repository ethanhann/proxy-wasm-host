//! The values a stream state exchanges with a guest.

use std::borrow::Cow;

/// Header or metadata pairs, in the order the guest serialized them.
pub type HeaderPairs<'a> = Vec<(Cow<'a, [u8]>, Cow<'a, [u8]>)>;

/// A call to a function of the embedder that the ABI does not name.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ForeignCall<'a> {
    /// The name the guest gave.
    pub name: Cow<'a, [u8]>,
    /// The argument bytes, which the guest and the embedder agree on.
    pub arguments: Cow<'a, [u8]>,
}

impl<'a> ForeignCall<'a> {
    /// A call to `name` with `arguments`.
    pub fn new(name: Cow<'a, [u8]>, arguments: Cow<'a, [u8]>) -> Self {
        Self { name, arguments }
    }

    /// The same call with no borrow left in it.
    #[must_use]
    pub fn into_owned(self) -> ForeignCall<'static> {
        ForeignCall {
            name: Cow::Owned(self.name.into_owned()),
            arguments: Cow::Owned(self.arguments.into_owned()),
        }
    }
}

/// The response a guest asks the proxy to send in place of the upstream one.
///
/// The crate hands you a value that borrows guest memory for the duration of
/// the call.
/// If you send the response inside the call, read the borrows.
/// If you queue the response and send it after the callback returns, call
/// [`LocalResponse::into_owned`] first, because the guest memory is gone by
/// then.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct LocalResponse<'a> {
    /// The HTTP status code.
    pub status_code: u32,
    /// The detail string for the status code, which may be empty.
    pub status_code_details: Cow<'a, [u8]>,
    /// The response body, which may be empty.
    pub body: Cow<'a, [u8]>,
    /// The response headers, in the order the guest serialized them.
    pub headers: HeaderPairs<'a>,
    /// The gRPC status, or `None` when the response carries none.
    ///
    /// The ABI types the field as unsigned, and every SDK sends the all ones
    /// value to mean that there is no gRPC status, which arrives here as
    /// `None`.
    pub grpc_status: Option<u32>,
}

impl<'a> LocalResponse<'a> {
    /// A response with a status code, no details, no body, no headers, and
    /// no gRPC status.
    pub fn new(status_code: u32) -> Self {
        Self {
            status_code,
            status_code_details: Cow::Borrowed(b""),
            body: Cow::Borrowed(b""),
            headers: Vec::new(),
            grpc_status: None,
        }
    }

    /// Sets the detail string for the status code.
    #[must_use]
    pub fn with_status_code_details(mut self, details: Cow<'a, [u8]>) -> Self {
        self.status_code_details = details;
        self
    }

    /// Sets the response body.
    #[must_use]
    pub fn with_body(mut self, body: Cow<'a, [u8]>) -> Self {
        self.body = body;
        self
    }

    /// Sets the response headers.
    #[must_use]
    pub fn with_headers(mut self, headers: HeaderPairs<'a>) -> Self {
        self.headers = headers;
        self
    }

    /// Sets the gRPC status.
    #[must_use]
    pub fn with_grpc_status(mut self, grpc_status: u32) -> Self {
        self.grpc_status = Some(grpc_status);
        self
    }

    /// The first value for `key`, or `None` when the guest did not set it.
    ///
    /// The comparison is exact, so a guest that writes a header name in
    /// another case is not found by this.
    pub fn header(&self, key: &[u8]) -> Option<&[u8]> {
        self.headers
            .iter()
            .find(|(name, _)| name.as_ref() == key)
            .map(|(_, value)| value.as_ref())
    }

    /// The same response with no borrow left in it.
    #[must_use]
    pub fn into_owned(self) -> LocalResponse<'static> {
        LocalResponse {
            status_code: self.status_code,
            status_code_details: Cow::Owned(self.status_code_details.into_owned()),
            body: Cow::Owned(self.body.into_owned()),
            headers: self
                .headers
                .into_iter()
                .map(|(key, value)| (Cow::Owned(key.into_owned()), Cow::Owned(value.into_owned())))
                .collect(),
            grpc_status: self.grpc_status,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(body: &[u8]) -> LocalResponse<'_> {
        LocalResponse::new(403)
            .with_status_code_details(Cow::Borrowed(b"denied"))
            .with_body(Cow::Borrowed(body))
            .with_headers(vec![(
                Cow::Borrowed(b"k".as_slice()),
                Cow::Borrowed(b"v".as_slice()),
            )])
            .with_grpc_status(7)
    }

    #[test]
    fn a_local_response_reads_back_its_values() {
        // Arrange
        let body = b"no".to_vec();

        // Act
        let response = response(&body);

        // Assert
        assert_eq!(response.status_code, 403);
        assert_eq!(response.status_code_details.as_ref(), b"denied");
        assert_eq!(response.body.as_ref(), b"no");
        assert_eq!(response.headers.len(), 1);
        assert_eq!(response.grpc_status, Some(7));
        assert_eq!(response.header(b"k"), Some(b"v".as_slice()));
        assert_eq!(response.header(b"K"), None);
    }

    #[test]
    fn a_local_response_outlives_the_borrow_it_was_built_from() {
        // Arrange
        let body = b"no".to_vec();
        let borrowed = response(&body);

        // Act
        let owned = borrowed.into_owned();

        // Assert
        drop(body);
        assert_eq!(owned.body.as_ref(), b"no");
        assert_eq!(owned.headers[0].1.as_ref(), b"v");
    }

    #[test]
    fn a_foreign_call_reads_back_its_values() {
        // Arrange
        let name = b"compress".to_vec();

        // Act
        let request = ForeignCall::new(Cow::Borrowed(&name), Cow::Borrowed(b"payload"));

        // Assert
        assert_eq!(request.name.as_ref(), b"compress");
        assert_eq!(request.arguments.as_ref(), b"payload");
    }

    #[test]
    fn a_foreign_call_outlives_the_borrow_it_was_built_from() {
        // Arrange
        let name = b"compress".to_vec();
        let borrowed = ForeignCall::new(Cow::Borrowed(&name), Cow::Borrowed(b"payload"));

        // Act
        let owned = borrowed.into_owned();

        // Assert
        drop(name);
        assert_eq!(owned.name.as_ref(), b"compress");
    }
}
