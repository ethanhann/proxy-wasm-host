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
//! You run a guest with [`Guest`], [`CallScope`], and [`PluginConfig`], and
//! [`Callback`] names the callback a report is about.
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

pub mod types;

mod call_scope;
mod callback;
mod context;
mod guest;
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
pub use plugin_config::PluginConfig;
pub use services::{Clock, LogSink, SystemClock, VmServices};
pub use shared_services::{
    InMemoryStore, InMemoryStoreLimits, InvalidMetricId, InvalidQueueId, MetricId, QueueId,
    SharedServices, SharedValue,
};
pub use stream_state::values::{CalloutStatus, ForeignCall, HeaderPairs, LocalResponse};
pub use stream_state::{Access, Invocation, NoStream, StreamState};

pub(crate) use context::table::ContextTable;
pub(crate) use state::{AbiAccess, AbiState};
