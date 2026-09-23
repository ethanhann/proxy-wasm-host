//! Proxy-Wasm ABI v0.2.1.
//!
//! The reference text is `abi-versions/v0.2.1/README.md` in the Proxy-Wasm
//! spec repository.
//!
//! [`Guest`] binds an instance to this ABI and drives its callbacks through
//! a [`CallScope`].
//! You lend a request to a scope as a [`StreamState`].
//!
//! # Return pointers
//!
//! A host function that answers a value takes the address to write it to,
//! and the crate treats that address as an ordinary one.
//! The address zero is a legal address for every host function but
//! `proxy_call_foreign_function`, where the ABI document calls the return
//! values optional and the crate reads zero as "I do not want this value".
//! A guest cannot lose a value that way, because the toolchains leave the
//! first page of memory unused so that a null pointer traps.
//!
//! # The host functions
//!
//! A guest imports 39 functions from the module `env`.
//! The crate registers every one of them for every guest.
//! The table says who answers each call.
//!
//! The second column names who answers.
//! "The crate" means that no method of yours runs.
//! The third column names the method that answers the call.
//! Most are trait methods you implement.
//! `proxy_get_log_level` answers from the value you set with
//! [`VmServices::with_log_level`].
//!
//! Four answers are left out of the last column, because they belong to many
//! rows at once.
//! A guest that passes an address the crate cannot read or write receives
//! `INVALID_MEMORY_ACCESS`.
//! A guest that passes a value outside the enum an argument names receives
//! `BAD_ARGUMENT`.
//! A guest whose allocator answers null receives `INTERNAL_FAILURE`, which
//! reaches every function that gives the guest a value.
//! An implementation of yours that refuses with `Status::Ok` also receives
//! `INTERNAL_FAILURE`, because the crate refuses to report a success as a
//! failure.
//!
//! A method that answers `Result<_, Status>` sends your own refusal straight
//! to the guest.
//! [`StreamState`] and [`SharedServices`] list the status each one gives when
//! you do not implement it.
//!
//! The last column is written by hand, and the test that reads this table
//! compares the names and the shape of each row and not that column.
//!
//! | Function | Served by | Method | Also answers |
//! |---|---|---|---|
//! | `proxy_done` | the crate | none | `NOT_FOUND` |
//! | `proxy_set_effective_context` | the crate | none | `BAD_ARGUMENT` |
//! | `proxy_log` | the services | [`LogSink::log`] | none |
//! | `proxy_get_log_level` | the services | [`VmServices::with_log_level`] | none |
//! | `proxy_get_current_time_nanoseconds` | the services | [`Clock::realtime_nanos`] | none |
//! | `proxy_set_tick_period_milliseconds` | the crate | none | `BAD_ARGUMENT` |
//! | `proxy_set_buffer_bytes` | the stream state | [`StreamState::buffer`] | `NOT_FOUND` |
//! | `proxy_get_buffer_bytes` | the crate for the two configurations and for a delivery, else the stream state | [`StreamState::buffer`] | `NOT_FOUND`, `BAD_ARGUMENT` for a start past the end |
//! | `proxy_get_buffer_status` | the same | [`StreamState::buffer`] | `NOT_FOUND`, `INTERNAL_FAILURE` |
//! | `proxy_get_header_map_size` | the crate for a delivery, else the stream state | [`StreamState::header_map`] | `BAD_ARGUMENT`, `SERIALIZATION_FAILURE` |
//! | `proxy_get_header_map_pairs` | the same | [`StreamState::header_map`] | `BAD_ARGUMENT`, `SERIALIZATION_FAILURE` |
//! | `proxy_set_header_map_pairs` | the same | [`StreamState::header_map`] | `BAD_ARGUMENT` |
//! | `proxy_get_header_map_value` | the same | [`StreamState::header_map`] | `BAD_ARGUMENT`, `NOT_FOUND` |
//! | `proxy_add_header_map_value` | the same | [`StreamState::header_map`] | `BAD_ARGUMENT` |
//! | `proxy_replace_header_map_value` | the same | [`StreamState::header_map`] | `BAD_ARGUMENT` |
//! | `proxy_remove_header_map_value` | the same | [`StreamState::header_map`] | `BAD_ARGUMENT` |
//! | `proxy_continue_stream` | the stream state | [`StreamState::continue_stream`] | `UNIMPLEMENTED` |
//! | `proxy_close_stream` | the stream state | [`StreamState::close_stream`] | `UNIMPLEMENTED` |
//! | `proxy_get_status` | the crate | none | `NOT_FOUND` |
//! | `proxy_send_local_response` | the stream state | [`StreamState::send_local_response`] | `UNIMPLEMENTED` |
//! | `proxy_http_call` | the callouts | [`Callouts::http_call`] | `BAD_ARGUMENT`, `INTERNAL_FAILURE` |
//! | `proxy_grpc_call` | the callouts | [`Callouts::grpc_call`] | `PARSE_FAILURE`, `INTERNAL_FAILURE` |
//! | `proxy_grpc_stream` | the callouts | [`Callouts::grpc_stream`] | `PARSE_FAILURE`, `INTERNAL_FAILURE` |
//! | `proxy_grpc_send` | the callouts | [`Callouts::grpc_send`] | `NOT_FOUND` |
//! | `proxy_grpc_cancel` | the callouts | [`Callouts::grpc_cancel`] | `NOT_FOUND` |
//! | `proxy_grpc_close` | the callouts | [`Callouts::grpc_close`] and [`Callouts::grpc_cancel`] | `NOT_FOUND` |
//! | `proxy_set_shared_data` | the shared services | [`SharedServices::set_shared_data`] | `NOT_FOUND` |
//! | `proxy_get_shared_data` | the shared services | [`SharedServices::get_shared_data`] | `NOT_FOUND` |
//! | `proxy_register_shared_queue` | the shared services | [`SharedServices::register_shared_queue`] | `NOT_FOUND` |
//! | `proxy_resolve_shared_queue` | the shared services | [`SharedServices::resolve_shared_queue`] | `NOT_FOUND` |
//! | `proxy_enqueue_shared_queue` | the shared services | [`SharedServices::enqueue_shared_queue`] | `NOT_FOUND` |
//! | `proxy_dequeue_shared_queue` | the shared services | [`SharedServices::dequeue_shared_queue`] | `NOT_FOUND` |
//! | `proxy_define_metric` | the shared services | [`SharedServices::define_metric`] | `NOT_FOUND` |
//! | `proxy_record_metric` | the shared services | [`SharedServices::record_metric`] | `NOT_FOUND` |
//! | `proxy_increment_metric` | the shared services | [`SharedServices::increment_metric`] | `NOT_FOUND` |
//! | `proxy_get_metric` | the shared services | [`SharedServices::get_metric`] | `NOT_FOUND` |
//! | `proxy_get_property` | the crate for the three plugin properties, else the stream state | [`StreamState::property`] | `NOT_FOUND` |
//! | `proxy_set_property` | the stream state | [`StreamState::set_property`] | `NOT_FOUND` |
//! | `proxy_call_foreign_function` | the stream state | [`StreamState::call_foreign_function`] | `NOT_FOUND` |
//!
//! # What the crate bounds
//!
//! A guest chooses how much work it asks for, so the crate bounds the work
//! one call can cost.
//!
//! | Bound | Default | Where you change it |
//! |---|---|---|
//! | The pairs one map a guest sends may declare | 1024 | [`Limits::with_max_decoded_pairs`](crate::Limits::with_max_decoded_pairs) |
//! | The bytes one map a guest sends may hold | 1 MiB | [`Limits::with_max_decoded_map_bytes`](crate::Limits::with_max_decoded_map_bytes) |
//! | The callouts one guest may hold open | 1024 | [`VmServices::with_max_open_callouts`] |
//! | The bytes of one shared value | 64 KiB | [`InMemoryStore::with_limits`] |
//! | The keys of the shared store | 4096 | [`InMemoryStore::with_limits`] |
//! | The items of one shared queue | 1024 | [`InMemoryStore::with_limits`] |
//! | The CPU time of one guest call | one second | [`Limits::with_cpu_time`](crate::Limits::with_cpu_time) |
//! | The memory of one instance | 128 MiB | [`Limits::with_memory_bytes`](crate::Limits::with_memory_bytes) |
//! | The WASI functions a guest may import | the eight the ABI names | fixed |
//!
//! The first three come from the C++ host that Envoy runs, so a guest that
//! host accepts is accepted here.
//! The store rows and the two instance rows are limits of this crate.
//! No other host applies them.
//!
//! The WASI surface is fixed.
//! The crate registers the eight functions the ABI document names, which are
//! `fd_write`, `clock_time_get`, `random_get`, `environ_sizes_get`,
//! `environ_get`, `args_sizes_get`, `args_get`, and `proc_exit`.
//! A guest that imports any other name from `wasi_snapshot_preview1` fails to
//! instantiate.
//! Every guest this project builds imports at most those eight, which covers
//! its own three guests, a `TinyGo` fixture, and seven example plugins of the
//! Rust SDK.
//! A guest built by the Go compiler imports more and does not load.
//!
//! # Where a guest sees another answer than on Envoy
//!
//! The C++ host is the one Envoy runs, so a plugin author tested against it.
//! Four rules of this crate answer a guest differently.
//! Each one closes a way for one request to reach another.
//!
//! 1. `proxy_set_effective_context` accepts a context under the root of the
//!    running callback alone. That host accepts any context of the virtual
//!    machine, so a request of one plugin can read the configuration of
//!    another plugin in the same machine.
//! 2. A callout belongs to the context that opened it. That host sends every
//!    callout function through the root, so one request can cancel or feed
//!    the callout of another request of the same plugin.
//! 3. A callout whose headers lack `:authority`, `:method`, or `:path` is
//!    refused with `BAD_ARGUMENT`. The ABI document requires that check of
//!    the host, and that host leaves it to the proxy.
//! 4. `proxy_get_buffer_bytes` refuses a start past the end of the buffer
//!    with `BAD_ARGUMENT`, which the ABI document names for an invalid
//!    start. That host clamps the length to zero and answers `OK`.
//!
//! Three more answers differ.
//! The ABI document defines none of them.
//!
//! 1. `proxy_close_stream` and `proxy_send_local_response` answer
//!    `UNIMPLEMENTED` when no stream state serves the call.
//! 2. `proxy_get_status` answers `NOT_FOUND` whenever no delivery holds a
//!    status, which covers a call outside a callout callback and a call
//!    inside the foreign function callback.
//! 3. A root context under [`Guest::enter_root`] reads `plugin_name`,
//!    `plugin_root_id`, and `plugin_vm_id`, because the crate answers those
//!    three itself. Every other property, and every foreign function call,
//!    reaches [`NoStream`], which answers `NOT_FOUND`. Enter the scope with
//!    a stream state of your own when a root needs more than the three.
//!
//! # The surface
//!
//! You link the host functions once with [`Host`].
//! You run a guest with [`Guest`], [`CallScope`], and [`PluginConfig`], and
//! [`Callback`] names the callback a report is about.
//! You build a guest again with [`GuestSpec`], and [`Guest::start`] answers
//! [`Started`] for each root.
//! You read why a guest or a callback failed through [`GuestError`], which
//! holds the refusals this version defines and the [`Error`](crate::Error)
//! of the runtime.
//! You serve a request by implementing [`StreamState`], whose methods receive
//! an [`Invocation`] and an [`Access`] and exchange [`HeaderPairs`],
//! [`LocalResponse`] and [`ForeignCall`] values.
//! You receive the callouts of a guest by implementing [`Callouts`], which
//! gives you a [`CalloutId`] with an [`HttpCall`], a [`GrpcCall`], or a
//! [`GrpcStream`], and takes an [`HttpCallRefusal`] or a
//! [`GrpcOpenRefusal`].
//! You give the result back as an [`HttpCallResponse`] or as a
//! [`GrpcStatus`], and [`CalloutProblem`] and [`InvalidCalloutId`] tell you
//! why a callout was refused.
//! [`GuestId`] names the guest a callout belongs to.
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
//! A stream context serves one [`StreamKind`], which the crate records at its
//! first stream callback and
//! [`Guest::expect_stream_kind`] declares before it.
//! You give a guest its log, its clock, and its configuration through
//! [`VmServices`], [`LogSink`], [`Clock`], and [`SystemClock`].
//! [`LogSink::log`] receives a [`LogContext`], which says which guest, which
//! plugin, and which call wrote the line.
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
mod guest_spec;
mod host;
pub(crate) mod host_functions;
pub(crate) mod payload;
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
pub use callout_service::{
    Callouts, GrpcCall, GrpcOpenRefusal, GrpcStatus, GrpcStream, HttpCall, HttpCallRefusal,
    HttpCallResponse,
};
pub use changes::{Changes, QueueRegistration};
pub use context::{
    ContextId, ContextProblem, ContextState, ContextType, InvalidContextId, StreamKind,
};
pub use guest::Guest;
pub use guest::identity::GuestId;
pub use guest::start::Started;
pub use guest_error::GuestError;
pub use guest_spec::GuestSpec;
pub use host::Host;
pub use plugin_config::PluginConfig;
pub use services::{Clock, LogContext, LogSink, SystemClock, VmServices};
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
