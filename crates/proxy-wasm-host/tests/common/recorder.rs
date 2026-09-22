//! Implementations of the public service traits that write every call to one
//! list, so a test reads the log lines and the calls in the order they ran.
//!
//! Each call is recorded with the `Invocation` the crate gave the service.
//! A write to the store is recorded after the store accepted it, so an event
//! means that the call took effect.

use std::collections::BTreeMap;
use std::ops::ControlFlow;
use std::sync::{Arc, Mutex, PoisonError};

use proxy_wasm_host::abi::v0_2_1::types::{
    BufferType, LogLevel, MapType, MetricType, Status, StreamType,
};
use proxy_wasm_host::abi::v0_2_1::{
    Access, CalloutId, Callouts, ForeignCall, GrpcCall, GrpcOpenRefusal, GrpcStream, HttpCall,
    HttpCallRefusal, InMemoryStore, Invocation, LocalResponse, LogSink, MetricId, QueueEnqueued,
    QueueId, SharedServices, SharedValue, StreamState,
};
use proxy_wasm_host::{Buffer, HeaderMap, VecHeaderMap};

use crate::common::answers::{
    CalloutCall, CalloutRefusal, CalloutRefusals, StreamCall, StreamRefusals,
};

/// One thing that reached a service, in the order it happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Log(LogLevel, String),
    SetProperty(Vec<String>, String),
    ContinueStream(StreamType),
    CloseStream(StreamType),
    LocalResponse {
        status: u32,
        headers: Vec<(String, String)>,
        body: String,
        details: String,
        grpc_status: Option<u32>,
    },
    ForeignCall {
        name: String,
        arguments: String,
    },
    HttpCall {
        callout: CalloutId,
        upstream: String,
    },
    GrpcCall {
        callout: CalloutId,
        service: String,
        method: String,
        message: String,
    },
    GrpcStream {
        callout: CalloutId,
        service: String,
        method: String,
    },
    GrpcSend {
        callout: CalloutId,
        message: String,
        end_of_stream: bool,
    },
    GrpcCancel(CalloutId),
    GrpcClose(CalloutId),
    SetSharedData(String, String),
    RegisterQueue(String, QueueId),
    ResolveQueue(String, QueueId),
    Enqueue(QueueId, String),
    Dequeue(QueueId, String),
    DefineMetric(MetricType, String),
    IncrementMetric(MetricId, i64),
    RecordMetric(MetricId, u64),
    Enqueued(QueueId, String),
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// The pairs of a header map as text, in their order.
pub fn pairs(map: &dyn HeaderMap) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let _ = map.for_each_pair(&mut |key, value| {
        out.push((text(key), text(value)));
        ControlFlow::Continue(())
    });
    out
}

/// One event with the invocation the crate gave the service, which a log
/// line has none of.
pub type Recorded = (Option<Invocation>, Event);

/// The shared list of events, and the refusals a test asked for.
#[derive(Clone, Default)]
pub struct Recorder {
    events: Arc<Mutex<Vec<Recorded>>>,
    refusals: Arc<Mutex<CalloutRefusals>>,
}

impl Recorder {
    fn push(&self, at: Invocation, event: Event) {
        self.entries().push((Some(at), event));
    }

    fn entries(&self) -> std::sync::MutexGuard<'_, Vec<Recorded>> {
        self.events.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Makes `call` report `refusal` in place of accepting.
    ///
    /// The entry stays until [`Recorder::clear`], so every call of that
    /// method is refused.
    /// A method with no entry answers as it does without this call.
    pub fn refuse(&self, call: CalloutCall, refusal: CalloutRefusal) {
        self.refusals
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(call, refusal);
    }

    fn refusal(&self, call: CalloutCall) -> Option<CalloutRefusal> {
        self.refusals
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&call)
            .copied()
    }

    /// Every event so far, with its invocation.
    pub fn recorded(&self) -> Vec<Recorded> {
        self.entries().clone()
    }

    /// Every event so far.
    pub fn events(&self) -> Vec<Event> {
        self.recorded()
            .into_iter()
            .map(|(_, event)| event)
            .collect()
    }

    /// The text of every log line at `level`, in order.
    pub fn logs_at(&self, level: LogLevel) -> Vec<String> {
        self.events()
            .into_iter()
            .filter_map(|event| match event {
                Event::Log(at, line) if at == level => Some(line),
                _ => None,
            })
            .collect()
    }

    /// The text of every info line, in order.
    pub fn logs(&self) -> Vec<String> {
        self.logs_at(LogLevel::Info)
    }

    /// Every event that is not a log line, in order.
    pub fn calls(&self) -> Vec<Event> {
        self.events()
            .into_iter()
            .filter(|event| !matches!(event, Event::Log(..)))
            .collect()
    }

    /// Empties the list and the refusals, so a test sees only what its Act
    /// section causes and every method answers as it does by default.
    pub fn clear(&self) {
        self.entries().clear();
        self.refusals
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }

    /// The shared services, forwarding to an in-memory store that reports
    /// each enqueue into the same list.
    pub fn shared(&self) -> RecordingShared {
        let recorder = self.clone();
        let observer = Arc::new(move |item: QueueEnqueued<'_>| {
            let event = Event::Enqueued(item.queue, text(item.name));
            recorder.entries().push((None, event));
        });
        RecordingShared {
            store: InMemoryStore::new().with_enqueue_observer(observer),
            recorder: self.clone(),
        }
    }
}

impl LogSink for Recorder {
    fn log(&self, level: LogLevel, message: &[u8]) {
        self.entries()
            .push((None, Event::Log(level, text(message))));
    }
}

impl Callouts for Recorder {
    fn http_call(
        &self,
        at: Invocation,
        callout: CalloutId,
        request: HttpCall<'_>,
    ) -> Result<(), HttpCallRefusal> {
        match self.refusal(CalloutCall::HttpCall) {
            Some(CalloutRefusal::Http(refusal)) => return Err(refusal),
            Some(other) => panic!("http_call cannot answer {other:?}"),
            None => {}
        }
        let upstream = text(&request.upstream);
        self.push(at, Event::HttpCall { callout, upstream });
        Ok(())
    }

    fn grpc_call(
        &self,
        at: Invocation,
        callout: CalloutId,
        request: GrpcCall<'_>,
    ) -> Result<(), GrpcOpenRefusal> {
        match self.refusal(CalloutCall::GrpcCall) {
            Some(CalloutRefusal::GrpcOpen(refusal)) => return Err(refusal),
            Some(other) => panic!("grpc_call cannot answer {other:?}"),
            None => {}
        }
        let service = text(&request.service);
        let method = text(&request.method);
        let message = text(&request.message);
        let event = Event::GrpcCall {
            callout,
            service,
            method,
            message,
        };
        self.push(at, event);
        Ok(())
    }

    fn grpc_stream(
        &self,
        at: Invocation,
        callout: CalloutId,
        request: GrpcStream<'_>,
    ) -> Result<(), GrpcOpenRefusal> {
        match self.refusal(CalloutCall::GrpcStream) {
            Some(CalloutRefusal::GrpcOpen(refusal)) => return Err(refusal),
            Some(other) => panic!("grpc_stream cannot answer {other:?}"),
            None => {}
        }
        let service = text(&request.service);
        let method = text(&request.method);
        self.push(
            at,
            Event::GrpcStream {
                callout,
                service,
                method,
            },
        );
        Ok(())
    }

    fn grpc_send(&self, at: Invocation, callout: CalloutId, message: &[u8], end_of_stream: bool) {
        let message = text(message);
        let event = Event::GrpcSend {
            callout,
            message,
            end_of_stream,
        };
        self.push(at, event);
    }

    fn grpc_cancel(&self, at: Invocation, callout: CalloutId) {
        self.push(at, Event::GrpcCancel(callout));
    }

    fn grpc_close(&self, at: Invocation, callout: CalloutId) {
        self.push(at, Event::GrpcClose(callout));
    }
}

/// Shared services that forward to an [`InMemoryStore`] and record each call
/// that the store accepted and that changes it.
pub struct RecordingShared {
    store: InMemoryStore,
    recorder: Recorder,
}

impl SharedServices for RecordingShared {
    fn get_shared_data(
        &self,
        at: Invocation,
        vm_id: &[u8],
        key: &[u8],
    ) -> Result<SharedValue, Status> {
        self.store.get_shared_data(at, vm_id, key)
    }

    fn set_shared_data(
        &self,
        at: Invocation,
        vm_id: &[u8],
        key: &[u8],
        value: &[u8],
        cas: Option<u32>,
    ) -> Result<(), Status> {
        self.store.set_shared_data(at, vm_id, key, value, cas)?;
        let event = Event::SetSharedData(text(key), text(value));
        self.recorder.push(at, event);
        Ok(())
    }

    fn register_shared_queue(
        &self,
        at: Invocation,
        vm_id: &[u8],
        name: &[u8],
    ) -> Result<QueueId, Status> {
        let queue = self.store.register_shared_queue(at, vm_id, name)?;
        self.recorder
            .push(at, Event::RegisterQueue(text(name), queue));
        Ok(queue)
    }

    fn resolve_shared_queue(
        &self,
        at: Invocation,
        vm_id: &[u8],
        name: &[u8],
    ) -> Result<QueueId, Status> {
        let queue = self.store.resolve_shared_queue(at, vm_id, name)?;
        self.recorder
            .push(at, Event::ResolveQueue(text(name), queue));
        Ok(queue)
    }

    fn enqueue_shared_queue(
        &self,
        at: Invocation,
        queue: QueueId,
        value: &[u8],
    ) -> Result<(), Status> {
        let event = Event::Enqueue(queue, text(value));
        // The store reports the item to its observer before it returns, so
        // the call goes in front of that report.
        let index = self.recorder.entries().len();
        self.store.enqueue_shared_queue(at, queue, value)?;
        self.recorder.entries().insert(index, (Some(at), event));
        Ok(())
    }

    fn dequeue_shared_queue(&self, at: Invocation, queue: QueueId) -> Result<Vec<u8>, Status> {
        let value = self.store.dequeue_shared_queue(at, queue)?;
        self.recorder.push(at, Event::Dequeue(queue, text(&value)));
        Ok(value)
    }

    fn define_metric(
        &self,
        at: Invocation,
        vm_id: &[u8],
        kind: MetricType,
        name: &[u8],
    ) -> Result<MetricId, Status> {
        let metric = self.store.define_metric(at, vm_id, kind, name)?;
        self.recorder
            .push(at, Event::DefineMetric(kind, text(name)));
        Ok(metric)
    }

    fn record_metric(&self, at: Invocation, metric: MetricId, value: u64) -> Result<(), Status> {
        self.store.record_metric(at, metric, value)?;
        self.recorder.push(at, Event::RecordMetric(metric, value));
        Ok(())
    }

    fn increment_metric(&self, at: Invocation, metric: MetricId, delta: i64) -> Result<(), Status> {
        self.store.increment_metric(at, metric, delta)?;
        self.recorder
            .push(at, Event::IncrementMetric(metric, delta));
        Ok(())
    }

    fn get_metric(&self, at: Invocation, metric: MetricId) -> Result<u64, Status> {
        self.store.get_metric(at, metric)
    }
}

/// A request or a connection that holds every map and buffer a stream
/// callback reads, and records each operation on the stream.
#[derive(Default)]
pub struct StreamDouble {
    recorder: Recorder,
    /// The refusals this double reports in place of its usual answer.
    pub refusals: StreamRefusals,
    pub request_headers: VecHeaderMap,
    pub request_trailers: VecHeaderMap,
    pub response_headers: VecHeaderMap,
    pub response_trailers: VecHeaderMap,
    pub request_body: Vec<u8>,
    pub response_body: Vec<u8>,
    pub downstream: Vec<u8>,
    pub upstream: Vec<u8>,
    pub properties: BTreeMap<Vec<String>, String>,
}

impl StreamDouble {
    /// The refusal a test asked for on `call`, as an error to report.
    fn refused(&self, call: StreamCall) -> Result<(), Status> {
        match self.refusals.get(&call) {
            Some(status) => Err(*status),
            None => Ok(()),
        }
    }

    /// A stream whose events go to `recorder`, with the request path
    /// property `/exercise`.
    pub fn new(recorder: &Recorder) -> Self {
        let mut properties = BTreeMap::new();
        properties.insert(
            vec!["request".to_owned(), "path".to_owned()],
            "/exercise".to_owned(),
        );
        Self {
            recorder: recorder.clone(),
            properties,
            ..Self::default()
        }
    }

    /// The same stream with `headers` as its request headers.
    pub fn with_request_headers(mut self, headers: &[(&str, &str)]) -> Self {
        self.request_headers = headers
            .iter()
            .map(|(key, value)| (key.as_bytes().to_vec(), value.as_bytes().to_vec()))
            .collect();
        self
    }

    /// The same for the response map, which a response callback reads.
    pub fn with_response_headers(mut self, headers: &[(&str, &str)]) -> Self {
        self.response_headers = headers
            .iter()
            .map(|(key, value)| (key.as_bytes().to_vec(), value.as_bytes().to_vec()))
            .collect();
        self
    }
}

fn path(path: &[&[u8]]) -> Vec<String> {
    path.iter().map(|segment| text(segment)).collect()
}

impl StreamState for StreamDouble {
    fn header_map(
        &mut self,
        _: Invocation,
        _: Access,
        map: MapType,
    ) -> Result<&mut dyn HeaderMap, Status> {
        self.refused(StreamCall::HeaderMap)?;
        match map {
            MapType::HttpRequestHeaders => Ok(&mut self.request_headers),
            MapType::HttpRequestTrailers => Ok(&mut self.request_trailers),
            MapType::HttpResponseHeaders => Ok(&mut self.response_headers),
            MapType::HttpResponseTrailers => Ok(&mut self.response_trailers),
            _ => Err(Status::NotFound),
        }
    }

    fn buffer(
        &mut self,
        _: Invocation,
        _: Access,
        buffer: BufferType,
    ) -> Result<&mut dyn Buffer, Status> {
        self.refused(StreamCall::Buffer)?;
        match buffer {
            BufferType::HttpRequestBody => Ok(&mut self.request_body),
            BufferType::HttpResponseBody => Ok(&mut self.response_body),
            BufferType::DownstreamData => Ok(&mut self.downstream),
            BufferType::UpstreamData => Ok(&mut self.upstream),
            _ => Err(Status::NotFound),
        }
    }

    fn continue_stream(&mut self, at: Invocation, stream: StreamType) -> Result<(), Status> {
        self.refused(StreamCall::ContinueStream)?;
        self.recorder.push(at, Event::ContinueStream(stream));
        Ok(())
    }

    fn close_stream(&mut self, at: Invocation, stream: StreamType) -> Result<(), Status> {
        self.refused(StreamCall::CloseStream)?;
        self.recorder.push(at, Event::CloseStream(stream));
        Ok(())
    }

    fn send_local_response(
        &mut self,
        at: Invocation,
        response: LocalResponse<'_>,
    ) -> Result<(), Status> {
        self.refused(StreamCall::SendLocalResponse)?;
        let headers = response
            .headers
            .iter()
            .map(|(key, value)| (text(key), text(value)))
            .collect();
        let event = Event::LocalResponse {
            status: response.status_code,
            headers,
            body: text(&response.body),
            details: text(&response.status_code_details),
            grpc_status: response.grpc_status,
        };
        self.recorder.push(at, event);
        Ok(())
    }

    fn property(&mut self, _: Invocation, segments: &[&[u8]]) -> Result<Vec<u8>, Status> {
        self.refused(StreamCall::Property)?;
        self.properties
            .get(&path(segments))
            .map(|value| value.as_bytes().to_vec())
            .ok_or(Status::NotFound)
    }

    fn set_property(
        &mut self,
        at: Invocation,
        segments: &[&[u8]],
        value: &[u8],
    ) -> Result<(), Status> {
        self.refused(StreamCall::SetProperty)?;
        let event = Event::SetProperty(path(segments), text(value));
        self.recorder.push(at, event);
        self.properties.insert(path(segments), text(value));
        Ok(())
    }

    fn call_foreign_function(
        &mut self,
        at: Invocation,
        request: ForeignCall<'_>,
    ) -> Result<Vec<u8>, Status> {
        self.refused(StreamCall::CallForeignFunction)?;
        let event = Event::ForeignCall {
            name: text(&request.name),
            arguments: text(&request.arguments),
        };
        self.recorder.push(at, event);
        Ok(b"pong".to_vec())
    }
}
