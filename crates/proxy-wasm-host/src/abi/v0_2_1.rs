//! Proxy-Wasm ABI v0.2.1.
//!
//! The reference text is `abi-versions/v0.2.1/README.md` in the Proxy-Wasm
//! spec repository.
//!
//! [`Guest`] binds an instance to this ABI and drives its callbacks through
//! a [`CallScope`].
//! You lend a request to a scope as a [`StreamState`].
//!
//! # The surface
//!
//! You link the host functions once with [`Host`].
//! You run a guest with [`Guest`], [`CallScope`], and [`PluginConfig`], and
//! [`Callback`] names the callback a report is about.
//! You read why a guest or a callback failed through [`GuestError`], which
//! holds the refusals this version defines and the [`Error`](crate::Error)
//! of the runtime.
//! You serve a request by implementing [`StreamState`], whose methods receive
//! an [`Invocation`] and an [`Access`] and exchange [`HeaderPairs`],
//! [`LocalResponse`], [`ForeignCall`], and [`CalloutStatus`] values.
//! [`NoStream`] is the stream state of a root context.
//! You follow a context with [`ContextId`], [`ContextType`], and
//! [`ContextState`], and you read a refusal through [`ContextProblem`] and
//! [`InvalidContextId`].
//! You give a guest its log, its clock, and its configuration through
//! [`VmServices`], [`LogSink`], [`Clock`], and [`SystemClock`].
//! You share data, queues, and metrics between guests by implementing
//! [`SharedServices`] or by using [`InMemoryStore`] with
//! [`InMemoryStoreLimits`], and the values involved are [`SharedValue`],
//! [`QueueId`], [`MetricId`], [`InvalidQueueId`], and [`InvalidMetricId`].
//! The enumerations the ABI defines are under [`types`].
//! [`WasmParams`] and [`WasmResults`] bound the types of
//! [`Guest::call_export`].

pub mod types;

mod call_scope;
mod callback;
mod context;
mod guest;
mod guest_error;
mod host;
pub(crate) mod host_functions;
mod plugin_config;
mod services;
mod shared_services;
mod state;
mod stream_state;
#[cfg(test)]
pub(crate) mod test_support;
pub(crate) mod unserved;
pub(crate) mod wasi;

pub use call_scope::CallScope;
pub use callback::Callback;
pub use context::{ContextId, ContextProblem, ContextState, ContextType, InvalidContextId};
pub use guest::Guest;
pub use guest_error::GuestError;
pub use host::Host;
pub use plugin_config::PluginConfig;
pub use services::{Clock, LogSink, SystemClock, VmServices};
pub use shared_services::{
    InMemoryStore, InMemoryStoreLimits, InvalidMetricId, InvalidQueueId, MetricId, QueueId,
    SharedServices, SharedValue,
};
pub use stream_state::values::{CalloutStatus, ForeignCall, HeaderPairs, LocalResponse};
pub use stream_state::{Access, Invocation, NoStream, StreamState};
/// The traits that bound the parameters and the results of
/// [`Guest::call_export`].
///
/// A change of the wasmtime major version is a breaking change of this
/// crate.
pub use wasmtime::{WasmParams, WasmResults};

pub(crate) use context::table::ContextTable;
pub(crate) use state::{AbiAccess, AbiState};
