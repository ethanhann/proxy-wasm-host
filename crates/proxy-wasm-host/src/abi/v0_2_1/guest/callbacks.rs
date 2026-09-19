//! The callbacks a guest exports, resolved once at construction.

use wasmtime::TypedFunc;

use crate::Error;
use crate::abi::v0_2_1::Callback;
use crate::runtime::Instance;

/// `proxy_on_http_call_response`, which takes the plugin context, the callout,
/// and the three counts.
type HttpCallResponseFn = TypedFunc<(i32, i32, i32, i32, i32), ()>;

/// The callbacks this crate drives, resolved once at construction.
pub(crate) struct Callbacks {
    pub(crate) context_create: Option<TypedFunc<(i32, i32), ()>>,
    pub(crate) vm_start: Option<TypedFunc<(i32, i32), i32>>,
    pub(crate) configure: Option<TypedFunc<(i32, i32), i32>>,
    pub(crate) request_headers: Option<TypedFunc<(i32, i32, i32), i32>>,
    pub(crate) http_call_response: Option<HttpCallResponseFn>,
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
            request_headers: instance.typed_func(Callback::RequestHeaders.export_name())?,
            http_call_response: instance.typed_func(Callback::HttpCallResponse.export_name())?,
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
            Callback::RequestHeaders => self.request_headers.is_some(),
            Callback::HttpCallResponse => self.http_call_response.is_some(),
            Callback::Done => self.done.is_some(),
            Callback::Log => self.log.is_some(),
            Callback::Delete => self.delete.is_some(),
            Callback::Tick => self.tick.is_some(),
            Callback::QueueReady => self.queue_ready.is_some(),
        }
    }
}
