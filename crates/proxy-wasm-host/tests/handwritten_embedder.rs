//! An embedder written by hand, with no test support of this crate.
//!
//! Every double below implements a public trait and nothing else, so the
//! tests prove that an embedder can serve a guest from the public API alone.
//! `tests/exports.rs` holds the list of names that surface is made of.
//!
//! The helpers below are test code, and the allowance clippy makes for a test
//! does not reach a function of an integration test that carries no test
//! attribute.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use proxy_wasm_host::abi::UnsupportedAbi;
use proxy_wasm_host::abi::v0_2_1::types::{Action, LogLevel, MapType, PeerType, Status};
use proxy_wasm_host::abi::v0_2_1::{
    Access, CallScope, CalloutId, Callouts, ContextId, ContextProblem, GrpcCall, GrpcOpenRefusal,
    GrpcStatus, GrpcStream, Guest, GuestError, GuestId, Host, Invocation, LogContext, LogSink,
    PluginConfig, StreamKind, StreamState, VmServices,
};
use proxy_wasm_host::{AbiVersion, Engine, Error, HeaderMap, Limits, Module, VecHeaderMap};

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
    fn log(&self, _: LogContext<'_>, _: LogLevel, _: &[u8]) {}
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

/// The names a worker writes when it serves a stream of either family.
#[test]
fn an_embedder_names_the_stream_surface_in_its_own_signatures() {
    // Arrange
    fn declare(guest: &mut Guest, stream: ContextId, kind: StreamKind) -> Result<(), GuestError> {
        guest.expect_stream_kind(stream, kind)
    }
    fn serve_connection<H: StreamState>(
        scope: &mut CallScope<'_, H>,
        stream: ContextId,
        chunk: &[u8],
    ) -> Result<Action, GuestError> {
        scope.on_new_connection(stream)?;
        let action =
            scope.on_downstream_data(stream, chunk.len().try_into().unwrap_or(u32::MAX), true)?;
        scope.on_downstream_connection_close(stream, PeerType::Remote)?;
        Ok(action)
    }
    let wat = r#"(module
        (memory (export "memory") 1)
        (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 1024)
        (func (export "proxy_abi_version_0_2_1")))"#;
    let engine = Engine::new().unwrap();
    let module = Module::new(&engine, &wat::parse_str(wat).unwrap()).unwrap();
    let host = Host::new(&engine).unwrap();
    let services = VmServices::new(std::sync::Arc::new(Discard));
    let mut guest = Guest::new(&host, &module, services, &Limits::default()).unwrap();
    let root = guest.enter_root().on_context_create(None).unwrap();
    let stream = guest.enter_root().on_context_create(Some(root)).unwrap();
    let declared = declare(&mut guest, stream, StreamKind::Tcp);
    let mut scope = guest.enter(Request {
        headers: VecHeaderMap::default(),
    });

    // Act
    let action = serve_connection(&mut scope, stream, b"bytes");

    // Assert
    assert!(declared.is_ok(), "{declared:?}");
    assert!(matches!(action, Ok(Action::Continue)), "{action:?}");
    drop(scope.finish());
    assert_eq!(guest.context_stream_kind(stream), Some(StreamKind::Tcp));
}
