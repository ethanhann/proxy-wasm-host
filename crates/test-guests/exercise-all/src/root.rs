//! The root context, which starts the VM, holds the shared resources, and
//! receives the ticks, the queue items, and the gRPC callouts.

use std::cell::RefCell;
use std::collections::hash_map::RandomState;
use std::hash::BuildHasher;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use proxy_wasm::hostcalls;
use proxy_wasm::traits::{Context, HttpContext, RootContext, StreamContext};
use proxy_wasm::types::{ContextType, MetricType};

use crate::{http, info, pairs, tcp, text};

thread_local! {
    /// The stream contexts whose done callback answered false.
    pub(crate) static DEFERRED: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
}

const TIMEOUT: Duration = Duration::from_secs(1);

pub(crate) struct Root {
    family: Option<ContextType>,
    queue: u32,
    counter: u32,
    gauge: u32,
}

impl Root {
    pub(crate) fn new(_context_id: u32) -> Self {
        Self {
            family: None,
            queue: 0,
            counter: 0,
            gauge: 0,
        }
    }

    fn nanoseconds(time: SystemTime) -> u128 {
        time.duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    }

    fn open_http_resources(&mut self) {
        let vm_id = text(self.get_property(vec!["plugin_vm_id"]));
        self.queue = self.register_shared_queue("exercise");
        let resolved = self.resolve_shared_queue(&vm_id, "exercise").unwrap_or(0);
        info(&format!(
            "queue registered={} resolved={resolved}",
            self.queue
        ));
        self.counter = hostcalls::define_metric(MetricType::Counter, "exercise_requests").unwrap();
        self.gauge = hostcalls::define_metric(MetricType::Gauge, "exercise_gauge").unwrap();
        info(&format!(
            "metrics counter={} gauge={}",
            self.counter, self.gauge
        ));
        self.set_tick_period(Duration::from_millis(100));
        let metadata = vec![("k", b"v".as_slice())];
        let call = self
            .dispatch_grpc_call(
                "grpc",
                "exercise.Echo",
                "Say",
                metadata.clone(),
                Some(b"hello"),
                TIMEOUT,
            )
            .unwrap();
        info(&format!("grpc_call id={call}"));
        let stream = self
            .open_grpc_stream("grpc", "exercise.Echo", "Chat", metadata)
            .unwrap();
        info(&format!("grpc_stream id={stream}"));
        self.send_grpc_stream_message(stream, Some(b"first"), false);
        info(&format!("grpc_send id={stream}"));
    }
}

impl Context for Root {
    fn on_grpc_call_response(&mut self, token_id: u32, _status_code: u32, response_size: usize) {
        let body = text(self.get_grpc_call_response_body(0, response_size));
        let (status, message) = self.get_grpc_status();
        let message = message.unwrap_or_default();
        info(&format!(
            "grpc_response id={token_id} status={status} message={message} body={body}"
        ));
    }

    fn on_grpc_stream_initial_metadata(&mut self, token_id: u32, _num_elements: u32) {
        let metadata = pairs(&self.get_grpc_stream_initial_metadata());
        info(&format!("grpc_initial id={token_id} pairs={metadata}"));
        self.close_grpc_stream(token_id);
        info(&format!("grpc_close id={token_id}"));
    }

    fn on_grpc_stream_message(&mut self, token_id: u32, message_size: usize) {
        let body = text(self.get_grpc_stream_message(0, message_size));
        info(&format!("grpc_message id={token_id} body={body}"));
    }

    fn on_grpc_stream_trailing_metadata(&mut self, token_id: u32, _num_elements: u32) {
        let metadata = pairs(&self.get_grpc_stream_trailing_metadata());
        info(&format!("grpc_trailing id={token_id} pairs={metadata}"));
    }

    fn on_grpc_stream_close(&mut self, token_id: u32, status_code: u32) {
        let (status, message) = self.get_grpc_status();
        let message = message.unwrap_or_default();
        info(&format!(
            "grpc_stream_close id={token_id} code={status_code} status={status} message={message}"
        ));
    }
}

impl RootContext for Root {
    fn on_vm_start(&mut self, _vm_configuration_size: usize) -> bool {
        let configuration = text(self.get_vm_configuration());
        info(&format!("vm_start configuration={configuration}"));
        let variable = std::env::var("EXERCISE").unwrap_or_default();
        info(&format!("environment EXERCISE={variable}"));
        info(&format!("arguments count={}", std::env::args().count()));
        let host = Self::nanoseconds(self.get_current_time());
        let wasi = Self::nanoseconds(SystemTime::now());
        info(&format!("clock host={host} wasi={wasi}"));
        let hash = RandomState::new().hash_one(0_u8);
        info(&format!("random hash={hash}"));
        let level = hostcalls::get_log_level().unwrap();
        info(&format!("log_level level={}", level as u32));
        println!("stdout ok");
        configuration != "refuse"
    }

    fn on_configure(&mut self, _plugin_configuration_size: usize) -> bool {
        let root_id = text(self.get_property(vec!["plugin_root_id"]));
        let configuration = text(self.get_plugin_configuration());
        info(&format!(
            "configure root_id={root_id} configuration={configuration}"
        ));
        self.family = match root_id.as_str() {
            "http" => Some(ContextType::HttpContext),
            "tcp" => Some(ContextType::StreamContext),
            _ => None,
        };
        if self.family == Some(ContextType::HttpContext) {
            self.open_http_resources();
        }
        if configuration == "panic" {
            panic!("exercise panic in configure");
        }
        configuration != "refuse"
    }

    fn on_tick(&mut self) {
        hostcalls::increment_metric(self.counter, 1).unwrap();
        hostcalls::record_metric(self.gauge, 7).unwrap();
        let counter = hostcalls::get_metric(self.counter).unwrap();
        let gauge = hostcalls::get_metric(self.gauge).unwrap();
        info(&format!("tick counter={counter} gauge={gauge}"));
        let call = self
            .dispatch_grpc_call("grpc", "exercise.Echo", "Say", vec![], None, TIMEOUT)
            .unwrap();
        self.cancel_grpc_call(call);
        info(&format!("grpc_cancel id={call}"));
        for context in DEFERRED.take() {
            hostcalls::set_effective_context(context).unwrap();
            hostcalls::done().unwrap();
            info(&format!("done context={context}"));
        }
    }

    fn on_queue_ready(&mut self, queue_id: u32) {
        let item = text(self.dequeue_shared_queue(queue_id).unwrap());
        info(&format!("queue_ready queue={queue_id} item={item}"));
    }

    fn create_http_context(&self, context_id: u32) -> Option<Box<dyn HttpContext>> {
        Some(Box::new(http::Http::new(context_id, self.queue)))
    }

    fn create_stream_context(&self, context_id: u32) -> Option<Box<dyn StreamContext>> {
        Some(Box::new(tcp::Tcp::new(context_id)))
    }

    fn get_type(&self) -> Option<ContextType> {
        self.family
    }
}
