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
use proxy_wasm_host::abi::v0_2_1::types::{
    Action, BufferType, LogLevel, MapType, MetricType, Status, StreamType,
};
use proxy_wasm_host::abi::v0_2_1::{
    Access, CallScope, Callback, CalloutStatus, Clock, ContextId, ContextProblem, ContextState,
    ContextType, ForeignCall, Guest, HeaderPairs, InMemoryStore, InMemoryStoreLimits,
    InvalidContextId, InvalidMetricId, InvalidQueueId, Invocation, LocalResponse, LogSink,
    MetricId, NoStream, PluginConfig, QueueId, SharedServices, SharedValue, StreamState,
    SystemClock, VmServices,
};
// What the codec names in a signature an embedder writes.
use proxy_wasm_host::codec::pairs::{EncodeError, PairVisitor, Pairs};

/// Every name above is reachable, which the imports prove at compile time.
#[test]
fn the_surface_is_reachable_at_its_paths() {
    // Arrange
    let version = AbiVersion::V0_2_1;

    // Act
    let named = format!("{version:?}");

    // Assert
    assert!(!named.is_empty());
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
