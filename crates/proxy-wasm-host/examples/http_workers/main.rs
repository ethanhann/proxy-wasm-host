//! A proxy with a pool of workers, one guest on each worker thread.
//!
//! It shows the two policies the crate leaves to you.
//! A queue item wakes the worker whose root registered the queue last, and a
//! worker that loses its guest builds a new one.
//! Send `curl -H 'x-trap: 1' http://127.0.0.1:2045/` to see the second policy.
//!
//! Run it with a plugin path, or with no path for the plugin that ships with
//! the example:
//!
//! ```text
//! http_workers [PATH | --wasm PATH | --no-wasm] [--workers N] [--port N] [--opt-level speed|speed-and-size]
//! ```
//!
//! `--workers` sets how many worker threads serve requests, and the default is
//! four. `--port` sets the port on 127.0.0.1, and the default is 2045.
//! `--opt-level speed-and-size` compiles the plugin into smaller code, and the
//! default is `speed`.
//!
//! `--no-wasm` runs no plugin at all. Each request still travels through the
//! dispatch to a worker thread, and the worker answers it as it arrived, so you
//! can measure the HTTP server and the dispatch on their own and subtract that
//! from a run with a plugin.
//!
//! `tiny_http` starts one thread for each open connection and holds two file
//! descriptors for each one. If you put the example under load, raise the open
//! file limit first with `ulimit -n`, or the server stops with "Too many open
//! files". If you compare it with a server that has a fixed pool of connection
//! threads, keep the number of connections at or below the size of that pool,
//! so both servers run one thread for each connection.

mod options;
mod request;
mod routes;
mod worker;

use std::error::Error;
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use proxy_wasm_host::abi::v0_2_1::{GuestSpec, Host, InMemoryStore, PluginConfig, VmServices};
use proxy_wasm_host::{EngineConfig, Limits, Module};
use tiny_http::Server;

use options::{Command, Options, Plugin, USAGE};
use request::TracingSink;
use routes::{QueueRoutes, observer};
use worker::{Failure, Job, Worker, serve_baseline};

const DEFAULT_GUEST: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/http-example.wasm"
);

type Outcome<T> = Result<T, Box<dyn Error + Send + Sync>>;

/// Why a worker could not start, for a reader of the terminal.
fn failure_message(index: usize, failure: &Failure) -> String {
    match failure {
        Failure::Build(error) => format!("worker {index} could not build its guest: {error}"),
        Failure::Refused(callback) => {
            format!("worker {index} has a plugin that refused its start in {callback}")
        }
    }
}

fn main() -> Outcome<()> {
    tracing_subscriber::fmt::init();
    let options = match options::parse(std::env::args().skip(1)) {
        Ok(Command::Run(options)) => options,
        Ok(Command::Help) => {
            println!("{USAGE}");
            return Ok(());
        }
        Err(error) => {
            eprintln!("{error}\n{USAGE}");
            std::process::exit(2);
        }
    };
    let authority = format!("127.0.0.1:{}", options.port);
    let (senders, plugin) = match &options.plugin {
        Plugin::None => (start_baseline(options.workers, &authority), "no plugin"),
        Plugin::Default => (
            start_workers(&options, Path::new(DEFAULT_GUEST), &authority)?,
            DEFAULT_GUEST,
        ),
        Plugin::Path(path) => (
            start_workers(&options, path, &authority)?,
            path.to_str().unwrap_or("a plugin"),
        ),
    };
    let server = Server::http(&authority)?;
    tracing::info!(
        "listening on {authority} with {} workers and {plugin}",
        options.workers
    );
    dispatch(&server, &senders)
}

/// One channel for each worker.
fn channels(count: usize) -> (Vec<Sender<Job>>, Vec<Receiver<Job>>) {
    (0..count).map(|_| channel()).unzip()
}

/// Starts workers that run no guest.
fn start_baseline(count: usize, authority: &str) -> Vec<Sender<Job>> {
    let (senders, receivers) = channels(count);
    for receiver in receivers {
        let authority = authority.to_owned();
        std::thread::spawn(move || serve_baseline(&receiver, &authority));
    }
    senders
}

/// Starts one worker with one guest of the plugin at `path` for each worker
/// the options ask for.
fn start_workers(options: &Options, path: &Path, authority: &str) -> Outcome<Vec<Sender<Job>>> {
    let bytes = std::fs::read(path)?;

    // The channels and the routes come first, because the observer of the store
    // holds a sender of each worker and reads the routes.
    let (senders, receivers) = channels(options.workers);
    let routes = QueueRoutes::default();
    let store =
        InMemoryStore::new().with_enqueue_observer(observer(routes.clone(), senders.clone()));

    let engine = EngineConfig::new()
        .with_opt_level(options.opt_level)
        .build()?;
    let module = Module::new(&engine, &bytes)?;
    let services = VmServices::new(Arc::new(TracingSink))
        .with_vm_id(*b"example")
        .with_shared(Arc::new(store));
    let spec = GuestSpec::new(&Host::new(&engine)?, &module, services, &Limits::default())?;
    let plugin = PluginConfig::new().with_name(*b"example");

    // The guests start here, in order, so a failure reaches you before the
    // socket opens and so the last registrant is the same worker in every run.
    for (index, receiver) in receivers.into_iter().enumerate() {
        let mut worker = Worker::start(index, &spec, plugin.clone(), &routes, authority)
            .map_err(|failure| failure_message(index, &failure))?;
        std::thread::spawn(move || worker.run(&receiver));
    }
    Ok(senders)
}

/// Hands each request the server receives to the workers in turn.
///
/// `tiny_http` stops accepting connections after the first failed accept, so
/// this returns that error rather than letting the example exit quietly.
fn dispatch(server: &Server, senders: &[Sender<Job>]) -> Outcome<()> {
    for count in 0.. {
        let request = server.recv()?;
        let index = count % senders.len();
        tracing::info!("{} {} to worker {index}", request.method(), request.url());
        if senders[index].send(Job::Request(request)).is_err() {
            tracing::warn!("worker {index} has stopped");
        }
    }
    Ok(())
}
