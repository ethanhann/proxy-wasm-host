//! The values a stream host exchanges with a guest.

use std::borrow::Cow;

/// The headers of a local response, in the order the guest serialized them.
pub type ResponseHeaders<'a> = Vec<(Cow<'a, [u8]>, Cow<'a, [u8]>)>;

/// The status of the callout the guest is handling.
///
/// The guest asks for it in `proxy_on_http_call_response` and in
/// `proxy_on_grpc_close`, and the value describes the call that callback
/// delivers.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct CalloutStatus<'a> {
    /// The status code of the HTTP call or the gRPC call.
    pub code: u32,
    /// The status message, which may be empty.
    pub message: Cow<'a, [u8]>,
}

impl<'a> CalloutStatus<'a> {
    /// A status with `code` and `message`.
    pub fn new(code: u32, message: Cow<'a, [u8]>) -> Self {
        Self { code, message }
    }

    /// The same status with no borrow left in it.
    #[must_use]
    pub fn into_owned(self) -> CalloutStatus<'static> {
        CalloutStatus {
            code: self.code,
            message: Cow::Owned(self.message.into_owned()),
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
    pub headers: ResponseHeaders<'a>,
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
    pub fn with_headers(mut self, headers: ResponseHeaders<'a>) -> Self {
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
    fn a_callout_status_reads_back_its_values() {
        // Arrange
        let message = b"unavailable".to_vec();

        // Act
        let status = CalloutStatus::new(503, Cow::Borrowed(&message));

        // Assert
        assert_eq!(status.code, 503);
        assert_eq!(status.message.as_ref(), b"unavailable");
    }

    #[test]
    fn a_callout_status_outlives_the_borrow_it_was_built_from() {
        // Arrange
        let message = b"unavailable".to_vec();
        let borrowed = CalloutStatus::new(503, Cow::Borrowed(&message));

        // Act
        let owned = borrowed.into_owned();

        // Assert
        drop(message);
        assert_eq!(
            owned,
            CalloutStatus::new(503, Cow::Owned(b"unavailable".to_vec()))
        );
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
}
