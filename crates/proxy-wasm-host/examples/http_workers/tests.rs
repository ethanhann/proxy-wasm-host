//! The tests of the worker of the `http_workers` example.

use std::io::Read;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use proxy_wasm_host::abi::v0_2_1::{Host, InMemoryStore, VmServices};
use proxy_wasm_host::{Engine, Limits, Module};
use tiny_http::{Header, TestRequest};

use crate::routes::observer;

use crate::request::{HttpRequest, TracingSink, request_state};

use super::*;

const GUEST: &[u8] = include_bytes!("../../tests/fixtures/http-example.wasm");

/// The authority of a server on the default port.
const AUTHORITY: &str = "127.0.0.1:2045";

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
            Worker::start(index, &spec, plugin.clone(), &routes, AUTHORITY)
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
    request_state(&test_request(headers), AUTHORITY)
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
fn a_baseline_answer_lists_the_request_as_it_arrived() {
    // Arrange
    let source = test_request(&[("x-trace", "7")]);

    // Act
    let answer = baseline_answer(&source, AUTHORITY);

    // Assert
    let mut body = String::new();
    answer.into_reader().read_to_string(&mut body).unwrap();
    assert_eq!(
        body,
        ":method: GET\n:path: /example\n:authority: 127.0.0.1:2045\nx-trace: 7\ncontent-length: 0\n",
        "a test request carries a content-length of its own"
    );
}

#[test]
fn a_baseline_worker_takes_every_request_and_stops_when_the_channel_closes() {
    // Arrange
    let (sender, receiver) = channel();
    sender.send(Job::Request(test_request(&[]))).unwrap();
    sender.send(Job::Request(test_request(&[]))).unwrap();
    drop(sender);

    // Act
    serve_baseline(&receiver, AUTHORITY);

    // Assert
    assert_eq!(
        receiver.try_recv().map(|_| ()),
        Err(std::sync::mpsc::TryRecvError::Disconnected),
        "the worker took both requests and returned when the channel closed"
    );
}

#[test]
fn a_missing_plugin_file_is_reported_with_its_path() {
    // Arrange
    let path = std::path::Path::new("/no/such/plugin.wasm");

    // Act
    let result = crate::start_workers(&crate::options::Options::default(), path, AUTHORITY);

    // Assert
    let message = result.map(|_| ()).unwrap_err().to_string();
    assert!(message.contains("/no/such/plugin.wasm"), "{message}");
}

#[test]
fn a_server_that_stops_accepting_ends_the_dispatch_with_an_error() {
    // Arrange
    let (sender, _receiver) = channel();
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    server.unblock();

    // Act
    let result = crate::dispatch(&server, &[sender]);

    // Assert
    let error = result.expect_err("a server that stops must not end the example quietly");
    assert!(!error.to_string().is_empty());
}
