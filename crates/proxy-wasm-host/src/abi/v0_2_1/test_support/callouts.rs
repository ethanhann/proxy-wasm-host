//! A callout service that records what the crate gave it.

use std::sync::{Arc, Mutex, PoisonError};

use crate::abi::v0_2_1::AbiAccess;
use crate::abi::v0_2_1::test_support::{RecordingSink, instance_with, wat_bytes};
use crate::abi::v0_2_1::{
    Callback, CalloutId, Callouts, ContextId, GrpcCall, GrpcOpenRefusal, GrpcStream, HttpCall,
    HttpCallRefusal, Invocation, VmServices,
};
use crate::runtime::{Engine, Instance, Module};

/// What the crate asked the service to do with a gRPC callout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum GrpcAsk {
    Call(GrpcCall<'static>),
    Stream(GrpcStream<'static>),
    Send { message: Vec<u8>, end: bool },
    Cancel,
    Close,
}

/// A service that accepts every callout, or refuses every one.
#[derive(Debug, Default)]
pub(crate) struct RecordingCallouts {
    calls: Mutex<Vec<(Invocation, CalloutId, HttpCall<'static>)>>,
    grpc: Mutex<Vec<(Invocation, CalloutId, GrpcAsk)>>,
    refusal: Option<HttpCallRefusal>,
    grpc_refusal: Option<GrpcOpenRefusal>,
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

    /// Refuses every gRPC call and stream with `refusal`.
    pub(crate) fn refusing_grpc(mut self, refusal: GrpcOpenRefusal) -> Self {
        self.grpc_refusal = Some(refusal);
        self
    }

    /// Everything the service was asked to do with a gRPC callout, in order.
    pub(crate) fn grpc(&self) -> Vec<(Invocation, CalloutId, GrpcAsk)> {
        self.grpc
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record_grpc(&self, call: Invocation, callout: CalloutId, ask: GrpcAsk) {
        self.grpc
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((call, callout, ask));
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

    fn grpc_call(
        &self,
        call: Invocation,
        callout: CalloutId,
        request: GrpcCall<'_>,
    ) -> Result<(), GrpcOpenRefusal> {
        self.record_grpc(call, callout, GrpcAsk::Call(request.into_owned()));
        self.grpc_refusal.map_or(Ok(()), Err)
    }

    fn grpc_stream(
        &self,
        call: Invocation,
        callout: CalloutId,
        request: GrpcStream<'_>,
    ) -> Result<(), GrpcOpenRefusal> {
        self.record_grpc(call, callout, GrpcAsk::Stream(request.into_owned()));
        self.grpc_refusal.map_or(Ok(()), Err)
    }

    fn grpc_send(&self, call: Invocation, callout: CalloutId, message: &[u8], end_of_stream: bool) {
        let ask = GrpcAsk::Send {
            message: message.to_vec(),
            end: end_of_stream,
        };
        self.record_grpc(call, callout, ask);
    }

    fn grpc_cancel(&self, call: Invocation, callout: CalloutId) {
        self.record_grpc(call, callout, GrpcAsk::Cancel);
    }

    fn grpc_close(&self, call: Invocation, callout: CalloutId) {
        self.record_grpc(call, callout, GrpcAsk::Close);
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
