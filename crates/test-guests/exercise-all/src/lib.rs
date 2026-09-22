//! A guest that calls every host function the Rust SDK reaches and logs what
//! it reads, one line for each step.
//!
//! The root id of a plugin is `http` or `tcp`, and it chooses the family of
//! the stream contexts under that root.
//!
//! A test changes what the guest does through these switches:
//!
//! - The VM configuration `refuse` makes `on_vm_start` answer false.
//! - The plugin configuration `refuse` makes `on_configure` answer false, and
//!   `panic` makes it panic, both after the HTTP root opened its callouts.
//! - The request header `x-exercise: panic` makes the request headers panic.
//! - `x-exercise: local` sends a local response.
//! - `x-exercise: defer` makes `on_done` answer false, and the next tick
//!   completes the context.

mod http;
mod root;
mod tcp;

use proxy_wasm::hostcalls;
use proxy_wasm::traits::RootContext;
use proxy_wasm::types::{Bytes, LogLevel};

proxy_wasm::main! {{
    proxy_wasm::set_log_level(LogLevel::Trace);
    proxy_wasm::set_root_context(|context_id| -> Box<dyn RootContext> {
        Box::new(root::Root::new(context_id))
    });
}}

/// Logs one line at the info level.
fn info(line: &str) {
    let _ = hostcalls::log(LogLevel::Info, line);
}

/// The bytes as text, or an empty text when there are none.
fn text(bytes: Option<Bytes>) -> String {
    bytes
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default()
}

/// Pairs as `key:value` separated by commas.
fn pairs(pairs: &[(String, Bytes)]) -> String {
    pairs
        .iter()
        .map(|(key, value)| format!("{key}:{}", String::from_utf8_lossy(value)))
        .collect::<Vec<_>>()
        .join(",")
}
