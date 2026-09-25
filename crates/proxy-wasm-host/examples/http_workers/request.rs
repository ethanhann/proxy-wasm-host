//! The request a guest reads, the answer of the example, and the log sink.
//!
//! These items are written again in `examples/http_server.rs`, so each example
//! reads on its own.

use std::borrow::Cow;
use std::fmt::Write as _;
use std::io::Cursor;
use std::ops::ControlFlow;

use proxy_wasm_host::HeaderMap;
use proxy_wasm_host::abi::v0_2_1::types::{Action, LogLevel, MapType, Status};
use proxy_wasm_host::abi::v0_2_1::{
    Access, Invocation, LocalResponse, LogContext, LogSink, StreamState,
};
use tiny_http::{Header, Request, Response};

use crate::headers::ProxyHeaders;

/// Emits one `tracing` event for a guest line.
///
/// The level of an event is part of its callsite, so each level needs its own
/// call.
/// The fields of an event are read by the arm that runs, and `tracing` reads
/// them only when a subscriber wants the line, so a line nobody records costs
/// no decoding.
macro_rules! emit {
    ($level:expr, $context:expr, $line:expr) => {
        match $level {
            LogLevel::Trace => tracing::trace!(
                target: "guest",
                plugin = %plugin_of(&$context),
                context = context_of(&$context),
                "{}", $line
            ),
            LogLevel::Debug => tracing::debug!(
                target: "guest",
                plugin = %plugin_of(&$context),
                context = context_of(&$context),
                "{}", $line
            ),
            LogLevel::Info => tracing::info!(
                target: "guest",
                plugin = %plugin_of(&$context),
                context = context_of(&$context),
                "{}", $line
            ),
            LogLevel::Warn => tracing::warn!(
                target: "guest",
                plugin = %plugin_of(&$context),
                context = context_of(&$context),
                "{}", $line
            ),
            LogLevel::Error | LogLevel::Critical => tracing::error!(
                target: "guest",
                plugin = %plugin_of(&$context),
                context = context_of(&$context),
                "{}", $line
            ),
        }
    };
}

/// The plugin a line came from, for a root that has not been configured yet.
const UNCONFIGURED: &str = "<unconfigured>";

/// A log sink that gives each guest line to `tracing`.
pub struct TracingSink;

/// The plugin a line came from, as text.
fn plugin_of<'a>(context: &'a LogContext<'_>) -> Cow<'a, str> {
    match &context.plugin_name {
        Some(name) => String::from_utf8_lossy(name),
        None => Cow::Borrowed(UNCONFIGURED),
    }
}

/// The context a line came from, or zero when no callback was running.
fn context_of(context: &LogContext<'_>) -> u32 {
    context.call.map_or(0, |call| call.context.get())
}

impl LogSink for TracingSink {
    fn log(&self, context: LogContext<'_>, level: LogLevel, message: &[u8]) {
        emit!(level, context, String::from_utf8_lossy(message));
    }
}

/// The answer a plugin sent by itself.
pub struct Local {
    status: u32,
    body: Vec<u8>,
}

/// The request a guest reads through its header map.
#[derive(Default)]
pub struct HttpRequest {
    /// The request headers, which the guest reads and changes.
    pub headers: ProxyHeaders,
    local: Option<Local>,
}

impl StreamState for HttpRequest {
    fn header_map(
        &mut self,
        _: Invocation,
        _: Access,
        map: MapType,
    ) -> Result<&mut dyn HeaderMap, Status> {
        match map {
            MapType::HttpRequestHeaders => Ok(&mut self.headers),
            _ => Err(Status::NotFound),
        }
    }

    fn send_local_response(
        &mut self,
        _: Invocation,
        response: LocalResponse<'_>,
    ) -> Result<(), Status> {
        self.local = Some(Local {
            status: response.status_code,
            body: response.body.into_owned(),
        });
        Ok(())
    }
}

/// The request headers, with the pseudo headers a guest expects first.
///
/// `authority` is the address the server listens on, which a guest reads as
/// `:authority`.
pub fn request_state(request: &Request, authority: &str) -> HttpRequest {
    let mut headers: Vec<(Vec<u8>, Vec<u8>)> = vec![
        (b":method".to_vec(), request.method().as_str().into()),
        (b":path".to_vec(), request.url().into()),
        (b":authority".to_vec(), authority.into()),
    ];
    for header in request.headers() {
        headers.push((
            header.field.as_str().as_str().into(),
            header.value.as_str().into(),
        ));
    }
    HttpRequest {
        headers: headers.into(),
        local: None,
    }
}

/// The pairs of a header map, in their order.
pub fn pairs(map: &dyn HeaderMap) -> Vec<(String, String)> {
    let mut out = Vec::new();
    {
        let mut visit = |key: &[u8], value: &[u8]| {
            out.push((
                String::from_utf8_lossy(key).into_owned(),
                String::from_utf8_lossy(value).into_owned(),
            ));
            ControlFlow::Continue(())
        };
        let _ = map.for_each_pair(&mut visit);
    }
    out
}

/// The answer of one request.
///
/// A plugin that sent its own answer decides the status and the body.
/// Otherwise the answer lists the headers the guest leaves behind, and it
/// carries each header that the guest can change.
/// The length and the type of the answer belong to the answer, so the headers
/// of the request do not reach it.
pub fn answer_of(state: &HttpRequest, action: Action) -> Response<Cursor<Vec<u8>>> {
    if let Some(local) = &state.local {
        tracing::info!("the plugin answered the request itself");
        let status = u16::try_from(local.status).unwrap_or(500);
        return Response::from_data(local.body.clone()).with_status_code(status);
    }
    if action == Action::Pause {
        tracing::info!("the guest paused the request, and a proxy would wait for it");
        return Response::from_string("the guest paused the request").with_status_code(504);
    }
    let pairs = pairs(&state.headers);
    let mut body = String::new();
    for (name, value) in &pairs {
        let _ = writeln!(body, "{name}: {value}");
    }
    let mut answer = Response::from_string(body);
    for (name, value) in pairs.iter().filter(|(name, _)| copied(name)) {
        if let Ok(header) = Header::from_bytes(name.as_bytes(), value.as_bytes()) {
            answer = answer.with_header(header);
        }
    }
    answer
}

/// Whether a header of the request belongs in the answer.
fn copied(name: &str) -> bool {
    !name.starts_with(':') && name != "content-length" && name != "content-type"
}

#[cfg(test)]
#[path = "request_tests.rs"]
mod tests;
