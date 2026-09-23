//! A proxy that runs one plugin on each request, with one guest on one thread.
//!
//! Start it, send a request with `curl`, and read the log of the plugin beside
//! the log of the host.
//! The guest log goes to `tracing`, so both reach the same subscriber.
//!
//! `tiny_http` holds two file descriptors for each open connection. If you put
//! the example under load, raise the open file limit first with `ulimit -n`,
//! or the server stops with "Too many open files".
//!
//! The sink, the request type, and the answer of this file are written again in
//! `examples/http_workers/worker.rs`, so each example reads on its own.

use std::borrow::Cow;
use std::io::Cursor;
use std::ops::ControlFlow;
use std::sync::Arc;

use proxy_wasm_host::abi::v0_2_1::types::{Action, LogLevel, MapType, Status};
use proxy_wasm_host::abi::v0_2_1::{
    Access, Callback, ContextId, Guest, GuestError, GuestSpec, Host, Invocation, LocalResponse,
    LogContext, LogSink, PluginConfig, Started, StreamKind, StreamState, VmServices,
};
use proxy_wasm_host::{Engine, HeaderMap, Limits, Module, VecHeaderMap};
use tiny_http::{Header, Request, Response, Server};

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
    headers: VecHeaderMap,
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
fn request_state(request: &Request) -> HttpRequest {
    let mut headers: Vec<(Vec<u8>, Vec<u8>)> = vec![
        (b":method".to_vec(), request.method().as_str().into()),
        (b":path".to_vec(), request.url().into()),
        (b":authority".to_vec(), ADDRESS.into()),
    ];
    for header in request.headers() {
        headers.push((
            header.field.as_str().as_str().to_lowercase().into(),
            header.value.as_str().into(),
        ));
    }
    HttpRequest {
        headers: headers.into(),
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
        let action = scope.on_request_headers(stream, count, true)?;
        if !scope.on_done(stream)? {
            tracing::info!("the guest holds the context, and the example deletes it anyway");
        }
        scope.on_log(stream)?;
        scope.on_delete(stream)?;
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
    let pairs = pairs(&state.headers);
    let mut body = String::new();
    for (name, value) in &pairs {
        use std::fmt::Write;
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
    let services = VmServices::new(Arc::new(TracingSink));
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
        let request = server.recv()?;
        let span = tracing::info_span!("request", path = request.url());
        let _entered = span.enter();
        let state = request_state(&request);
        tracing::info!("{} {}", request.method(), request.url());
        let answer = match serve(&mut guest, root, state) {
            Ok(answer) => answer,
            Err(error) => {
                tracing::error!("the guest failed: {error}");
                let (fresh, fresh_root) = start(spec)?;
                guest = fresh;
                root = fresh_root;
                tracing::info!("a new guest serves the next request");
                Response::from_string("the guest failed").with_status_code(500)
            }
        };
        if let Err(error) = request.respond(answer) {
            tracing::warn!("the answer did not reach the client: {error}");
        }
    }
}

#[cfg(test)]
#[path = "http_server_tests.rs"]
mod tests;
