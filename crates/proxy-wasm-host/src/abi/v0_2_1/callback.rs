//! The callbacks a guest exports.

use std::fmt;

/// A `proxy_on_*` callback that this crate can call.
///
/// Every callback is optional for a guest.
/// [`Guest::exports_callback`](super::Guest::exports_callback) tells you
/// whether a guest exports one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Callback {
    /// `proxy_on_context_create`.
    ContextCreate,
    /// `proxy_on_vm_start`.
    VmStart,
    /// `proxy_on_configure`.
    Configure,
    /// `proxy_on_new_connection`.
    NewConnection,
    /// `proxy_on_downstream_data`.
    DownstreamData,
    /// `proxy_on_downstream_connection_close`.
    DownstreamConnectionClose,
    /// `proxy_on_upstream_data`.
    UpstreamData,
    /// `proxy_on_upstream_connection_close`.
    UpstreamConnectionClose,
    /// `proxy_on_request_headers`.
    RequestHeaders,
    /// `proxy_on_request_body`.
    RequestBody,
    /// `proxy_on_request_trailers`.
    RequestTrailers,
    /// `proxy_on_response_headers`.
    ResponseHeaders,
    /// `proxy_on_response_body`.
    ResponseBody,
    /// `proxy_on_response_trailers`.
    ResponseTrailers,
    /// `proxy_on_http_call_response`.
    HttpCallResponse,
    /// `proxy_on_grpc_receive_initial_metadata`.
    GrpcReceiveInitialMetadata,
    /// `proxy_on_grpc_receive`.
    GrpcReceive,
    /// `proxy_on_grpc_receive_trailing_metadata`.
    GrpcReceiveTrailingMetadata,
    /// `proxy_on_grpc_close`.
    GrpcClose,
    /// `proxy_on_done`.
    Done,
    /// `proxy_on_log`.
    Log,
    /// `proxy_on_delete`.
    Delete,
    /// `proxy_on_tick`.
    Tick,
    /// `proxy_on_queue_ready`.
    QueueReady,
    /// `proxy_on_foreign_function`.
    ForeignFunction,
}

impl Callback {
    /// The export name the ABI gives the callback.
    pub fn export_name(self) -> &'static str {
        match self {
            Self::ContextCreate => "proxy_on_context_create",
            Self::VmStart => "proxy_on_vm_start",
            Self::Configure => "proxy_on_configure",
            Self::NewConnection => "proxy_on_new_connection",
            Self::DownstreamData => "proxy_on_downstream_data",
            Self::DownstreamConnectionClose => "proxy_on_downstream_connection_close",
            Self::UpstreamData => "proxy_on_upstream_data",
            Self::UpstreamConnectionClose => "proxy_on_upstream_connection_close",
            Self::RequestHeaders => "proxy_on_request_headers",
            Self::RequestBody => "proxy_on_request_body",
            Self::RequestTrailers => "proxy_on_request_trailers",
            Self::ResponseHeaders => "proxy_on_response_headers",
            Self::ResponseBody => "proxy_on_response_body",
            Self::ResponseTrailers => "proxy_on_response_trailers",
            Self::HttpCallResponse => "proxy_on_http_call_response",
            Self::GrpcReceiveInitialMetadata => "proxy_on_grpc_receive_initial_metadata",
            Self::GrpcReceive => "proxy_on_grpc_receive",
            Self::GrpcReceiveTrailingMetadata => "proxy_on_grpc_receive_trailing_metadata",
            Self::GrpcClose => "proxy_on_grpc_close",
            Self::Done => "proxy_on_done",
            Self::Log => "proxy_on_log",
            Self::Delete => "proxy_on_delete",
            Self::Tick => "proxy_on_tick",
            Self::QueueReady => "proxy_on_queue_ready",
            Self::ForeignFunction => "proxy_on_foreign_function",
        }
    }

    /// Every callback this crate drives.
    ///
    /// The callbacks of a stream come in lifecycle order, with the TCP
    /// family before the HTTP family.
    /// The callbacks of a callout follow them, because a callout belongs to
    /// the context that made it and not to a place in that order.
    /// The two that a root gets at any time come next.
    /// The callback of a foreign function call comes last, because the host
    /// starts it and it belongs to no lifecycle.
    pub const ALL: &[Self] = &[
        Self::ContextCreate,
        Self::VmStart,
        Self::Configure,
        Self::NewConnection,
        Self::DownstreamData,
        Self::DownstreamConnectionClose,
        Self::UpstreamData,
        Self::UpstreamConnectionClose,
        Self::RequestHeaders,
        Self::RequestBody,
        Self::RequestTrailers,
        Self::ResponseHeaders,
        Self::ResponseBody,
        Self::ResponseTrailers,
        Self::HttpCallResponse,
        Self::GrpcReceiveInitialMetadata,
        Self::GrpcReceive,
        Self::GrpcReceiveTrailingMetadata,
        Self::GrpcClose,
        Self::Done,
        Self::Log,
        Self::Delete,
        Self::Tick,
        Self::QueueReady,
        Self::ForeignFunction,
    ];
}

impl fmt::Display for Callback {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.export_name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn position(callback: Callback) -> usize {
        match callback {
            Callback::ContextCreate => 0,
            Callback::VmStart => 1,
            Callback::Configure => 2,
            Callback::NewConnection => 3,
            Callback::DownstreamData => 4,
            Callback::DownstreamConnectionClose => 5,
            Callback::UpstreamData => 6,
            Callback::UpstreamConnectionClose => 7,
            Callback::RequestHeaders => 8,
            Callback::RequestBody => 9,
            Callback::RequestTrailers => 10,
            Callback::ResponseHeaders => 11,
            Callback::ResponseBody => 12,
            Callback::ResponseTrailers => 13,
            Callback::HttpCallResponse => 14,
            Callback::GrpcReceiveInitialMetadata => 15,
            Callback::GrpcReceive => 16,
            Callback::GrpcReceiveTrailingMetadata => 17,
            Callback::GrpcClose => 18,
            Callback::Done => 19,
            Callback::Log => 20,
            Callback::Delete => 21,
            Callback::Tick => 22,
            Callback::QueueReady => 23,
            Callback::ForeignFunction => 24,
        }
    }

    #[test]
    fn every_callback_is_listed_once_in_the_documented_order() {
        // Arrange
        let callbacks = Callback::ALL;

        // Act
        let positions: Vec<usize> = callbacks
            .iter()
            .map(|callback| position(*callback))
            .collect();

        // Assert
        assert_eq!(positions, (0..25).collect::<Vec<usize>>());
        assert_eq!(Callback::Delete.to_string(), "proxy_on_delete");
        assert!(
            callbacks
                .iter()
                .all(|callback| callback.export_name().starts_with("proxy_on_"))
        );
    }
}
