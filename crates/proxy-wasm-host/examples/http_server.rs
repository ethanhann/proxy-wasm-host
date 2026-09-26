//! A proxy that runs one plugin on each request, with one guest on one thread.
//!
//! Start it, send a request with `curl`, and read the log of the plugin beside
//! the log of the host.
//! The guest log goes to `tracing`, so both reach the same subscriber.
//!
//! `tiny_http` holds two file descriptors for each open connection.
//! If you put the example under load, raise the open file limit first with
//! `ulimit -n`, or the server stops with "Too many open files".
//!
//! The request headers stay in the `tiny_http` type, and
//! `http_server_headers.rs` lends them to the guest through the `HeaderMap`
//! trait, so the answer carries them with no second conversion.
//! A request body reaches the guest through `proxy_on_request_body` after the
//! headers, and the guest reads it as the request body buffer.
//!
//! The sink and the answer of this file are written again in
//! `examples/http_workers/request.rs`, so each example reads on its own.
//! That copy keeps header names in lower case, as a proxy does, and this one
//! keeps them as the guest wrote them.

use std::borrow::Cow;
use std::io::Cursor;
use std::ops::ControlFlow;
use std::sync::Arc;

use proxy_wasm_host::abi::v0_2_1::types::{Action, BufferType, LogLevel, MapType, Status};
use proxy_wasm_host::abi::v0_2_1::{
    Access, Callback, ContextId, Guest, GuestError, GuestSpec, Host, Invocation, LocalResponse,
    LogContext, LogSink, PluginConfig, Started, StreamKind, StreamState, VmServices,
};
use proxy_wasm_host::{Buffer, Engine, HeaderMap, Limits, Module};
use tiny_http::{Header, Request, Response, Server};

use crate::headers::RequestHeaders;

#[path = "http_server_headers.rs"]
mod headers;

/// The plugin this example runs when no path is given.
const DEFAULT_GUEST: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/add-request-header.wasm"
);

const ADDRESS: &str = "127.0.0.1:2045";

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
///
/// `Critical` arrives as `ERROR`, because `tracing` has no level above it.
struct TracingSink;

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
struct Local {
    status: u32,
    body: Vec<u8>,
}

/// The request a guest reads through its header map.
#[derive(Default)]
struct HttpRequest {
    headers: RequestHeaders,
    body: Vec<u8>,
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

    fn buffer(
        &mut self,
        _: Invocation,
        _: Access,
        buffer: BufferType,
    ) -> Result<&mut dyn Buffer, Status> {
        match buffer {
            BufferType::HttpRequestBody => Ok(&mut self.body),
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
/// The state of one request, with its body read to the end.
fn request_state(request: &mut Request) -> HttpRequest {
    let headers = RequestHeaders::new(
        request.method().as_str(),
        request.url(),
        ADDRESS,
        request.headers().to_vec(),
    );
    let mut body = Vec::new();
    if let Err(error) = request.as_reader().read_to_end(&mut body) {
        tracing::warn!("the body of the request did not arrive whole: {error}");
    }
    HttpRequest {
        headers,
        body,
        local: None,
    }
}

/// The pairs of a header map, in their order.
fn pairs(map: &dyn HeaderMap) -> Vec<(String, String)> {
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

/// Runs one request through the guest and builds the answer.
///
/// The example reads no body, so it tells the guest that the headers end the
/// stream, and it drops the body of the request.
fn serve(
    guest: &mut Guest,
    root: ContextId,
    state: HttpRequest,
) -> Result<Response<Cursor<Vec<u8>>>, GuestError> {
    let (answer, state) = guest.with(state, |scope| {
        let stream = scope.on_context_create(Some(root))?;
        scope.expect_stream_kind(stream, StreamKind::Http)?;
        let count = u32::try_from(scope.stream().headers.len()).unwrap_or(u32::MAX);
        let body_size = u32::try_from(scope.stream().body.len()).unwrap_or(u32::MAX);
        let mut action = scope.on_request_headers(stream, count, body_size == 0)?;
        if action == Action::Continue && body_size > 0 {
            action = scope.on_request_body(stream, body_size, true)?;
        }
        if scope.on_done(stream)? {
            scope.on_log(stream)?;
            scope.on_delete(stream)?;
        } else {
            tracing::info!("the guest holds the context, and this example never deletes it");
        }
        Ok::<_, GuestError>(action)
    });
    Ok(answer_of(&state, answer?))
}

/// The answer of one request.
///
/// A plugin that sent its own answer decides the status and the body.
/// Otherwise the answer lists the headers the guest leaves behind, and it
/// carries each header that the guest can change.
/// The length and the type of the answer belong to the answer, so the headers
/// of the request do not reach it.
fn answer_of(state: &HttpRequest, action: Action) -> Response<Cursor<Vec<u8>>> {
    if let Some(local) = &state.local {
        tracing::info!("the plugin answered the request itself");
        let status = u16::try_from(local.status).unwrap_or(500);
        return Response::from_data(local.body.clone()).with_status_code(status);
    }
    if action == Action::Pause {
        tracing::info!("the guest paused the request, and a proxy would wait for it");
        return Response::from_string("the guest paused the request").with_status_code(504);
    }
    let mut body = String::new();
    for (name, value) in pairs(&state.headers) {
        use std::fmt::Write;
        let _ = writeln!(body, "{name}: {value}");
    }
    let mut answer = Response::from_string(body);
    for header in state
        .headers
        .fields()
        .iter()
        .filter(|header| copied(header))
    {
        answer = answer.with_header(header.clone());
    }
    answer
}

fn copied(header: &Header) -> bool {
    !header.field.equiv("content-length") && !header.field.equiv("content-type")
}

/// Why the example has no guest.
#[derive(Debug)]
enum Failure {
    /// The build failed.
    Build(GuestError),
    /// The plugin refused its start, so it serves nothing.
    Refused(Callback),
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Build(error) => write!(f, "the guest did not build: {error}"),
            Self::Refused(callback) => write!(f, "the plugin refused its start in {callback}"),
        }
    }
}

impl std::error::Error for Failure {}

/// Builds a guest and starts its root.
fn start(spec: &GuestSpec) -> Result<(Guest, ContextId), Failure> {
    let mut guest = spec.build().map_err(Failure::Build)?;
    match guest
        .start(PluginConfig::new().with_name(*b"example"))
        .map_err(Failure::Build)?
    {
        Started::Serving(root) => Ok((guest, root)),
        Started::Refused { callback, .. } => Err(Failure::Refused(callback)),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt::init();
    let path = std::env::args().nth(1).unwrap_or(DEFAULT_GUEST.to_owned());
    let bytes = std::fs::read(&path)?;
    let engine = Engine::new()?;
    let module = Module::new(&engine, &bytes)?;
    let services = VmServices::new(Arc::new(TracingSink)).with_vm_id(*b"example");
    let spec = GuestSpec::new(&Host::new(&engine)?, &module, services, &Limits::default())?;
    let server = Server::http(ADDRESS)?;
    tracing::info!("listening on {ADDRESS} with the plugin {path}");
    run(&server, &spec)
}

/// Serves every request the server receives, one at a time.
///
/// `tiny_http` stops accepting connections after the first failed accept, so
/// this returns that error rather than letting the example exit quietly.
fn run(server: &Server, spec: &GuestSpec) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (mut guest, mut root) = start(spec)?;
    loop {
        let mut request = server.recv()?;
        let span = tracing::info_span!("request", path = request.url());
        let _entered = span.enter();
        tracing::info!("{} {}", request.method(), request.url());
        let state = request_state(&mut request);
        let answer = match serve(&mut guest, root, state) {
            Ok(answer) => answer,
            Err(error) => {
                tracing::error!("the guest failed: {error}");
                Response::from_string("the guest failed").with_status_code(500)
            }
        };
        if !guest.is_serving() {
            let (fresh, fresh_root) = start(spec)?;
            guest = fresh;
            root = fresh_root;
            tracing::info!("a new guest serves the next request");
        }
        if let Err(error) = request.respond(answer) {
            tracing::warn!("the answer did not reach the client: {error}");
        }
    }
}

#[cfg(test)]
#[path = "http_server_tests.rs"]
mod tests;
