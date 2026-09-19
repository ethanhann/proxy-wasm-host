//! A callout service that records what the crate gave it.

use std::sync::{Arc, Mutex, PoisonError};

use crate::abi::v0_2_1::AbiAccess;
use crate::abi::v0_2_1::test_support::{RecordingSink, instance_with, wat_bytes};
use crate::abi::v0_2_1::{
    Callback, CalloutId, Callouts, ContextId, HttpCall, HttpCallRefusal, Invocation, VmServices,
};
use crate::runtime::{Engine, Instance, Module};

/// A service that accepts every HTTP call, or refuses every one.
#[derive(Debug, Default)]
pub(crate) struct RecordingCallouts {
    calls: Mutex<Vec<(Invocation, CalloutId, HttpCall<'static>)>>,
    refusal: Option<HttpCallRefusal>,
}

impl RecordingCallouts {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Refuses every call with `refusal`.
    pub(crate) fn refusing(mut self, refusal: HttpCallRefusal) -> Self {
        self.refusal = Some(refusal);
        self
    }

    /// Every call the service was asked about, in order.
    pub(crate) fn calls(&self) -> Vec<(Invocation, CalloutId, HttpCall<'static>)> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Callouts for RecordingCallouts {
    fn http_call(
        &self,
        call: Invocation,
        callout: CalloutId,
        request: HttpCall<'_>,
    ) -> Result<(), HttpCallRefusal> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((call, callout, request.into_owned()));
        self.refusal.map_or(Ok(()), Err)
    }
}

/// Services with a recording sink and `callouts`.
pub(crate) fn services_with(callouts: Arc<RecordingCallouts>) -> VmServices {
    VmServices::new(Arc::new(RecordingSink::default())).with_callouts(callouts)
}

/// An instance of `wat` with `services`, a root context that is effective,
/// and a running callback.
pub(crate) fn callout_hosted(
    engine: &Engine,
    wat: &str,
    services: VmServices,
) -> (Instance, ContextId) {
    let module = Module::new(engine, &wat_bytes(wat)).unwrap();
    let mut instance = instance_with(engine, &module, services).unwrap();
    let state = instance.state_mut();
    let root = state.abi_mut().contexts_mut().create(None).unwrap();
    state.abi_mut().contexts_mut().set_effective(root);
    state
        .abi_mut()
        .set_current_callback(Some(Callback::RequestHeaders));
    (instance, root)
}
