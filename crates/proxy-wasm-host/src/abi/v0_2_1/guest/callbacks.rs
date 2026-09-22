//! The callbacks a guest exports, resolved once at construction.

use wasmtime::TypedFunc;

use crate::Error;
use crate::abi::v0_2_1::Callback;
use crate::runtime::Instance;

/// `proxy_on_http_call_response`, which takes the plugin context, the callout,
/// and the three counts.
type HttpCallResponseFn = TypedFunc<(i32, i32, i32, i32, i32), ()>;

/// A gRPC callback, which takes the plugin context, the callout, and one
/// count, size, or code.
type GrpcFn = TypedFunc<(i32, i32, i32), ()>;

/// A stream callback that takes a count and an end of stream flag and
/// answers an action.
type DataFn = TypedFunc<(i32, i32, i32), i32>;

/// A stream callback that takes one count and answers an action.
type CountFn = TypedFunc<(i32, i32), i32>;

/// A callback that closes one side of a connection, which takes the peer
/// type and answers nothing.
type CloseFn = TypedFunc<(i32, i32), ()>;

/// `proxy_on_foreign_function`, which takes the context, the function, and
/// the size of the arguments.
type ForeignFn = TypedFunc<(i32, i32, i32), ()>;

/// The callbacks this crate drives, resolved once at construction.
pub(crate) struct Callbacks {
    pub(crate) context_create: Option<TypedFunc<(i32, i32), ()>>,
    pub(crate) vm_start: Option<TypedFunc<(i32, i32), i32>>,
    pub(crate) configure: Option<TypedFunc<(i32, i32), i32>>,
    pub(crate) new_connection: Option<TypedFunc<i32, i32>>,
    pub(crate) downstream_data: Option<DataFn>,
    pub(crate) downstream_connection_close: Option<CloseFn>,
    pub(crate) upstream_data: Option<DataFn>,
    pub(crate) upstream_connection_close: Option<CloseFn>,
    pub(crate) request_headers: Option<DataFn>,
    pub(crate) request_body: Option<DataFn>,
    pub(crate) request_trailers: Option<CountFn>,
    pub(crate) response_headers: Option<DataFn>,
    pub(crate) response_body: Option<DataFn>,
    pub(crate) response_trailers: Option<CountFn>,
    pub(crate) foreign_function: Option<ForeignFn>,
    pub(crate) http_call_response: Option<HttpCallResponseFn>,
    pub(crate) grpc_receive_initial_metadata: Option<GrpcFn>,
    pub(crate) grpc_receive: Option<GrpcFn>,
    pub(crate) grpc_receive_trailing_metadata: Option<GrpcFn>,
    pub(crate) grpc_close: Option<GrpcFn>,
    pub(crate) done: Option<TypedFunc<i32, i32>>,
    pub(crate) log: Option<TypedFunc<i32, ()>>,
    pub(crate) delete: Option<TypedFunc<i32, ()>>,
    pub(crate) tick: Option<TypedFunc<i32, ()>>,
    pub(crate) queue_ready: Option<TypedFunc<(i32, i32), ()>>,
}

impl Callbacks {
    pub(super) fn resolve(instance: &mut Instance) -> Result<Self, Error> {
        Ok(Self {
            context_create: instance.typed_func(Callback::ContextCreate.export_name())?,
            vm_start: instance.typed_func(Callback::VmStart.export_name())?,
            configure: instance.typed_func(Callback::Configure.export_name())?,
            new_connection: instance.typed_func(Callback::NewConnection.export_name())?,
            downstream_data: instance.typed_func(Callback::DownstreamData.export_name())?,
            downstream_connection_close: instance
                .typed_func(Callback::DownstreamConnectionClose.export_name())?,
            upstream_data: instance.typed_func(Callback::UpstreamData.export_name())?,
            upstream_connection_close: instance
                .typed_func(Callback::UpstreamConnectionClose.export_name())?,
            request_headers: instance.typed_func(Callback::RequestHeaders.export_name())?,
            request_body: instance.typed_func(Callback::RequestBody.export_name())?,
            request_trailers: instance.typed_func(Callback::RequestTrailers.export_name())?,
            response_headers: instance.typed_func(Callback::ResponseHeaders.export_name())?,
            response_body: instance.typed_func(Callback::ResponseBody.export_name())?,
            response_trailers: instance.typed_func(Callback::ResponseTrailers.export_name())?,
            foreign_function: instance.typed_func(Callback::ForeignFunction.export_name())?,
            http_call_response: instance.typed_func(Callback::HttpCallResponse.export_name())?,
            grpc_receive_initial_metadata: instance
                .typed_func(Callback::GrpcReceiveInitialMetadata.export_name())?,
            grpc_receive: instance.typed_func(Callback::GrpcReceive.export_name())?,
            grpc_receive_trailing_metadata: instance
                .typed_func(Callback::GrpcReceiveTrailingMetadata.export_name())?,
            grpc_close: instance.typed_func(Callback::GrpcClose.export_name())?,
            done: instance.typed_func(Callback::Done.export_name())?,
            log: instance.typed_func(Callback::Log.export_name())?,
            delete: instance.typed_func(Callback::Delete.export_name())?,
            tick: instance.typed_func(Callback::Tick.export_name())?,
            queue_ready: instance.typed_func(Callback::QueueReady.export_name())?,
        })
    }

    pub(super) fn exports(&self, callback: Callback) -> bool {
        match callback {
            Callback::ContextCreate => self.context_create.is_some(),
            Callback::VmStart => self.vm_start.is_some(),
            Callback::Configure => self.configure.is_some(),
            Callback::NewConnection => self.new_connection.is_some(),
            Callback::DownstreamData => self.downstream_data.is_some(),
            Callback::DownstreamConnectionClose => self.downstream_connection_close.is_some(),
            Callback::UpstreamData => self.upstream_data.is_some(),
            Callback::UpstreamConnectionClose => self.upstream_connection_close.is_some(),
            Callback::RequestHeaders => self.request_headers.is_some(),
            Callback::RequestBody => self.request_body.is_some(),
            Callback::RequestTrailers => self.request_trailers.is_some(),
            Callback::ResponseHeaders => self.response_headers.is_some(),
            Callback::ResponseBody => self.response_body.is_some(),
            Callback::ResponseTrailers => self.response_trailers.is_some(),
            Callback::ForeignFunction => self.foreign_function.is_some(),
            Callback::HttpCallResponse => self.http_call_response.is_some(),
            Callback::GrpcReceiveInitialMetadata => self.grpc_receive_initial_metadata.is_some(),
            Callback::GrpcReceive => self.grpc_receive.is_some(),
            Callback::GrpcReceiveTrailingMetadata => self.grpc_receive_trailing_metadata.is_some(),
            Callback::GrpcClose => self.grpc_close.is_some(),
            Callback::Done => self.done.is_some(),
            Callback::Log => self.log.is_some(),
            Callback::Delete => self.delete.is_some(),
            Callback::Tick => self.tick.is_some(),
            Callback::QueueReady => self.queue_ready.is_some(),
        }
    }
}
