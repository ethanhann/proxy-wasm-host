//! The example plugins of the Rust SDK, run as guests.
//!
//! Nobody on this project wrote these plugins. They are copies of the
//! examples of the SDK at tag v0.2.5, built from `crates/test-guests/sdk-*`,
//! and `NOTICE` records where they come from.
//!
//! A failure here means that the crate and the SDK disagree about the ABI,
//! and not that a guest of this project needs a fix.
//!
//! The helpers below are test code, and the allowance clippy makes for a test
//! does not reach a function of an integration test that carries no test
//! attribute.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::borrow::Cow;
use std::sync::Arc;
use std::time::Duration;

use proxy_wasm_host::abi::v0_2_1::types::{Action, LogLevel, Status};
use proxy_wasm_host::abi::v0_2_1::{
    CalloutId, ContextId, GrpcStatus, Guest, GuestError, GuestSpec, Host, HttpCallRefusal,
    HttpCallResponse, PluginConfig, Started, StreamKind, VmServices,
};
use proxy_wasm_host::{Engine, Limits, Module};

mod common;

use common::answers::{CalloutCall, CalloutRefusal, StreamCall};
use common::harness::{FixedClock, pair};
use common::recorder::{Event, Recorder, StreamDouble, pairs};

const HELLO_WORLD: &[u8] = include_bytes!("fixtures/sdk-hello-world.wasm");
const HTTP_HEADERS: &[u8] = include_bytes!("fixtures/sdk-http-headers.wasm");
const HTTP_CONFIG: &[u8] = include_bytes!("fixtures/sdk-http-config.wasm");
const HTTP_BODY: &[u8] = include_bytes!("fixtures/sdk-http-body.wasm");
const HTTP_AUTH_RANDOM: &[u8] = include_bytes!("fixtures/sdk-http-auth-random.wasm");
const GRPC_AUTH_RANDOM: &[u8] = include_bytes!("fixtures/sdk-grpc-auth-random.wasm");
const ENVOY_FILTER_METADATA: &[u8] = include_bytes!("fixtures/sdk-envoy-filter-metadata.wasm");

/// One SDK example, with the recorder that serves it.
struct Example {
    spec: GuestSpec,
    recorder: Recorder,
}

impl Example {
    fn new(module: &[u8]) -> Self {
        let engine = Engine::new().unwrap();
        let module = Module::new(&engine, module).unwrap();
        let host = Host::new(&engine).unwrap();
        let recorder = Recorder::default();
        let services = VmServices::new(Arc::new(recorder.clone()))
            .with_clock(Arc::new(FixedClock))
            .with_log_level(LogLevel::Trace)
            .with_vm_id(*b"sdk")
            .with_shared(Arc::new(recorder.shared()))
            .with_callouts(Arc::new(recorder.clone()));
        let spec = GuestSpec::new(&host, &module, services, &Limits::default()).unwrap();
        Self { spec, recorder }
    }

    /// A guest whose root is started with `configuration`.
    fn started(&self, configuration: &str) -> (Guest, ContextId) {
        let mut guest = self.spec.build().unwrap();
        let plugin = PluginConfig::new()
            .with_name(*b"sdk")
            .with_root_id(*b"sdk")
            .with_configuration(configuration.as_bytes().to_vec());
        match guest.start(plugin).unwrap() {
            Started::Serving(root) => (guest, root),
            refused @ Started::Refused { .. } => panic!("the root was refused: {refused:?}"),
        }
    }

    /// The text of every log line the guest wrote, at any level.
    fn lines(&self) -> Vec<String> {
        self.recorder
            .events()
            .into_iter()
            .filter_map(|event| match event {
                Event::Log(_, line) => Some(line),
                _ => None,
            })
            .collect()
    }
}

/// A stream context under `root`, with the state the test lends it.
fn stream(guest: &mut Guest, root: ContextId, state: StreamDouble) -> (ContextId, StreamDouble) {
    let (created, state) = guest.with(state, |scope| {
        let stream = scope.on_context_create(Some(root))?;
        scope.expect_stream_kind(stream, StreamKind::Http)?;
        Ok::<_, GuestError>(stream)
    });
    (created.unwrap(), state)
}

fn callout(id: u32) -> CalloutId {
    CalloutId::try_from(id).unwrap()
}

#[test]
fn the_hello_world_guest_logs_and_sets_a_tick_period() {
    // Arrange
    let example = Example::new(HELLO_WORLD);

    // Act
    let (mut guest, root) = example.started("");

    // Assert
    assert!(
        example.lines().contains(&"Hello, World!".to_owned()),
        "the guest logs one line from its VM start, and wrote {:?}",
        example.lines()
    );
    let changes = guest.take_changes();
    assert_eq!(
        changes.tick_periods.get(&root),
        Some(&Some(Duration::from_secs(5))),
        "the guest asks for a tick every five seconds"
    );
}

#[test]
fn the_hello_world_guest_reads_the_clock_of_the_services_on_a_tick() {
    // Arrange
    let example = Example::new(HELLO_WORLD);
    let (mut guest, root) = example.started("");
    example.recorder.clear();

    // Act
    let result = guest.enter_root().on_tick(root);

    // Assert
    assert!(result.is_ok());
    let lines = example.lines();
    assert_eq!(lines.len(), 1, "one tick writes one line, not {lines:?}");
    assert!(
        lines[0].starts_with("It's 1970-01-01"),
        "the guest reads the clock the services gave, and wrote {:?}",
        lines[0]
    );
    assert!(
        lines[0].contains("your lucky number is"),
        "on this target the guest takes the branch that reads WASI randomness"
    );
}

#[test]
fn the_http_headers_guest_logs_every_request_header() {
    // Arrange
    let example = Example::new(HTTP_HEADERS);
    let (mut guest, root) = example.started("");
    let state = StreamDouble::new(&example.recorder)
        .with_request_headers(&[(":path", "/"), ("accept", "text/plain")]);
    let (context, state) = stream(&mut guest, root, state);
    example.recorder.clear();

    // Act
    let (action, _state) = guest.with(state, |scope| scope.on_request_headers(context, 2, true));

    // Assert
    assert_eq!(action.unwrap(), Action::Continue);
    let lines = example.lines();
    assert!(
        lines.iter().any(|line| line.ends_with("-> :path: /")),
        "each header reaches the guest, which wrote {lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.ends_with("-> accept: text/plain"))
    );
}

#[test]
fn the_http_headers_guest_answers_the_hello_path() {
    // Arrange
    let example = Example::new(HTTP_HEADERS);
    let (mut guest, root) = example.started("");
    let state = StreamDouble::new(&example.recorder).with_request_headers(&[(":path", "/hello")]);
    let (context, state) = stream(&mut guest, root, state);
    example.recorder.clear();

    // Act
    let (action, _state) = guest.with(state, |scope| scope.on_request_headers(context, 1, true));

    // Assert
    assert_eq!(action.unwrap(), Action::Pause);
    let response = example
        .recorder
        .calls()
        .into_iter()
        .find_map(|event| match event {
            Event::LocalResponse {
                status,
                headers,
                body,
                ..
            } => Some((status, headers, body)),
            _ => None,
        })
        .expect("the guest answers the request itself");
    assert_eq!(response.0, 200);
    assert_eq!(
        response.1,
        vec![
            ("Hello".to_owned(), "World".to_owned()),
            ("Powered-By".to_owned(), "proxy-wasm".to_owned()),
        ]
    );
    assert_eq!(response.2, "Hello, World!\n");
}

#[test]
fn the_http_config_guest_adds_the_header_of_its_configuration() {
    // Arrange
    let example = Example::new(HTTP_CONFIG);
    let (mut guest, root) = example.started("from-the-configuration");
    let state = StreamDouble::new(&example.recorder);
    let (context, state) = stream(&mut guest, root, state);

    // Act
    let (action, state) = guest.with(state, |scope| scope.on_response_headers(context, 0, true));

    // Assert
    assert_eq!(action.unwrap(), Action::Continue);
    assert!(
        pairs(&state.response_headers).contains(&(
            "custom-header".to_owned(),
            "from-the-configuration".to_owned()
        )),
        "the plugin configuration reaches the guest and comes back as a header"
    );
}

#[test]
fn the_http_body_guest_pauses_until_the_end_of_the_stream() {
    // Arrange
    let example = Example::new(HTTP_BODY);
    let (mut guest, root) = example.started("");
    let mut state = StreamDouble::new(&example.recorder);
    state.response_body = b"a secret".to_vec();
    let (context, state) = stream(&mut guest, root, state);

    // Act
    let (action, _state) = guest.with(state, |scope| scope.on_response_body(context, 8, false));

    // Assert
    assert_eq!(
        action.unwrap(),
        Action::Pause,
        "the guest waits for the whole body"
    );
}

#[test]
fn the_http_body_guest_redacts_a_body_that_holds_the_word() {
    // Arrange
    let example = Example::new(HTTP_BODY);
    let (mut guest, root) = example.started("");
    let mut state =
        StreamDouble::new(&example.recorder).with_response_headers(&[("content-length", "8")]);
    state.response_body = b"a secret".to_vec();
    let (context, state) = stream(&mut guest, root, state);
    let (headers, state) = guest.with(state, |scope| scope.on_response_headers(context, 1, false));

    // Act
    let (action, state) = guest.with(state, |scope| scope.on_response_body(context, 8, true));

    // Assert
    assert_eq!(headers.unwrap(), Action::Continue);
    assert_eq!(action.unwrap(), Action::Continue);
    assert_eq!(
        String::from_utf8(state.response_body.clone()).unwrap(),
        "Original message body (8 bytes) redacted.\n"
    );
    assert!(
        pairs(&state.response_headers).is_empty(),
        "the guest removes the length it would invalidate"
    );
}

#[test]
fn the_http_body_guest_leaves_a_body_it_cannot_read() {
    // Arrange
    let example = Example::new(HTTP_BODY);
    let (mut guest, root) = example.started("");
    let mut state = StreamDouble::new(&example.recorder);
    state.response_body = b"a secret".to_vec();
    let (context, mut state) = stream(&mut guest, root, state);
    state.refusals.insert(StreamCall::Buffer, Status::NotFound);

    // Act
    let (action, state) = guest.with(state, |scope| scope.on_response_body(context, 8, true));

    // Assert
    assert_eq!(action.unwrap(), Action::Continue);
    assert_eq!(
        state.response_body, b"a secret",
        "a body the guest cannot read is left as it is"
    );
}

#[test]
fn the_auth_guest_opens_a_callout_and_resumes_on_an_even_byte() {
    // Arrange
    let example = Example::new(HTTP_AUTH_RANDOM);
    let (mut guest, root) = example.started("");
    let state = StreamDouble::new(&example.recorder);
    let (context, state) = stream(&mut guest, root, state);
    let (opened, state) = guest.with(state, |scope| scope.on_request_headers(context, 0, true));

    // Act
    let (delivered, _state) = guest.with(state, |scope| {
        scope.on_http_call_response(
            context,
            callout(1),
            HttpCallResponse::received(vec![pair(":status", "200")])
                .with_body(Cow::Borrowed(b"\x02")),
        )
    });

    // Assert
    assert_eq!(opened.unwrap(), Action::Pause, "the guest waits");
    assert!(delivered.is_ok());
    let calls = example.recorder.calls();
    assert!(
        calls
            .iter()
            .any(|event| matches!(event, Event::HttpCall { .. })),
        "the guest opens a callout"
    );
    assert!(
        calls
            .iter()
            .any(|event| matches!(event, Event::ContinueStream(_))),
        "an even first byte resumes the request, and the calls were {calls:?}"
    );
}

#[test]
fn the_auth_guest_answers_403_on_an_odd_byte() {
    // Arrange
    let example = Example::new(HTTP_AUTH_RANDOM);
    let (mut guest, root) = example.started("");
    let state = StreamDouble::new(&example.recorder);
    let (context, state) = stream(&mut guest, root, state);
    let (opened, state) = guest.with(state, |scope| scope.on_request_headers(context, 0, true));

    // Act
    let (delivered, _state) = guest.with(state, |scope| {
        scope.on_http_call_response(
            context,
            callout(1),
            HttpCallResponse::received(vec![pair(":status", "200")])
                .with_body(Cow::Borrowed(b"\x03")),
        )
    });

    // Assert
    assert_eq!(opened.unwrap(), Action::Pause);
    assert!(delivered.is_ok());
    let answered = example
        .recorder
        .calls()
        .into_iter()
        .any(|event| matches!(event, Event::LocalResponse { status: 403, .. }));
    assert!(answered, "an odd first byte forbids the request");
}

#[test]
fn the_auth_guest_traps_when_the_open_of_its_callout_is_refused() {
    // Arrange
    let example = Example::new(HTTP_AUTH_RANDOM);
    let (mut guest, root) = example.started("");
    let state = StreamDouble::new(&example.recorder);
    let (context, state) = stream(&mut guest, root, state);
    example.recorder.refuse(
        CalloutCall::HttpCall,
        CalloutRefusal::Http(HttpCallRefusal::UnknownUpstream),
    );

    // Act
    let (outcome, _state) = guest.with(state, |scope| scope.on_request_headers(context, 0, true));

    // Assert
    assert!(
        outcome.is_err(),
        "the guest unwraps the open of its callout"
    );
    assert!(guest.is_poisoned(), "a trap poisons the guest");
}

#[test]
fn the_grpc_guest_opens_a_grpc_callout() {
    // Arrange
    let example = Example::new(GRPC_AUTH_RANDOM);
    let (mut guest, root) = example.started("");
    let state = StreamDouble::new(&example.recorder)
        .with_request_headers(&[("content-type", "application/grpc"), (":path", "/x")]);
    let (context, state) = stream(&mut guest, root, state);
    example.recorder.clear();

    // Act
    let (action, _state) = guest.with(state, |scope| scope.on_request_headers(context, 2, true));

    // Assert
    assert_eq!(action.unwrap(), Action::Pause);
    let opened = example
        .recorder
        .calls()
        .into_iter()
        .find_map(|event| match event {
            Event::GrpcCall {
                service, method, ..
            } => Some((service, method)),
            _ => None,
        })
        .expect("the guest opens a gRPC callout");
    assert_eq!(
        opened,
        ("grpcbin.GRPCBin".to_owned(), "RandomError".to_owned())
    );
}

#[test]
fn the_grpc_guest_answers_a_grpc_status_on_an_odd_code() {
    // Arrange
    let example = Example::new(GRPC_AUTH_RANDOM);
    let (mut guest, root) = example.started("");
    let state = StreamDouble::new(&example.recorder)
        .with_request_headers(&[("content-type", "application/grpc"), (":path", "/x")]);
    let (context, state) = stream(&mut guest, root, state);
    let (_, state) = guest.with(state, |scope| scope.on_request_headers(context, 2, true));
    example.recorder.clear();

    // Act
    let (delivered, _state) = guest.with(state, |scope| {
        scope.on_grpc_close(context, callout(1), GrpcStatus::new(3, ""))
    });

    // Assert
    assert!(delivered.is_ok());
    let answered = example
        .recorder
        .calls()
        .into_iter()
        .find_map(|event| match event {
            Event::LocalResponse {
                status,
                headers,
                body,
                grpc_status,
                ..
            } => Some((status, headers, body, grpc_status)),
            _ => None,
        })
        .expect("an odd status makes the guest answer the request itself");
    assert_eq!(
        answered.3,
        Some(10),
        "the guest answers with the gRPC status Aborted"
    );
    assert_eq!(
        answered.0, 200,
        "the SDK sends a gRPC answer as an HTTP 200 that carries the gRPC status"
    );
    assert_eq!(
        answered.2, "Aborted by Proxy-Wasm!",
        "the SDK sends the gRPC message as the body"
    );
    assert_eq!(
        answered.1,
        vec![("Powered-By".to_owned(), "proxy-wasm".to_owned())]
    );
}

#[test]
fn the_metadata_guest_reads_a_property_path_of_four_elements() {
    // Arrange
    let example = Example::new(ENVOY_FILTER_METADATA);
    let (mut guest, root) = example.started("");
    let mut state = StreamDouble::new(&example.recorder);
    state.properties.insert(
        vec![
            "metadata".to_owned(),
            "filter_metadata".to_owned(),
            "envoy.filters.http.lua".to_owned(),
            "uppercased-custom-metadata".to_owned(),
        ],
        "SHOUTED".to_owned(),
    );
    let (context, state) = stream(&mut guest, root, state);
    example.recorder.clear();

    // Act
    let (action, _state) = guest.with(state, |scope| scope.on_request_headers(context, 0, true));

    // Assert
    assert_eq!(action.unwrap(), Action::Pause);
    let response = example
        .recorder
        .calls()
        .into_iter()
        .find_map(|event| match event {
            Event::LocalResponse { headers, body, .. } => Some((headers, body)),
            _ => None,
        })
        .expect("the guest answers with the metadata it read");
    assert!(
        response
            .0
            .contains(&("uppercased-metadata".to_owned(), "SHOUTED".to_owned())),
        "the value of the four element path reaches the answer"
    );
    assert_eq!(
        response.1,
        "Custom response with Envoy metadata: \"SHOUTED\"\n"
    );
}
