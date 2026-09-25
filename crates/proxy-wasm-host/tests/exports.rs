//! The list of names an embedder writes, at the paths it writes them by.
//!
//! The rule is that the surface is every type that appears in the signature of a
//! method an embedder implements, plus the entry points and the error types.
//! The rule has two halves, and this file holds both.
//! The import block below lists each item, so a name that is dropped rather
//! than moved stops this file compiling.
//! An import cannot prove that a name is absent, so the three tests compare
//! the export statements of the crate as text.
//! `tests/handwritten_embedder.rs` runs a guest with these names and nothing
//! else.
// Naming each item is the test, so an unused import here is the point.
#![allow(unused_imports)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

// What no ABI version owns.
use proxy_wasm_host::{
    AbiVersion, Buffer, Engine, EngineConfig, Error, HeaderMap, Limit, Limits, MemoryError, Module,
    NotAllowed, OptLevel, VecHeaderMap,
};
// What ABI v0.2.1 defines.
use proxy_wasm_host::abi::UnsupportedAbi;
use proxy_wasm_host::abi::v0_2_1::types::{
    Action, BufferType, LogLevel, MapType, MetricType, Status, StreamType,
};
use proxy_wasm_host::abi::v0_2_1::{
    Access, CallScope, Callback, CalloutId, CalloutKind, CalloutProblem, Callouts, Changes, Clock,
    ContextId, ContextProblem, ContextState, ContextType, ForeignCall, GrpcCall, GrpcOpenRefusal,
    GrpcStatus, GrpcStream, Guest, GuestError, GuestId, GuestSpec, HeaderPairs, Host, HttpCall,
    HttpCallRefusal, HttpCallResponse, InMemoryStore, InMemoryStoreLimits, InvalidCalloutId,
    InvalidContextId, InvalidMetricId, InvalidQueueId, Invocation, LocalResponse, LogContext,
    LogSink, MetricId, NoStream, OpenCallout, PluginConfig, QueueEnqueued, QueueId, QueueProblem,
    QueueRegistration, SharedServices, SharedValue, Started, StreamKind, StreamState, SystemClock,
    VmServices, WasmParams, WasmResults,
};
// The codec types that appear in a signature an embedder writes.
use proxy_wasm_host::codec::pairs::{EncodeError, PairVisitor, Pairs};

// The rest of the public surface, which an embedder uses less often.
use proxy_wasm_host::abi::v0_2_1::types::{
    PeerType, UnknownValue, WasiClockId, WasiErrno, WasiFdId,
};
use proxy_wasm_host::codec::pairs::{
    COUNT_SIZE, DecodeError, Field, PairLimits, PairSource, decode_pairs, encode_pairs,
    encode_visited, encoded_size, pair_encoded_size, total_size,
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
            "pub use runtime::{Engine, EngineConfig, Limits, Module, OptLevel};",
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
            "pub use context::{ ContextId, ContextProblem, ContextState, ContextType, InvalidContextId, StreamKind, };",
            "pub use guest::Guest;",
            "pub use guest::identity::GuestId;",
            "pub use guest::start::Started;",
            "pub use guest_error::GuestError;",
            "pub use guest_spec::GuestSpec;",
            "pub use host::Host;",
            "pub use plugin_config::PluginConfig;",
            "pub use services::{Clock, LogContext, LogSink, SystemClock, VmServices};",
            "pub use shared_services::{ InMemoryStore, InMemoryStoreLimits, InvalidMetricId, InvalidQueueId, MetricId, QueueEnqueued, QueueId, QueueProblem, SharedServices, SharedValue, };",
            "pub use stream_state::values::{ForeignCall, HeaderPairs, LocalResponse};",
            "pub use stream_state::{Access, Invocation, NoStream, StreamState};",
            "pub use wasmtime::{WasmParams, WasmResults};",
            "pub use proxy::{Action, BufferType, LogLevel, MapType, MetricType, PeerType, Status, StreamType};",
            "pub use wasi::{WasiClockId, WasiErrno, WasiFdId};",
        ]
    );
}
