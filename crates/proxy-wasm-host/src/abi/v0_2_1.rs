//! Proxy-Wasm ABI v0.2.1.
//!
//! The reference text is `abi-versions/v0.2.1/README.md` in the Proxy-Wasm
//! spec repository.
//!
//! [`Guest`] binds an instance to this ABI and drives its callbacks through
//! a [`CallScope`].
//! You lend a request to a scope as a [`StreamHost`].

pub mod types;

mod call_scope;
mod callback;
mod context;
mod guest;
pub(crate) mod host_functions;
mod stream_host;
#[cfg(test)]
pub(crate) mod test_support;

pub use call_scope::CallScope;
pub use callback::Callback;
pub use context::{ContextId, ContextProblem, ContextState, ContextType, InvalidContextId};
pub use guest::Guest;
pub use stream_host::{Access, HostCall, NoStream, StreamHost};

pub(crate) use context::ContextTable;
