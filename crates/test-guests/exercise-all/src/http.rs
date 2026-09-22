//! The HTTP context, which reaches every family a request can use.

use std::time::Duration;

use proxy_wasm::hostcalls;
use proxy_wasm::traits::{Context, HttpContext};
use proxy_wasm::types::{Action, BufferType};

use crate::root::DEFERRED;
use crate::{info, text};

pub(crate) struct Http {
    context_id: u32,
    queue: u32,
    deferred: bool,
}

impl Http {
    pub(crate) fn new(context_id: u32, queue: u32) -> Self {
        Self {
            context_id,
            queue,
            deferred: false,
        }
    }
}

impl Context for Http {
    fn on_http_call_response(
        &mut self,
        _token_id: u32,
        num_headers: usize,
        body_size: usize,
        num_trailers: usize,
    ) {
        let headers = self.get_http_call_response_headers();
        let body = text(self.get_http_call_response_body(0, body_size));
        let trailers = self.get_http_call_response_trailers();
        info(&format!(
            "http_response headers={num_headers}:{} body={body} trailers={num_trailers}:{}",
            headers.len(),
            trailers.len()
        ));
        self.resume_http_request();
    }

    fn on_foreign_function(&mut self, function_id: u32, arguments_size: usize) {
        let arguments = hostcalls::get_buffer(BufferType::CallData, 0, arguments_size).unwrap();
        info(&format!(
            "foreign_function id={function_id} arguments={}",
            text(arguments)
        ));
    }

    fn on_done(&mut self) -> bool {
        info(&format!("done deferred={}", self.deferred));
        if self.deferred {
            DEFERRED.with_borrow_mut(|deferred| deferred.push(self.context_id));
        }
        !self.deferred
    }
}

impl HttpContext for Http {
    fn on_http_request_headers(&mut self, num_headers: usize, _end_of_stream: bool) -> Action {
        let count = self.get_http_request_headers().len();
        let path = text(self.get_property(vec!["request", "path"]));
        info(&format!(
            "request_headers count={num_headers}:{count} path={path}"
        ));
        match self.get_http_request_header("x-exercise").as_deref() {
            Some("panic") => panic!("exercise panic in request headers"),
            Some("local") => {
                self.send_http_response(403, vec![("x-local", "yes")], Some(b"denied"));
                info("local_response status=403");
                return Action::Pause;
            }
            Some("defer") => self.deferred = true,
            _ => {}
        }
        self.add_http_request_header("x-added", "1");
        self.set_http_request_header("x-replaced", Some("2"));
        self.set_http_request_header("x-removed", None);
        self.set_property(vec!["exercise", "seen"], Some(b"yes"));
        let (value, cas) = self.get_shared_data("exercise");
        self.set_shared_data("exercise", Some(b"seen"), cas)
            .unwrap();
        info(&format!(
            "shared_data value={} cas={}",
            text(value),
            cas.unwrap_or(0)
        ));
        let item = format!("from {}", self.context_id);
        self.enqueue_shared_queue(self.queue, Some(item.as_bytes()))
            .unwrap();
        info(&format!("enqueue queue={}", self.queue));
        let headers = vec![
            (":method", "GET"),
            (":path", "/"),
            (":authority", "upstream"),
        ];
        let call = self
            .dispatch_http_call("upstream", headers, None, vec![], Duration::from_secs(1))
            .unwrap();
        info(&format!("http_call id={call}"));
        Action::Pause
    }

    fn on_http_request_body(&mut self, body_size: usize, end_of_stream: bool) -> Action {
        let body = text(self.get_http_request_body(0, body_size));
        self.set_http_request_body(0, body_size, b"replaced");
        info(&format!(
            "request_body size={body_size} end={end_of_stream} body={body}"
        ));
        Action::Continue
    }

    fn on_http_request_trailers(&mut self, num_trailers: usize) -> Action {
        self.set_http_request_trailers(vec![("x-trailer", "set")]);
        info(&format!("request_trailers count={num_trailers}"));
        Action::Continue
    }

    fn on_http_response_headers(&mut self, num_headers: usize, _end_of_stream: bool) -> Action {
        let count = self.get_http_response_headers().len();
        info(&format!("response_headers count={num_headers}:{count}"));
        let answer = self
            .call_foreign_function("exercise_echo", Some(b"ping"))
            .unwrap();
        info(&format!("foreign_call answer={}", text(answer)));
        Action::Continue
    }

    fn on_http_response_body(&mut self, body_size: usize, end_of_stream: bool) -> Action {
        let body = text(self.get_http_response_body(0, body_size));
        info(&format!(
            "response_body size={body_size} end={end_of_stream} body={body}"
        ));
        Action::Continue
    }

    fn on_http_response_trailers(&mut self, num_trailers: usize) -> Action {
        let count = self.get_http_response_trailers().len();
        info(&format!("response_trailers count={num_trailers}:{count}"));
        Action::Continue
    }

    fn on_log(&mut self) {
        info(&format!("log context={}", self.context_id));
    }
}
