//! The cost of building a guest, of starting its root, and of one request.
#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used)]

use std::hint::black_box;
use std::sync::Arc;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use proxy_wasm_host::abi::v0_2_1::types::{LogLevel, MapType, Status};
use proxy_wasm_host::abi::v0_2_1::{
    Access, ContextId, GuestSpec, Host, Invocation, LogContext, LogSink, PluginConfig, StreamKind,
    StreamState, VmServices,
};
use proxy_wasm_host::{Engine, HeaderMap, Limits, Module, VecHeaderMap};

const ADD_REQUEST_HEADER: &[u8] = include_bytes!("../tests/fixtures/add-request-header.wasm");
const EXERCISE_ALL: &[u8] = include_bytes!("../tests/fixtures/exercise-all.wasm");

/// A guest with no import and no start function, so its build is the part of
/// the work that no guest can avoid.
const BARE: &str = r#"(module
    (memory (export "memory") 1)
    (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 1024)
    (func (export "proxy_abi_version_0_2_1")))"#;

struct Discard;

impl LogSink for Discard {
    fn log(&self, _: LogContext<'_>, _: LogLevel, _: &[u8]) {}
}

#[derive(Default)]
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

fn spec(bytes: &[u8]) -> GuestSpec {
    let engine = Engine::new().unwrap();
    let module = Module::new(&engine, bytes).unwrap();
    let services = VmServices::new(Arc::new(Discard));
    GuestSpec::new(
        &Host::new(&engine).unwrap(),
        &module,
        services,
        &Limits::default(),
    )
    .unwrap()
}

/// The build of a guest, with the drop of the guest outside the timing.
///
/// The bare module shows the instantiation and the lookup of the callbacks,
/// and the two modules of the Rust SDK add the start function of the SDK.
fn guest_build(c: &mut Criterion) {
    let bare = wat::parse_str(BARE).unwrap();
    let modules = [
        ("bare", bare.as_slice()),
        ("add-request-header", ADD_REQUEST_HEADER),
        ("exercise-all", EXERCISE_ALL),
    ];
    let mut group = c.benchmark_group("guest_build");
    for (name, bytes) in modules {
        let spec = spec(bytes);
        group.bench_function(name, |b| {
            b.iter_with_large_drop(|| spec.build().unwrap());
        });
    }
    group.finish();
}

/// The start of one root, with the build and the drop outside the timing.
fn guest_start(c: &mut Criterion) {
    let spec = spec(ADD_REQUEST_HEADER);
    c.bench_function("guest_start", |b| {
        b.iter_batched(
            || spec.build().unwrap(),
            |mut guest| {
                let started = guest.start(PluginConfig::new()).unwrap();
                (guest, started)
            },
            BatchSize::SmallInput,
        );
    });
}

fn request_lifecycle(c: &mut Criterion) {
    let spec = spec(ADD_REQUEST_HEADER);
    let mut guest = spec.build().unwrap();
    let root = guest.start(PluginConfig::new()).unwrap().root();
    c.bench_function("request_lifecycle", |b| {
        b.iter(|| {
            let (answer, request) = guest.with(Request::default(), |scope| {
                let stream: ContextId = scope.on_context_create(Some(root))?;
                scope.expect_stream_kind(stream, StreamKind::Http)?;
                scope.on_request_headers(stream, 0, false)?;
                scope.on_request_body(stream, 0, true)?;
                scope.on_response_headers(stream, 0, false)?;
                scope.on_response_body(stream, 0, true)?;
                scope.on_done(stream)?;
                scope.on_log(stream)?;
                scope.on_delete(stream)
            });
            black_box((answer.unwrap(), request))
        });
    });
}

criterion_group!(benches, guest_build, guest_start, request_lifecycle);
criterion_main!(benches);
