//! A guest that adds a `Wasm-Context` request header holding its context
//! identifier.

use proxy_wasm::hostcalls;
use proxy_wasm::traits::{Context, HttpContext};
use proxy_wasm::types::{Action, LogLevel};

proxy_wasm::main! {{
    proxy_wasm::set_log_level(LogLevel::Trace);
    proxy_wasm::set_http_context(|context_id, _root_context_id| -> Box<dyn HttpContext> {
        Box::new(AddRequestHeader { context_id })
    });
}}

struct AddRequestHeader {
    context_id: u32,
}

impl Context for AddRequestHeader {}

impl HttpContext for AddRequestHeader {
    fn on_http_request_headers(&mut self, _num_headers: usize, _end_of_stream: bool) -> Action {
        let _ = hostcalls::log(LogLevel::Info, "adding header");
        self.set_http_request_header("Wasm-Context", Some(&self.context_id.to_string()));
        Action::Continue
    }
}
