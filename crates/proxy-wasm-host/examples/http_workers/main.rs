//! A proxy with a pool of workers, one guest on each worker thread.
//!
//! It shows the two policies the crate leaves to you.
//! A queue item wakes the worker whose root registered the queue last, and a
//! worker that loses its guest builds a new one.
//! Send `curl -H 'x-trap: 1' http://127.0.0.1:2045/` to see the second policy.
//!
//! With no arguments, it runs the plugin that ships with the example.
//! You can name another plugin with a path, or run with no plugin:
//!
//! ```text
//! http_workers [PATH | --wasm PATH | --no-wasm] [--workers N] [--port N]
//! ```
//!
//! `--workers` sets how many worker threads serve requests, and the default is
//! four. `--port` sets the port on 127.0.0.1, and the default is 2045. `--help`
//! prints the usage line. An option it cannot use prints the reason and the
//! usage line, and the process exits with status 2.
//!
//! The request headers follow the rules of a proxy. A name is stored in lower
//! case and compared without regard to case, and a header the guest replaces
//! moves to the end.
//!
//! # Measuring
//!
//! `--no-wasm` runs no plugin. Each request still goes through the dispatch to
//! a worker thread, and the worker answers it as it arrived. You can measure
//! the HTTP server and the dispatch on their own this way, and subtract that
//! from a run with a plugin.
//!
//! The example logs two lines for each request at the info level, so set
//! `RUST_LOG=error` before you measure, or the log becomes part of the cost.
//!
//! One thread accepts every request and hands it to a worker, and the worker
//! writes the answer to the socket. A server that answers on the thread of the
//! connection spends its time in other places, so compare the difference to
//! each server's own `--no-wasm` run rather than the raw numbers.
//!
//! `tiny_http` starts one thread for each open connection and holds two file
//! descriptors for each one. Before you put the example under load, raise the
//! open file limit with `ulimit -n`, or the server stops with "Too many open
//! files". If you compare it with a server that has a fixed pool of connection
//! threads, keep the number of connections at or below the size of that pool,
//! so both servers run one thread for each connection.

mod headers;
mod options;
mod request;
mod routes;
mod worker;

use std::error::Error;
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use proxy_wasm_host::abi::v0_2_1::{GuestSpec, Host, InMemoryStore, PluginConfig, VmServices};
use proxy_wasm_host::{Engine, Limits, Module};
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
        Plugin::None => (
            start_baseline(options.workers, &authority)?,
            "no plugin".to_owned(),
        ),
        Plugin::Default => (
            start_workers(&options, Path::new(DEFAULT_GUEST), &authority)?,
            format!("the plugin {DEFAULT_GUEST}"),
        ),
        Plugin::Path(path) => (
            start_workers(&options, path, &authority)?,
            format!("the plugin {}", path.display()),
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
fn start_baseline(count: usize, authority: &str) -> Outcome<Vec<Sender<Job>>> {
    let (senders, receivers) = channels(count);
    for (index, receiver) in receivers.into_iter().enumerate() {
        let authority = authority.to_owned();
        spawn(index, move || serve_baseline(&receiver, &authority))?;
    }
    Ok(senders)
}

/// Starts the thread of one worker, and reports a thread the system refuses
/// rather than panicking.
fn spawn(index: usize, work: impl FnOnce() + Send + 'static) -> Outcome<()> {
    std::thread::Builder::new()
        .name(format!("worker {index}"))
        .spawn(work)
        .map_err(|error| format!("worker {index} could not start its thread: {error}"))?;
    Ok(())
}

/// Starts one worker with one guest of the plugin at `path` for each worker
/// the options ask for.
fn start_workers(options: &Options, path: &Path, authority: &str) -> Outcome<Vec<Sender<Job>>> {
    let bytes = std::fs::read(path)
        .map_err(|error| format!("cannot read the plugin {}: {error}", path.display()))?;

    // The channels and the routes come first, because the observer of the store
    // holds a sender of each worker and reads the routes.
    let (senders, receivers) = channels(options.workers);
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
        let mut worker = Worker::start(index, &spec, plugin.clone(), &routes, authority)
            .map_err(|failure| failure_message(index, &failure))?;
        spawn(index, move || worker.run(&receiver))?;
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
