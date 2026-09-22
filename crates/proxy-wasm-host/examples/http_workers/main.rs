//! A proxy with a pool of workers, one guest on each worker thread.
//!
//! It shows the two policies the crate leaves to you.
//! A queue item wakes the worker whose root registered the queue last, which is
//! the rule of the C++ host, and a worker that loses its guest builds a new one.
//! Send `curl -H 'x-trap: 1' http://127.0.0.1:2045/` to see the second policy.

mod request;
mod routes;
mod worker;

use std::sync::Arc;
use std::sync::mpsc::{Sender, channel};

use proxy_wasm_host::abi::v0_2_1::{GuestSpec, Host, InMemoryStore, PluginConfig, VmServices};
use proxy_wasm_host::{Engine, Limits, Module};
use tiny_http::Server;

use request::{ADDRESS, TracingSink};
use routes::{QueueRoutes, observer};
use worker::{Failure, Job, Worker};

const DEFAULT_GUEST: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/http-example.wasm"
);

const WORKERS: usize = 4;

/// Why a worker could not start, for a reader of the terminal.
fn failure_message(index: usize, failure: &Failure) -> String {
    match failure {
        Failure::Build(error) => format!("worker {index} could not build its guest: {error}"),
        Failure::Refused(callback) => {
            format!("worker {index} has a plugin that refused its start in {callback}")
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt::init();
    let path = std::env::args().nth(1).unwrap_or(DEFAULT_GUEST.to_owned());
    let bytes = std::fs::read(&path)?;

    // The channels and the routes come first, because the observer of the store
    // holds a sender of each worker and reads the routes.
    let mut senders: Vec<Sender<Job>> = Vec::new();
    let mut receivers = Vec::new();
    for _ in 0..WORKERS {
        let (sender, receiver) = channel();
        senders.push(sender);
        receivers.push(receiver);
    }
    let routes = QueueRoutes::default();
    let store =
        InMemoryStore::new().with_enqueue_observer(observer(routes.clone(), senders.clone()));

    let engine = Engine::new()?;
    let module = Module::new(&engine, &bytes)?;
    let services = VmServices::new(Arc::new(TracingSink))
        .with_vm_id(*b"example")
        .with_shared(Arc::new(store));
    let spec = GuestSpec::new(&Host::new(&engine)?, &module, services, &Limits::default())?;
    let plugin = PluginConfig::new().with_name(*b"example");

    // The guests start here, in order, so a failure reaches you before the
    // socket opens and so the last registrant is the same worker in every run.
    for (index, receiver) in receivers.into_iter().enumerate() {
        let mut worker = Worker::start(index, &spec, plugin.clone(), &routes)
            .map_err(|failure| failure_message(index, &failure))?;
        std::thread::spawn(move || worker.run(&receiver));
    }

    let server = Server::http(ADDRESS)?;
    tracing::info!("listening on {ADDRESS} with {WORKERS} workers and the plugin {path}");
    for (count, request) in server.incoming_requests().enumerate() {
        let index = count % WORKERS;
        tracing::info!("{} {} to worker {index}", request.method(), request.url());
        if senders[index].send(Job::Request(request)).is_err() {
            tracing::warn!("worker {index} has stopped");
        }
    }
    Ok(())
}
