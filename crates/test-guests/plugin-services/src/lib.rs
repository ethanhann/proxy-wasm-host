//! A plugin that uses the services a host gives a plugin outside of one request.
//!
//! Each second its root logs a tick, and every tenth tick it calls the upstream `httpbin` and
//! logs the status of the response.
//! Each request adds one to the counter `plugin_requests_total` and puts its path on the shared
//! queue `paths`, and the root logs each path that it takes from the queue.
//! Each response gets the headers `x-route`, `x-client`, and `x-node` from the properties
//! `xds.route_name`, `source.address`, and `node.name`.

use proxy_wasm::hostcalls;
use proxy_wasm::traits::{Context, HttpContext, RootContext};
use proxy_wasm::types::{Action, ContextType, LogLevel, MetricType};
use std::time::Duration;

const QUEUE: &str = "paths";
const COUNTER: &str = "plugin_requests_total";
const TICK_PERIOD: Duration = Duration::from_secs(1);
const TICKS_BETWEEN_CALLOUTS: u64 = 10;
const CALLOUT_UPSTREAM: &str = "httpbin";
const CALLOUT_TIMEOUT: Duration = Duration::from_secs(5);

proxy_wasm::main! {{
    proxy_wasm::set_log_level(LogLevel::Info);
    proxy_wasm::set_root_context(|_| -> Box<dyn RootContext> {
        Box::new(Root { queue: 0, counter: 0, ticks: 0 })
    });
}}

fn log(level: LogLevel, line: &str) {
    let _ = hostcalls::log(level, line);
}

fn property_text(context: &impl Context, path: Vec<&str>) -> Option<String> {
    let bytes = context.get_property(path)?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

struct Root {
    queue: u32,
    counter: u32,
    ticks: u64,
}

impl Context for Root {
    fn on_http_call_response(&mut self, _token: u32, _headers: usize, _body: usize, _trailers: usize) {
        match self.get_http_call_response_header(":status") {
            Some(status) => log(LogLevel::Info, &format!("httpbin responded with {status}")),
            None => log(LogLevel::Warn, "the callout to httpbin failed"),
        }
    }
}

impl RootContext for Root {
    fn on_configure(&mut self, _plugin_configuration_size: usize) -> bool {
        self.queue = self.register_shared_queue(QUEUE);
        self.counter = match hostcalls::define_metric(MetricType::Counter, COUNTER) {
            Ok(counter) => counter,
            Err(status) => {
                log(LogLevel::Error, &format!("cannot define {COUNTER}: {status:?}"));
                return false;
            }
        };
        let node = property_text(self, vec!["node", "name"]);
        let node = node.as_deref().unwrap_or("an unnamed node");
        log(LogLevel::Info, &format!("configured on {node}"));
        self.set_tick_period(TICK_PERIOD);
        true
    }

    fn on_tick(&mut self) {
        self.ticks += 1;
        log(LogLevel::Info, &format!("tick {}", self.ticks));
        if self.ticks % TICKS_BETWEEN_CALLOUTS != 1 {
            return;
        }
        let headers = vec![
            (":method", "GET"),
            (":path", "/status/204"),
            (":authority", "httpbin.org"),
        ];
        match self.dispatch_http_call(CALLOUT_UPSTREAM, headers, None, vec![], CALLOUT_TIMEOUT) {
            Ok(_) => log(LogLevel::Info, &format!("tick {} called httpbin", self.ticks)),
            Err(status) => log(LogLevel::Warn, &format!("cannot call httpbin: {status:?}")),
        }
    }

    fn on_queue_ready(&mut self, queue_id: u32) {
        if let Ok(Some(path)) = self.dequeue_shared_queue(queue_id) {
            log(LogLevel::Info, &format!("path seen: {}", String::from_utf8_lossy(&path)));
        }
    }

    fn create_http_context(&self, _context_id: u32) -> Option<Box<dyn HttpContext>> {
        Some(Box::new(Request {
            queue: self.queue,
            counter: self.counter,
        }))
    }

    fn get_type(&self) -> Option<ContextType> {
        Some(ContextType::HttpContext)
    }
}

struct Request {
    queue: u32,
    counter: u32,
}

impl Context for Request {}

impl HttpContext for Request {
    fn on_http_request_headers(&mut self, _num_headers: usize, _end_of_stream: bool) -> Action {
        let _ = hostcalls::increment_metric(self.counter, 1);
        if let Some(path) = self.get_http_request_header(":path") {
            let _ = self.enqueue_shared_queue(self.queue, Some(path.as_bytes()));
        }
        Action::Continue
    }

    fn on_http_response_headers(&mut self, _num_headers: usize, _end_of_stream: bool) -> Action {
        let headers = [
            ("x-route", vec!["xds", "route_name"]),
            ("x-client", vec!["source", "address"]),
            ("x-node", vec!["node", "name"]),
        ];
        for (header, path) in headers {
            if let Some(value) = property_text(self, path) {
                self.add_http_response_header(header, &value);
            }
        }
        Action::Continue
    }
}
