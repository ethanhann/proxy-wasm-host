//! Proxy-Wasm ABI v0.2.1.
//!
//! The reference text is `abi-versions/v0.2.1/README.md` in the Proxy-Wasm
//! spec repository.
//!
//! [`Guest`] binds an instance to this ABI and drives its callbacks through
//! a [`CallScope`].
//! You lend a request to a scope as a [`StreamState`].

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
