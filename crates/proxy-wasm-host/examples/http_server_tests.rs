//! The tests of the `http_server` example.

use std::io::Write;
use std::sync::{Mutex, PoisonError};

use tiny_http::TestRequest;

use proxy_wasm_host::abi::v0_2_1::GuestId;

use super::*;

const GUEST: &[u8] = include_bytes!("../tests/fixtures/add-request-header.wasm");

/// A plugin that answers a request with the header `x-deny` by itself.
const DENIER: &[u8] = include_bytes!("../tests/fixtures/http-example.wasm");

/// A guest whose request body pauses the stream, and whose headers do not.
const BODY_PAUSING: &str = r#"(module
    (memory (export "memory") 1)
    (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 1024)
    (func (export "proxy_abi_version_0_2_1"))
    (func (export "proxy_on_request_body") (param i32 i32 i32) (result i32) i32.const 1))"#;

/// A guest that keeps every stream context after `proxy_on_done`.
const HOLDING: &str = r#"(module
    (memory (export "memory") 1)
    (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 1024)
    (func (export "proxy_abi_version_0_2_1"))
    (func (export "proxy_on_done") (param i32) (result i32) i32.const 0))"#;

/// A guest whose request headers pause the stream.
const PAUSING: &str = r#"(module
    (memory (export "memory") 1)
    (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 1024)
    (func (export "proxy_abi_version_0_2_1"))
    (func (export "proxy_on_request_headers") (param i32 i32 i32) (result i32) i32.const 1))"#;

fn spec_of(bytes: &[u8]) -> GuestSpec {
    let engine = Engine::new().unwrap();
    let module = Module::new(&engine, bytes).unwrap();
    let services = VmServices::new(Arc::new(TracingSink));
    GuestSpec::new(
        &Host::new(&engine).unwrap(),
        &module,
        services,
        &Limits::default(),
    )
    .unwrap()
}

fn guest_of(bytes: &[u8]) -> (Guest, ContextId) {
    start(&spec_of(bytes)).unwrap()
}

/// A writer that keeps every line a subscriber writes.
#[derive(Clone, Default)]
struct Lines(Arc<Mutex<Vec<u8>>>);

impl Write for Lines {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Lines {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

#[test]
fn a_request_gets_the_header_the_plugin_adds() {
    // Arrange
    let (mut guest, root) = guest_of(GUEST);
    let mut request = Request::from(TestRequest::new().with_path("/"));
    let state = request_state(&mut request);

    // Act
    let answer = serve(&mut guest, root, state);

    // Assert
    let answer = answer.unwrap();
    assert_eq!(answer.status_code().0, 200);
    let names: Vec<String> = answer
        .headers()
        .iter()
        .map(|header| header.field.as_str().as_str().to_lowercase())
        .collect();
    assert!(names.contains(&"wasm-context".to_owned()), "{names:?}");
}

#[test]
fn a_request_body_reaches_the_guest_after_the_headers() {
    // Arrange
    let (mut guest, root) = guest_of(&wat::parse_str(BODY_PAUSING).unwrap());
    let mut with_body = Request::from(TestRequest::new().with_path("/").with_body("payload"));
    let mut without = Request::from(TestRequest::new().with_path("/"));
    let states = (request_state(&mut with_body), request_state(&mut without));

    // Act
    let answers = (
        serve(&mut guest, root, states.0),
        serve(&mut guest, root, states.1),
    );

    // Assert
    assert_eq!(
        answers.0.unwrap().status_code().0,
        504,
        "the body callback paused"
    );
    assert_eq!(
        answers.1.unwrap().status_code().0,
        200,
        "no body, no body callback"
    );
}

#[test]
fn a_context_the_guest_holds_is_answered_and_kept() {
    // Arrange
    let (mut guest, root) = guest_of(&wat::parse_str(HOLDING).unwrap());
    let mut request = Request::from(TestRequest::new().with_path("/"));
    let state = request_state(&mut request);

    // Act
    let answer = serve(&mut guest, root, state);

    // Assert
    assert_eq!(answer.unwrap().status_code().0, 200);
    let stream = ContextId::try_from(2).unwrap();
    assert_eq!(
        guest.context_state(stream),
        Some(proxy_wasm_host::abi::v0_2_1::ContextState::Pending),
        "the context waits for proxy_done"
    );
}

#[test]
fn a_paused_request_answers_504() {
    // Arrange
    let (mut guest, root) = guest_of(&wat::parse_str(PAUSING).unwrap());
    let mut request = Request::from(TestRequest::new().with_path("/"));
    let state = request_state(&mut request);

    // Act
    let answer = serve(&mut guest, root, state);

    // Assert
    assert_eq!(answer.unwrap().status_code().0, 504);
}

#[test]
fn the_sink_maps_each_level_to_its_tracing_level() {
    // Arrange
    let lines = Lines::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(lines.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .finish();
    let context = LogContext::new(b"vm", GuestId::next());
    let levels = [
        LogLevel::Trace,
        LogLevel::Debug,
        LogLevel::Info,
        LogLevel::Warn,
        LogLevel::Error,
        LogLevel::Critical,
    ];

    // Act
    tracing::subscriber::with_default(subscriber, || {
        for level in levels {
            TracingSink.log(context.clone(), level, b"line");
        }
    });

    // Assert
    let text = String::from_utf8(
        lines
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone(),
    )
    .unwrap();
    let seen: Vec<&str> = text
        .lines()
        .filter_map(|line| line.split_whitespace().nth(1))
        .collect();
    assert_eq!(seen, ["TRACE", "DEBUG", "INFO", "WARN", "ERROR", "ERROR"]);
    assert_eq!(text.matches("guest: line").count(), 6, "{text}");
}

#[test]
fn a_denied_request_gets_the_answer_of_the_plugin() {
    // Arrange
    let (mut guest, root) = guest_of(DENIER);
    let mut source = Request::from(
        TestRequest::new()
            .with_path("/")
            .with_header("x-deny: 1".parse::<Header>().unwrap()),
    );
    let state = request_state(&mut source);

    // Act
    let answer = serve(&mut guest, root, state);

    // Assert
    assert_eq!(answer.unwrap().status_code().0, 403);
}

#[test]
fn the_answer_carries_no_length_or_type_of_the_request() {
    // Arrange
    let (mut guest, root) = guest_of(GUEST);
    let mut source = Request::from(
        TestRequest::new()
            .with_path("/")
            .with_header("content-type: application/json".parse::<Header>().unwrap()),
    );
    let state = request_state(&mut source);

    // Act
    let answer = serve(&mut guest, root, state);

    // Assert
    let answer = answer.unwrap();
    let headers: Vec<(String, String)> = answer
        .headers()
        .iter()
        .map(|header| {
            (
                header.field.as_str().as_str().to_lowercase(),
                header.value.as_str().to_owned(),
            )
        })
        .collect();
    let content_type = headers.iter().find(|(name, _)| name == "content-type");
    assert_eq!(
        content_type.map(|(_, value)| value.as_str()),
        Some("text/plain; charset=UTF-8"),
        "the answer keeps its own type, and not the type of the request"
    );
    assert!(
        !headers.iter().any(|(name, _)| name == "content-length"),
        "{headers:?}"
    );
}

#[test]
fn the_request_state_holds_the_three_pseudo_headers() {
    // Arrange
    let mut source = Request::from(TestRequest::new().with_path("/example"));

    // Act
    let state = request_state(&mut source);

    // Assert
    let names: Vec<String> = pairs(&state.headers)
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    assert_eq!(names[..3], [":method", ":path", ":authority"]);
    assert_eq!(pairs(&state.headers)[1].1, "/example");
}

#[test]
fn a_server_that_stops_accepting_ends_the_run_with_an_error() {
    // Arrange
    let spec = spec_of(GUEST);
    let server = Server::http("127.0.0.1:0").unwrap();
    server.unblock();

    // Act
    let result = run(&server, &spec);

    // Assert
    let error = result.expect_err("a server that stops must not end the example quietly");
    assert!(!error.to_string().is_empty());
}
