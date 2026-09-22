//! One worker: a guest, the jobs it runs, and the guest it builds again.

use std::io::Cursor;
use std::sync::mpsc::Receiver;

use proxy_wasm_host::HeaderMap;
use proxy_wasm_host::abi::v0_2_1::{
    Callback, ContextId, Guest, GuestError, GuestId, GuestSpec, PluginConfig, QueueId, Started,
    StreamKind,
};
use tiny_http::{Request, Response};

use crate::request::{HttpRequest, answer_of, request_state};
use crate::routes::QueueRoutes;

/// What a worker receives.
pub enum Job {
    /// One request to serve.
    Request(Request),
    /// One item that waits on a queue of this worker.
    QueueReady { queue: QueueId, root: ContextId },
}

/// One thread with one guest.
pub struct Worker {
    index: usize,
    spec: GuestSpec,
    plugin: PluginConfig,
    routes: QueueRoutes,
    guest: Option<(Guest, ContextId)>,
}

/// Why a worker has no guest.
pub enum Failure {
    /// The build failed, and the worker tries again on its next job.
    Build(GuestError),
    /// The plugin refused its start, so the worker stops.
    Refused(Callback),
}

impl Worker {
    /// Builds the guest of this worker and records its registrations.
    ///
    /// # Errors
    ///
    /// Returns the failure of the build or the callback that refused.
    pub fn start(
        index: usize,
        spec: &GuestSpec,
        plugin: PluginConfig,
        routes: &QueueRoutes,
    ) -> Result<Self, Failure> {
        let mut worker = Self {
            index,
            spec: spec.clone(),
            plugin,
            routes: routes.clone(),
            guest: None,
        };
        worker.rebuild()?;
        Ok(worker)
    }

    /// Runs each job until the channel closes or the plugin refuses a start.
    pub fn run(&mut self, jobs: &Receiver<Job>) {
        while let Ok(job) = jobs.recv() {
            let span = tracing::info_span!("worker", index = self.index);
            let _entered = span.enter();
            self.handle(job);
            self.collect_registrations();
            if !self.serving() {
                match self.rebuild() {
                    Ok(()) => {}
                    Err(Failure::Build(error)) => {
                        tracing::error!("the build failed, and this worker waits: {error}");
                    }
                    Err(Failure::Refused(callback)) => {
                        tracing::error!("the plugin refused its start in {callback}");
                        self.routes.forget(self.index);
                        return;
                    }
                }
            }
        }
        self.routes.forget(self.index);
    }

    fn handle(&mut self, job: Job) {
        match job {
            Job::Request(request) => {
                let state = request_state(&request);
                let answer = self.serve(state);
                if let Err(error) = request.respond(answer) {
                    tracing::warn!("the answer did not reach the client: {error}");
                }
            }
            Job::QueueReady { queue, root } => {
                let Some((guest, _)) = self.guest.as_mut() else {
                    tracing::warn!("queue {queue:?} waits for a guest");
                    return;
                };
                if let Err(error) = guest.enter_root().on_queue_ready(root, queue) {
                    tracing::error!("the queue callback failed: {error}");
                }
            }
        }
    }

    /// Runs one request and builds its answer.
    ///
    /// This example answers the failure here, because the worker owns the guest
    /// that it builds again.
    pub fn serve(&mut self, state: HttpRequest) -> Response<Cursor<Vec<u8>>> {
        let Some((guest, root)) = self.guest.as_mut() else {
            return Response::from_string("no guest serves this worker").with_status_code(503);
        };
        let root = *root;
        let (answer, state) = guest.with(state, |scope| {
            let stream = scope.on_context_create(Some(root))?;
            scope.expect_stream_kind(stream, StreamKind::Http)?;
            let count = u32::try_from(scope.stream().headers.len()).unwrap_or(u32::MAX);
            let action = scope.on_request_headers(stream, count, true)?;
            if !scope.on_done(stream)? {
                tracing::info!("the guest holds the context, and the example deletes it anyway");
            }
            scope.on_log(stream)?;
            scope.on_delete(stream)?;
            Ok::<_, GuestError>(action)
        });
        match answer {
            Ok(action) => answer_of(&state, action),
            Err(error) => {
                tracing::error!("the guest failed: {error}");
                Response::from_string("the guest failed").with_status_code(500)
            }
        }
    }

    /// Whether this worker has a guest that serves.
    pub fn serving(&self) -> bool {
        self.guest
            .as_ref()
            .is_some_and(|(guest, _)| guest.is_serving())
    }

    /// The guest of this worker, which changes with every rebuild.
    pub fn guest_id(&self) -> Option<GuestId> {
        self.guest.as_ref().map(|(guest, _)| guest.id())
    }

    /// Writes every new registration of the guest into the routes.
    fn collect_registrations(&mut self) {
        let Some((guest, _)) = self.guest.as_mut() else {
            return;
        };
        let changes = guest.take_changes();
        for registration in changes.queues {
            self.routes
                .register(registration.queue, self.index, registration.root);
        }
    }

    /// Builds a new guest, starts its root, and records its registrations.
    fn rebuild(&mut self) -> Result<(), Failure> {
        self.guest = None;
        let mut guest = self.spec.build().map_err(Failure::Build)?;
        match guest.start(self.plugin.clone()).map_err(Failure::Build)? {
            Started::Serving(root) => {
                self.guest = Some((guest, root));
                self.collect_registrations();
                tracing::info!(
                    "worker {} serves with a new guest {:?}",
                    self.index,
                    self.guest_id()
                );
                Ok(())
            }
            Started::Refused { callback, .. } => Err(Failure::Refused(callback)),
        }
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
