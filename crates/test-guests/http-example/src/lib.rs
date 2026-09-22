//! The plugin of the worker pool example.
//!
//! It logs each request header, adds a header of its own, and puts the path of
//! the request on a shared queue that its root reads.
//! A request with the header `x-deny` receives its own answer, as a plugin that
//! authorizes requests does.
//! A request with the header `x-trap` makes it panic, so the example shows how
//! a host replaces a guest that failed.

use proxy_wasm::hostcalls;
use proxy_wasm::traits::{Context, HttpContext, RootContext};
use proxy_wasm::types::{Action, ContextType, LogLevel};

const QUEUE: &str = "paths";

proxy_wasm::main! {{
    proxy_wasm::set_log_level(LogLevel::Trace);
    proxy_wasm::set_root_context(|_| -> Box<dyn RootContext> { Box::new(Root { queue: 0 }) });
}}

fn log(level: LogLevel, line: &str) {
    let _ = hostcalls::log(level, line);
}

struct Root {
    queue: u32,
}

impl Context for Root {}

impl RootContext for Root {
    fn on_configure(&mut self, _plugin_configuration_size: usize) -> bool {
        self.queue = self.register_shared_queue(QUEUE);
        log(LogLevel::Info, &format!("registered queue {}", self.queue));
        true
    }

    fn on_queue_ready(&mut self, queue_id: u32) {
        match self.dequeue_shared_queue(queue_id) {
            Ok(Some(item)) => log(
                LogLevel::Info,
                &format!("path seen: {}", String::from_utf8_lossy(&item)),
            ),
            Ok(None) => log(LogLevel::Debug, "queue empty"),
            Err(status) => log(LogLevel::Warn, &format!("the queue refused: {status:?}")),
        }
    }

    fn create_http_context(&self, context_id: u32) -> Option<Box<dyn HttpContext>> {
        Some(Box::new(Request {
            context_id,
            queue: self.queue,
        }))
    }

    fn get_type(&self) -> Option<ContextType> {
        Some(ContextType::HttpContext)
    }
}

struct Request {
    context_id: u32,
    queue: u32,
}

impl Context for Request {}

impl HttpContext for Request {
    fn on_http_request_headers(&mut self, _num_headers: usize, _end_of_stream: bool) -> Action {
        for (name, value) in self.get_http_request_headers() {
            log(LogLevel::Info, &format!("{name} -> {value}"));
        }
        if self.get_http_request_header("x-deny").is_some() {
            self.send_http_response(403, vec![("x-denied", "yes")], Some(b"denied"));
            log(LogLevel::Info, "local response 403");
            return Action::Pause;
        }
        if self.get_http_request_header("x-trap").is_some() {
            log(LogLevel::Error, "trap requested");
            panic!("the example plugin trapped on request");
        }
        self.set_http_request_header("x-proxy-wasm", Some(&self.context_id.to_string()));
        match self.get_http_request_header(":path") {
            Some(path) => {
                let _ = self.enqueue_shared_queue(self.queue, Some(path.as_bytes()));
                log(LogLevel::Info, &format!("enqueued {path}"));
            }
            None => log(LogLevel::Warn, "no path"),
        }
        Action::Continue
    }
}
