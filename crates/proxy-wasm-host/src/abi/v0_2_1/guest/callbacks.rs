//! The callbacks a guest exports, resolved once at construction.

use wasmtime::TypedFunc;

use crate::Error;
use crate::abi::v0_2_1::Callback;
use crate::runtime::Instance;

/// The seven callbacks, resolved once at construction.
pub(crate) struct Callbacks {
    pub(crate) context_create: Option<TypedFunc<(i32, i32), ()>>,
    pub(crate) vm_start: Option<TypedFunc<(i32, i32), i32>>,
    pub(crate) configure: Option<TypedFunc<(i32, i32), i32>>,
    pub(crate) request_headers: Option<TypedFunc<(i32, i32, i32), i32>>,
    pub(crate) done: Option<TypedFunc<i32, i32>>,
    pub(crate) log: Option<TypedFunc<i32, ()>>,
    pub(crate) delete: Option<TypedFunc<i32, ()>>,
}

impl Callbacks {
    pub(super) fn resolve(instance: &mut Instance) -> Result<Self, Error> {
        Ok(Self {
            context_create: instance.typed_func(Callback::ContextCreate.export_name())?,
            vm_start: instance.typed_func(Callback::VmStart.export_name())?,
            configure: instance.typed_func(Callback::Configure.export_name())?,
            request_headers: instance.typed_func(Callback::RequestHeaders.export_name())?,
            done: instance.typed_func(Callback::Done.export_name())?,
            log: instance.typed_func(Callback::Log.export_name())?,
            delete: instance.typed_func(Callback::Delete.export_name())?,
        })
    }

    pub(super) fn exports(&self, callback: Callback) -> bool {
        match callback {
            Callback::ContextCreate => self.context_create.is_some(),
            Callback::VmStart => self.vm_start.is_some(),
            Callback::Configure => self.configure.is_some(),
            Callback::RequestHeaders => self.request_headers.is_some(),
            Callback::Done => self.done.is_some(),
            Callback::Log => self.log.is_some(),
            Callback::Delete => self.delete.is_some(),
        }
    }
}
