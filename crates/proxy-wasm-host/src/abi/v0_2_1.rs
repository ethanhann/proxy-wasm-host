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
//! [`LocalResponse`] and [`ForeignCall`] values.
//! You receive the HTTP calls of a guest by implementing [`Callouts`], which
//! gives you a [`CalloutId`] and an [`HttpCall`] and takes an
//! [`HttpCallRefusal`].
//! You give the result back as an [`HttpCallResponse`], and
//! [`CalloutProblem`] and [`InvalidCalloutId`] tell you why a callout was
//! refused.
//! [`Guest::open_callouts`] gives an [`OpenCallout`] for each open callout,
//! with its [`CalloutKind`].
//! You learn what a guest changed through [`Changes`], which names each
//! [`QueueRegistration`], and an
//! [`InMemoryStore`] tells you of a queue item through [`QueueEnqueued`],
//! with [`QueueProblem`] as the refusal of a queue callback.
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
mod callout;
mod callout_service;
mod changes;
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
pub use callout::{CalloutId, CalloutKind, CalloutProblem, InvalidCalloutId, OpenCallout};
pub use callout_service::{Callouts, HttpCall, HttpCallRefusal, HttpCallResponse};
pub use changes::{Changes, QueueRegistration};
pub use context::{ContextId, ContextProblem, ContextState, ContextType, InvalidContextId};
pub use guest::Guest;
pub use guest_error::GuestError;
pub use host::Host;
pub use plugin_config::PluginConfig;
pub use services::{Clock, LogSink, SystemClock, VmServices};
pub use shared_services::{
    InMemoryStore, InMemoryStoreLimits, InvalidMetricId, InvalidQueueId, MetricId, QueueEnqueued,
    QueueId, QueueProblem, SharedServices, SharedValue,
};
pub use stream_state::values::{ForeignCall, HeaderPairs, LocalResponse};
pub use stream_state::{Access, Invocation, NoStream, StreamState};
/// The traits that bound the parameters and the results of
/// [`Guest::call_export`].
///
/// A change of the wasmtime major version is a breaking change of this
/// crate.
#[doc(no_inline)]
pub use wasmtime::{WasmParams, WasmResults};

pub(crate) use context::table::ContextTable;
pub(crate) use state::{AbiAccess, AbiState};

/// Builds the ABI state of one instance, boxed for the store data to hold.
///
/// The runtime keeps this as an opaque value and never reads inside it, so
/// the ABI layer adds state without a change under `runtime/`.
/// The runtime decides when an instance gets one, and the ABI layer decides
/// what it is.
pub(crate) fn state(services: VmServices) -> Box<dyn std::any::Any + Send> {
    Box::new(AbiState::new(services))
}
