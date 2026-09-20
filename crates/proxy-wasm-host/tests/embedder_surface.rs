//! The surface an embedder names, at the paths it names them by.
//!
//! The rule is that the surface is every type named in the signature of a
//! method an embedder implements, plus the entry points and the error types.
//! A name that is dropped rather than moved stops this file compiling.
// Naming each item is the test, so an unused import here is the point.
#![allow(unused_imports)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

// What no ABI version owns.
use proxy_wasm_host::{
    AbiVersion, Buffer, Engine, EngineConfig, Error, HeaderMap, Limit, Limits, MemoryError, Module,
    NotAllowed, VecHeaderMap,
};
// What ABI v0.2.1 defines.
use proxy_wasm_host::abi::UnsupportedAbi;
use proxy_wasm_host::abi::v0_2_1::types::{
    Action, BufferType, LogLevel, MapType, MetricType, Status, StreamType,
};
use proxy_wasm_host::abi::v0_2_1::{
    Access, CallScope, Callback, CalloutId, CalloutKind, CalloutProblem, Callouts, Changes, Clock,
    ContextId, ContextProblem, ContextState, ContextType, ForeignCall, GrpcCall, GrpcOpenRefusal,
    GrpcStatus, GrpcStream, Guest, GuestError, GuestId, HeaderPairs, Host, HttpCall,
    HttpCallRefusal, HttpCallResponse, InMemoryStore, InMemoryStoreLimits, InvalidCalloutId,
    InvalidContextId, InvalidMetricId, InvalidQueueId, Invocation, LocalResponse, LogSink,
    MetricId, NoStream, OpenCallout, PluginConfig, QueueEnqueued, QueueId, QueueProblem,
    QueueRegistration, SharedServices, SharedValue, StreamState, SystemClock, VmServices,
    WasmParams, WasmResults,
};
// What the codec names in a signature an embedder writes.
use proxy_wasm_host::codec::pairs::{EncodeError, PairVisitor, Pairs};

// The rest of the public surface, which an embedder names less often.
use proxy_wasm_host::abi::v0_2_1::types::{
    PeerType, UnknownValue, WasiClockId, WasiErrno, WasiFdId,
};
use proxy_wasm_host::codec::pairs::{
    COUNT_SIZE, DecodeError, Field, PairSource, decode_pairs, encode_pairs, encode_visited,
    encoded_size, pair_encoded_size, total_size,
};
use proxy_wasm_host::codec::path::{decode_path, encode_path};

/// Every `pub use` statement of `source`, on one line each.
///
/// An import above proves that a name exists.
/// It cannot prove that a name is absent, so the statements that export the
/// surface are compared as text.
fn exports_of(source: &str) -> Vec<String> {
    let mut statements = Vec::new();
    let mut open: Option<String> = None;
    for line in source.lines() {
        if open.is_none() && line.starts_with("pub use ") {
            open = Some(String::new());
        }
        if let Some(statement) = open.as_mut() {
            statement.push_str(line.trim());
            if line.trim_end().ends_with(';') {
                statements.extend(open.take());
            } else {
                statement.push(' ');
            }
        }
    }
    statements
}

fn source(path: &str) -> String {
    std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(path)).unwrap()
}

#[test]
fn the_crate_root_exports_only_names_that_no_abi_version_owns() {
    // Arrange
    let source = source("src/lib.rs");

    // Act
    let exports = exports_of(&source);

    // Assert
    assert_eq!(
        exports,
        [
            "pub use abi::AbiVersion;",
            "pub use buffer::Buffer;",
            "pub use error::{Error, Limit, MemoryError};",
            "pub use header_map::{HeaderMap, VecHeaderMap};",
            "pub use runtime::{Engine, EngineConfig, Limits, Module};",
        ]
    );
}

#[test]
fn the_abi_module_exports_the_version_and_its_refusal() {
    // Arrange
    let source = source("src/abi.rs");

    // Act
    let exports = exports_of(&source);

    // Assert
    assert_eq!(exports, ["pub use version::{AbiVersion, UnsupportedAbi};"]);
}

#[test]
fn the_versioned_module_exports_the_names_listed_here() {
    // Arrange
    let sources = [
        source("src/abi/v0_2_1.rs"),
        source("src/abi/v0_2_1/types.rs"),
    ];

    // Act
    let exports: Vec<String> = sources.iter().flat_map(|s| exports_of(s)).collect();

    // Assert
    assert_eq!(
        exports,
        [
            "pub use call_scope::CallScope;",
            "pub use callback::Callback;",
            "pub use callout::{CalloutId, CalloutKind, CalloutProblem, InvalidCalloutId, OpenCallout};",
            "pub use callout_service::{ Callouts, GrpcCall, GrpcOpenRefusal, GrpcStatus, GrpcStream, HttpCall, HttpCallRefusal, HttpCallResponse, };",
            "pub use changes::{Changes, QueueRegistration};",
            "pub use context::{ContextId, ContextProblem, ContextState, ContextType, InvalidContextId};",
            "pub use guest::Guest;",
            "pub use guest::identity::GuestId;",
            "pub use guest_error::GuestError;",
            "pub use host::Host;",
            "pub use plugin_config::PluginConfig;",
            "pub use services::{Clock, LogSink, SystemClock, VmServices};",
            "pub use shared_services::{ InMemoryStore, InMemoryStoreLimits, InvalidMetricId, InvalidQueueId, MetricId, QueueEnqueued, QueueId, QueueProblem, SharedServices, SharedValue, };",
            "pub use stream_state::values::{ForeignCall, HeaderPairs, LocalResponse};",
            "pub use stream_state::{Access, Invocation, NoStream, StreamState};",
            "pub use wasmtime::{WasmParams, WasmResults};",
            "pub use proxy::{Action, BufferType, LogLevel, MapType, MetricType, PeerType, Status, StreamType};",
            "pub use wasi::{WasiClockId, WasiErrno, WasiFdId};",
        ]
    );
}

/// A request that serves its headers and nothing else.
struct Request {
    headers: VecHeaderMap,
}

impl StreamState for Request {
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
}

struct Discard;

impl LogSink for Discard {
    fn log(&self, _: LogLevel, _: &[u8]) {}
}

#[test]
fn the_smallest_embedder_runs_a_stream_with_the_names_above() {
    // Arrange
    let wat = r#"(module
        (memory (export "memory") 1)
        (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 1024)
        (func (export "proxy_abi_version_0_2_1")))"#;
    let engine = Engine::new().unwrap();
    let module = Module::new(&engine, &wat::parse_str(wat).unwrap()).unwrap();
    let services = VmServices::new(std::sync::Arc::new(Discard));
    let host = Host::new(&engine).unwrap();
    let mut guest = Guest::new(&host, &module, services, &Limits::default()).unwrap();
    let mut root_scope = guest.enter_root();
    let root = root_scope.on_context_create(None).unwrap();
    let started = root_scope.on_vm_start(root).unwrap();
    let configured = root_scope.on_configure(root, PluginConfig::new()).unwrap();
    let stream = root_scope.on_context_create(Some(root)).unwrap();
    drop(root_scope);
    let request = Request {
        headers: VecHeaderMap::default(),
    };

    // Act
    let (answer, request) = guest.with(request, |scope| {
        let action = scope.on_request_headers(stream, 0, true)?;
        let done = scope.on_done(stream)?;
        scope.on_log(stream)?;
        scope.on_delete(stream)?;
        Ok::<_, GuestError>((action, done))
    });

    // Assert
    assert!(matches!(answer, Ok((Action::Continue, true))));
    assert!(started && configured);
    assert!(request.headers.is_empty());
    assert_eq!(guest.context_state(stream), None);
    assert_eq!(guest.abi(), AbiVersion::V0_2_1);
    assert!(!guest.is_poisoned());
}

/// The status prints the name the ABI uses, which an embedder logs.
#[test]
fn a_status_prints_the_name_the_abi_uses() {
    // Arrange
    let status = Status::NotFound;

    // Act
    let printed = status.to_string();

    // Assert
    assert_eq!(printed, "NOT_FOUND");
}

/// A guest whose request header callback traps.
const TRAPPER: &str = r#"(module
    (memory (export "memory") 1)
    (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 1024)
    (func (export "proxy_abi_version_0_2_1"))
    (func (export "proxy_on_request_headers") (param i32 i32 i32) (result i32) unreachable))"#;

fn trapper() -> Guest {
    let engine = Engine::new().unwrap();
    let host = Host::new(&engine).unwrap();
    let module = Module::new(&engine, &wat::parse_str(TRAPPER).unwrap()).unwrap();
    let services = VmServices::new(std::sync::Arc::new(Discard));
    Guest::new(&host, &module, services, &Limits::default()).unwrap()
}

#[test]
fn a_refusal_of_the_abi_is_read_at_one_level() {
    // Arrange
    let mut guest = trapper();
    let unknown = ContextId::try_from(9).unwrap();

    // Act
    let result = guest.enter_root().on_done(unknown);

    // Assert
    assert!(matches!(
        result,
        Err(GuestError::Context { id, problem: ContextProblem::Unknown }) if id == unknown
    ));
    assert!(!guest.is_poisoned());
}

#[test]
fn a_failure_of_the_runtime_is_read_at_two_levels() {
    // Arrange
    let mut guest = trapper();
    let root = guest.enter_root().on_context_create(None).unwrap();
    let stream = guest.enter_root().on_context_create(Some(root)).unwrap();

    // Act
    let result = guest.enter_root().on_request_headers(stream, 0, true);

    // Assert
    assert!(matches!(
        result,
        Err(GuestError::Runtime(Error::Trap { .. }))
    ));
    assert!(guest.is_poisoned());
}

#[test]
fn a_module_with_no_accepted_version_is_refused_by_name() {
    // Arrange
    let engine = Engine::new().unwrap();
    let host = Host::new(&engine).unwrap();
    let wat = r#"(module
        (memory (export "memory") 1)
        (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 1024)
        (func (export "proxy_abi_version_0_1_0")))"#;
    let module = Module::new(&engine, &wat::parse_str(wat).unwrap()).unwrap();
    let services = VmServices::new(std::sync::Arc::new(Discard));

    // Act
    let result = Guest::new(&host, &module, services, &Limits::default());

    // Assert
    assert!(matches!(
        result,
        Err(GuestError::UnsupportedAbi(UnsupportedAbi { found })) if found == ["proxy_abi_version_0_1_0"]
    ));
}

/// The names a worker writes when it serves a gRPC callout.
#[test]
fn an_embedder_names_the_grpc_surface_in_its_own_signatures() {
    // Arrange
    struct Outbox;
    impl Callouts for Outbox {
        fn grpc_call(
            &self,
            call: Invocation,
            _: CalloutId,
            request: GrpcCall<'_>,
        ) -> Result<(), GrpcOpenRefusal> {
            let _: GuestId = call.guest;
            if request.upstream.as_ref() == b"authz" {
                return Ok(());
            }
            Err(GrpcOpenRefusal::UnknownUpstream)
        }

        fn grpc_stream(
            &self,
            _: Invocation,
            _: CalloutId,
            _: GrpcStream<'_>,
        ) -> Result<(), GrpcOpenRefusal> {
            Ok(())
        }
    }
    let request = GrpcCall::new(
        std::borrow::Cow::Borrowed(b"authz"),
        std::borrow::Cow::Borrowed(b"example.Authz"),
        std::borrow::Cow::Borrowed(b"Check"),
    );
    let call = Invocation::new(GuestId::next(), ContextId::try_from(1).unwrap());

    // Act
    let answer = Outbox.grpc_call(call, CalloutId::try_from(1_u32).unwrap(), request);

    // Assert
    assert_eq!(answer, Ok(()));
    assert_eq!(GrpcStatus::new(14, "unavailable").code, 14);
    assert_eq!(
        Status::from(GrpcOpenRefusal::UnknownUpstream),
        Status::ParseFailure
    );
}
