//! The gRPC callouts a guest asks the embedder to make, and the status one
//! ends with.

use std::borrow::Cow;
use std::time::Duration;

use super::http::owned_pairs;
use crate::abi::v0_2_1::HeaderPairs;
use crate::abi::v0_2_1::types::Status;

/// Why you refused a gRPC call or a gRPC stream.
///
/// Each case is a status that every guest SDK accepts from `proxy_grpc_call`
/// and from `proxy_grpc_stream`, so a refusal is an error value in the guest
/// and never a trap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum GrpcOpenRefusal {
    /// You do not know the upstream, which the guest reads as
    /// `PARSE_FAILURE`.
    /// The ABI specifies that status for a gRPC callout, where an HTTP call gets
    /// `BAD_ARGUMENT`.
    UnknownUpstream,
    /// You cannot reach the server, which the guest reads as
    /// `INTERNAL_FAILURE`.
    Failed,
}

impl From<GrpcOpenRefusal> for Status {
    fn from(refusal: GrpcOpenRefusal) -> Self {
        match refusal {
            GrpcOpenRefusal::UnknownUpstream => Self::ParseFailure,
            GrpcOpenRefusal::Failed => Self::InternalFailure,
        }
    }
}

/// The gRPC call a guest asks you to make.
///
/// A call sends one message and gets one answer.
/// The crate hands you a value that borrows guest memory for the duration of
/// the call.
/// Call [`GrpcCall::into_owned`] to keep it after you return.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct GrpcCall<'a> {
    /// The name of the upstream, as the guest wrote it.
    pub upstream: Cow<'a, [u8]>,
    /// The name of the gRPC service, for example `example.Authz`.
    pub service: Cow<'a, [u8]>,
    /// The name of the method on that service.
    pub method: Cow<'a, [u8]>,
    /// The metadata the guest sends with the call, which may be empty.
    pub initial_metadata: HeaderPairs<'a>,
    /// The request message, which may be empty.
    pub message: Cow<'a, [u8]>,
    /// How long the guest waits for the answer.
    /// The ABI gives a timeout of zero no meaning.
    pub timeout: Duration,
}

impl<'a> GrpcCall<'a> {
    /// A call to `method` of `service` on `upstream`, with no metadata, no
    /// message, and a timeout of zero, for a test of your own service.
    pub fn new(upstream: Cow<'a, [u8]>, service: Cow<'a, [u8]>, method: Cow<'a, [u8]>) -> Self {
        Self {
            upstream,
            service,
            method,
            initial_metadata: Vec::new(),
            message: Cow::Borrowed(&[]),
            timeout: Duration::ZERO,
        }
    }

    /// Sets the metadata.
    #[must_use]
    pub fn with_initial_metadata(mut self, metadata: HeaderPairs<'a>) -> Self {
        self.initial_metadata = metadata;
        self
    }

    /// Sets the message.
    #[must_use]
    pub fn with_message(mut self, message: Cow<'a, [u8]>) -> Self {
        self.message = message;
        self
    }

    /// Sets the timeout.
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// A copy that borrows nothing, for a call you make after you return.
    #[must_use]
    pub fn into_owned(self) -> GrpcCall<'static> {
        GrpcCall {
            upstream: Cow::Owned(self.upstream.into_owned()),
            service: Cow::Owned(self.service.into_owned()),
            method: Cow::Owned(self.method.into_owned()),
            initial_metadata: owned_pairs(self.initial_metadata),
            message: Cow::Owned(self.message.into_owned()),
            timeout: self.timeout,
        }
    }
}

/// The gRPC stream a guest asks you to open.
///
/// A stream stays open, and the guest sends messages on it through
/// [`Callouts::grpc_send`](crate::abi::v0_2_1::Callouts::grpc_send).
/// The crate hands you a value that borrows guest memory for the duration of
/// the call.
/// Call [`GrpcStream::into_owned`] to keep it after you return.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct GrpcStream<'a> {
    /// The name of the upstream, as the guest wrote it.
    pub upstream: Cow<'a, [u8]>,
    /// The name of the gRPC service, for example `example.Authz`.
    pub service: Cow<'a, [u8]>,
    /// The name of the method on that service.
    pub method: Cow<'a, [u8]>,
    /// The metadata the guest sends when the stream opens, which may be
    /// empty.
    pub initial_metadata: HeaderPairs<'a>,
}

impl<'a> GrpcStream<'a> {
    /// A stream to `method` of `service` on `upstream`, with no metadata, for
    /// a test of your own service.
    pub fn new(upstream: Cow<'a, [u8]>, service: Cow<'a, [u8]>, method: Cow<'a, [u8]>) -> Self {
        Self {
            upstream,
            service,
            method,
            initial_metadata: Vec::new(),
        }
    }

    /// Sets the metadata.
    #[must_use]
    pub fn with_initial_metadata(mut self, metadata: HeaderPairs<'a>) -> Self {
        self.initial_metadata = metadata;
        self
    }

    /// A copy that borrows nothing, for a stream you open after you return.
    #[must_use]
    pub fn into_owned(self) -> GrpcStream<'static> {
        GrpcStream {
            upstream: Cow::Owned(self.upstream.into_owned()),
            service: Cow::Owned(self.service.into_owned()),
            method: Cow::Owned(self.method.into_owned()),
            initial_metadata: owned_pairs(self.initial_metadata),
        }
    }
}

/// The status a gRPC call or stream ends with.
///
/// gRPC ends every call with a code and a text message, where zero is
/// success and fourteen means that the server was not available.
/// You give the status to
/// [`CallScope::on_grpc_close`](crate::abi::v0_2_1::CallScope::on_grpc_close),
/// and the guest reads it with `proxy_get_status` inside that callback.
///
/// The message is a `String`, because a guest of the Rust SDK stops with a
/// panic on bytes that are not UTF-8.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct GrpcStatus {
    /// The gRPC status code, which reaches the guest whatever its value.
    pub code: u32,
    /// The status message, which may be empty.
    pub message: String,
}

impl GrpcStatus {
    /// A status with `code` and `message`.
    pub fn new(code: u32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
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
    fn an_owned_grpc_call_keeps_every_field() {
        // Arrange
        let upstream = b"authz".to_vec();
        let call = GrpcCall::new(
            Cow::Borrowed(&upstream),
            Cow::Borrowed(b"example.Authz"),
            Cow::Borrowed(b"Check"),
        )
        .with_initial_metadata(pairs(&[(b"k", b"v")]))
        .with_message(Cow::Borrowed(b"body"))
        .with_timeout(Duration::from_millis(250));
        let expected = call.clone();

        // Act
        let owned = call.into_owned();

        // Assert
        assert_eq!(owned, expected);
        assert!(matches!(owned.upstream, Cow::Owned(_)));
        assert!(matches!(owned.message, Cow::Owned(_)));
        assert_eq!(owned.timeout, Duration::from_millis(250));
    }

    #[test]
    fn an_owned_grpc_stream_keeps_every_field() {
        // Arrange
        let upstream = b"authz".to_vec();
        let stream = GrpcStream::new(
            Cow::Borrowed(&upstream),
            Cow::Borrowed(b"example.Authz"),
            Cow::Borrowed(b"Watch"),
        )
        .with_initial_metadata(pairs(&[(b"k", b"v")]));
        let expected = stream.clone();

        // Act
        let owned = stream.into_owned();

        // Assert
        assert_eq!(owned, expected);
        assert!(matches!(owned.service, Cow::Owned(_)));
        assert!(matches!(owned.method, Cow::Owned(_)));
        assert_eq!(owned.initial_metadata.len(), 1);
    }

    #[test]
    fn a_status_takes_a_borrowed_and_an_owned_message() {
        // Arrange
        let owned = String::from("unavailable");

        // Act
        let statuses = [
            GrpcStatus::new(14, "unavailable"),
            GrpcStatus::new(14, owned),
        ];

        // Assert
        assert_eq!(statuses[0], statuses[1]);
        assert_eq!(statuses[0].code, 14);
        assert_eq!(statuses[0].message, "unavailable");
    }
}
