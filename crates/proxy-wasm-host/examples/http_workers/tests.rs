//! The tests of the worker of the `http_workers` example.

use std::io::Write;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, PoisonError};

use proxy_wasm_host::abi::v0_2_1::types::LogLevel;
use proxy_wasm_host::abi::v0_2_1::{GuestId, Host, InMemoryStore, LogContext, LogSink, VmServices};
use proxy_wasm_host::{Engine, Limits, Module};
use tiny_http::{Header, TestRequest};

use crate::routes::observer;

use crate::request::{HttpRequest, TracingSink, pairs, request_state};

use super::*;

const GUEST: &[u8] = include_bytes!("../../tests/fixtures/http-example.wasm");

/// A pool of `count` workers on one store, in the order they started.
struct Pool {
    workers: Vec<Worker>,
    receivers: Vec<Receiver<Job>>,
}

fn pool(count: usize) -> Pool {
    let mut senders: Vec<Sender<Job>> = Vec::new();
    let mut receivers = Vec::new();
    for _ in 0..count {
        let (sender, receiver) = channel();
        senders.push(sender);
        receivers.push(receiver);
    }
    let routes = QueueRoutes::default();
    let store = InMemoryStore::new().with_enqueue_observer(observer(routes.clone(), senders));
    let engine = Engine::new().unwrap();
    let module = Module::new(&engine, GUEST).unwrap();
    let services = VmServices::new(Arc::new(TracingSink))
        .with_vm_id(*b"example")
        .with_shared(Arc::new(store));
    let spec = GuestSpec::new(
        &Host::new(&engine).unwrap(),
        &module,
        services,
        &Limits::default(),
    )
    .unwrap();
    let plugin = PluginConfig::new().with_name(*b"example");
    let workers = (0..count)
        .map(|index| {
            Worker::start(index, &spec, plugin.clone(), &routes)
                .ok()
                .unwrap()
        })
        .collect();
    Pool { workers, receivers }
}

fn test_request(headers: &[(&str, &str)]) -> Request {
    let mut test = TestRequest::new().with_path("/example");
    for (name, value) in headers {
        test = test.with_header(format!("{name}: {value}").parse::<Header>().unwrap());
    }
    Request::from(test)
}

fn request(headers: &[(&str, &str)]) -> HttpRequest {
    request_state(&test_request(headers))
}

#[test]
fn an_item_wakes_the_worker_that_registered_last() {
    // Arrange
    let mut pool = pool(2);
    let state = request(&[]);

    // Act
    let answer = pool.workers[0].serve(state);

    // Assert
    assert_eq!(answer.status_code().0, 200);
    assert!(pool.receivers[0].try_recv().is_err());
    let job = pool.receivers[1].try_recv();
    assert!(
        matches!(job, Ok(Job::QueueReady { queue, .. }) if queue.get() == 1),
        "the last registrant takes the item"
    );
}

#[test]
fn a_worker_that_traps_answers_500() {
    // Arrange
    let mut pool = pool(1);
    let trapping = request(&[("x-trap", "1")]);

    // Act
    let answer = pool.workers[0].serve(trapping);

    // Assert
    assert_eq!(answer.status_code().0, 500);
    assert!(!pool.workers[0].serving());
}

#[test]
fn a_rebuilt_worker_serves_the_next_request() {
    // Arrange
    let mut pool = pool(1);
    let trapping = request(&[("x-trap", "1")]);
    let next = request(&[]);
    pool.workers[0].serve(trapping);
    pool.workers[0].rebuild().ok().unwrap();

    // Act
    let answer = pool.workers[0].serve(next);

    // Assert
    assert_eq!(answer.status_code().0, 200);
    assert!(pool.workers[0].serving());
}

#[test]
fn the_loop_rebuilds_a_guest_that_trapped() {
    // Arrange
    let mut pool = pool(1);
    let (sender, receiver) = channel();
    let before = pool.workers[0].guest_id();
    sender
        .send(Job::Request(test_request(&[("x-trap", "1")])))
        .unwrap();
    sender.send(Job::Request(test_request(&[]))).unwrap();
    drop(sender);

    // Act
    pool.workers[0].run(&receiver);

    // Assert
    assert!(pool.workers[0].serving());
    assert_ne!(pool.workers[0].guest_id(), before);
}

#[test]
fn a_worker_with_no_guest_answers_503() {
    // Arrange
    let mut pool = pool(1);
    pool.workers[0].guest = None;
    let state = request(&[]);

    // Act
    let answer = pool.workers[0].serve(state);

    // Assert
    assert_eq!(answer.status_code().0, 503);
    assert!(!pool.workers[0].serving());
}

#[test]
fn a_poisoned_guest_takes_a_queue_job_without_a_panic() {
    // Arrange
    let mut pool = pool(1);
    let trapping = request(&[("x-trap", "1")]);
    pool.workers[0].serve(trapping);
    let job = Job::QueueReady {
        queue: QueueId::try_from(1).unwrap(),
        root: ContextId::try_from(1).unwrap(),
    };

    // Act
    pool.workers[0].handle(job);

    // Assert
    assert!(!pool.workers[0].serving());
}

#[test]
fn a_denied_request_gets_the_answer_of_the_plugin() {
    // Arrange
    let mut pool = pool(1);
    let denied = request(&[("x-deny", "1")]);

    // Act
    let answer = pool.workers[0].serve(denied);

    // Assert
    assert_eq!(answer.status_code().0, 403);
    assert!(pool.workers[0].serving());
}

#[test]
fn the_answer_carries_no_length_or_type_of_the_request() {
    // Arrange
    let mut pool = pool(1);
    let state = request(&[("content-type", "application/json")]);

    // Act
    let answer = pool.workers[0].serve(state);

    // Assert
    let headers: Vec<(String, String)> = answer
        .headers()
        .iter()
        .map(|header| {
            (
                header.field.as_str().as_str().to_lowercase(),
                header.value.as_str().to_owned(),
            )
        })
        .collect();
    let content_type = headers.iter().find(|(name, _)| name == "content-type");
    assert_eq!(
        content_type.map(|(_, value)| value.as_str()),
        Some("text/plain; charset=UTF-8"),
        "the answer keeps its own type, and not the type of the request"
    );
    let names: Vec<&String> = headers.iter().map(|(name, _)| name).collect();
    assert!(
        !names.iter().any(|name| *name == "content-length"),
        "{names:?}"
    );
    assert!(
        names.iter().any(|name| *name == "x-proxy-wasm"),
        "{names:?}"
    );
}

#[test]
fn the_request_state_holds_the_three_pseudo_headers() {
    // Arrange
    let source = test_request(&[]);

    // Act
    let state = request_state(&source);

    // Assert
    let names: Vec<String> = pairs(&state.headers)
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    assert_eq!(names[..3], [":method", ":path", ":authority"]);
    assert_eq!(pairs(&state.headers)[1].1, "/example");
}

#[test]
fn the_sink_maps_each_level_to_its_tracing_level() {
    // Arrange
    let lines = Lines::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(lines.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .finish();
    let context = LogContext::new(b"vm", GuestId::next());
    let levels = [
        LogLevel::Trace,
        LogLevel::Debug,
        LogLevel::Info,
        LogLevel::Warn,
        LogLevel::Error,
        LogLevel::Critical,
    ];

    // Act
    tracing::subscriber::with_default(subscriber, || {
        for level in levels {
            TracingSink.log(context.clone(), level, b"line");
        }
    });

    // Assert
    let text = lines.text();
    let seen: Vec<&str> = text
        .lines()
        .filter_map(|line| line.split_whitespace().nth(1))
        .collect();
    assert_eq!(seen, ["TRACE", "DEBUG", "INFO", "WARN", "ERROR", "ERROR"]);
    assert_eq!(text.matches("guest: line").count(), 6, "{text}");
}

/// A writer that keeps every line a subscriber writes.
#[derive(Clone, Default)]
struct Lines(Arc<Mutex<Vec<u8>>>);

impl Lines {
    fn text(&self) -> String {
        let bytes = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        String::from_utf8(bytes).unwrap()
    }
}

impl Write for Lines {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Lines {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}
