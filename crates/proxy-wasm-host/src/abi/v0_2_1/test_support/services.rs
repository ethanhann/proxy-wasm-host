//! The shared services double that the ABI layer's tests share.

use std::sync::{Mutex, PoisonError};

use crate::abi::v0_2_1::types::{MetricType, Status};
use crate::abi::v0_2_1::{
    InMemoryStore, Invocation, MetricId, QueueId, SharedServices, SharedValue,
};

/// One call a body made into the shared services.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SharedCall {
    Get(Vec<u8>, Vec<u8>),
    Set(Vec<u8>, Vec<u8>, Vec<u8>, Option<u32>),
    Register(Vec<u8>, Vec<u8>),
    Resolve(Vec<u8>, Vec<u8>),
    Enqueue(QueueId, Vec<u8>),
    Dequeue(QueueId),
    Define(Vec<u8>, MetricType, Vec<u8>),
    Record(MetricId, u64),
    Increment(MetricId, i64),
    GetMetric(MetricId),
}

/// Shared services that record every call and serve them from a store, or
/// refuse them all with one status.
///
/// A test keeps its own `Arc` of this and passes a clone to
/// `VmServices::with_shared`, because the trait is not downcastable.
#[derive(Debug, Default)]
pub(crate) struct RecordingServices {
    inner: InMemoryStore,
    calls: Mutex<Vec<(Invocation, SharedCall)>>,
    refusal: Option<Status>,
}

impl RecordingServices {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Services that refuse every call with `status`.
    pub(crate) fn refusing(mut self, status: Status) -> Self {
        self.refusal = Some(status);
        self
    }

    pub(crate) fn calls(&self) -> Vec<(Invocation, SharedCall)> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record<T>(&self, call: Invocation, made: SharedCall, answer: T) -> Result<T, Status> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((call, made));
        match self.refusal {
            Some(status) => Err(status),
            None => Ok(answer),
        }
    }
}

impl SharedServices for RecordingServices {
    fn get_shared_data(
        &self,
        call: Invocation,
        vm_id: &[u8],
        key: &[u8],
    ) -> Result<SharedValue, Status> {
        let made = SharedCall::Get(vm_id.to_vec(), key.to_vec());
        self.record(call, made, ())?;
        self.inner.get_shared_data(call, vm_id, key)
    }

    fn set_shared_data(
        &self,
        call: Invocation,
        vm_id: &[u8],
        key: &[u8],
        value: &[u8],
        cas: Option<u32>,
    ) -> Result<(), Status> {
        let made = SharedCall::Set(vm_id.to_vec(), key.to_vec(), value.to_vec(), cas);
        self.record(call, made, ())?;
        self.inner.set_shared_data(call, vm_id, key, value, cas)
    }

    fn register_shared_queue(
        &self,
        call: Invocation,
        vm_id: &[u8],
        name: &[u8],
    ) -> Result<QueueId, Status> {
        let made = SharedCall::Register(vm_id.to_vec(), name.to_vec());
        self.record(call, made, ())?;
        self.inner.register_shared_queue(call, vm_id, name)
    }

    fn resolve_shared_queue(
        &self,
        call: Invocation,
        vm_id: &[u8],
        name: &[u8],
    ) -> Result<QueueId, Status> {
        let made = SharedCall::Resolve(vm_id.to_vec(), name.to_vec());
        self.record(call, made, ())?;
        self.inner.resolve_shared_queue(call, vm_id, name)
    }

    fn enqueue_shared_queue(
        &self,
        call: Invocation,
        queue: QueueId,
        value: &[u8],
    ) -> Result<(), Status> {
        let made = SharedCall::Enqueue(queue, value.to_vec());
        self.record(call, made, ())?;
        self.inner.enqueue_shared_queue(call, queue, value)
    }

    fn dequeue_shared_queue(&self, call: Invocation, queue: QueueId) -> Result<Vec<u8>, Status> {
        let made = SharedCall::Dequeue(queue);
        self.record(call, made, ())?;
        self.inner.dequeue_shared_queue(call, queue)
    }

    fn define_metric(
        &self,
        call: Invocation,
        vm_id: &[u8],
        kind: MetricType,
        name: &[u8],
    ) -> Result<MetricId, Status> {
        let made = SharedCall::Define(vm_id.to_vec(), kind, name.to_vec());
        self.record(call, made, ())?;
        self.inner.define_metric(call, vm_id, kind, name)
    }

    fn record_metric(&self, call: Invocation, metric: MetricId, value: u64) -> Result<(), Status> {
        let made = SharedCall::Record(metric, value);
        self.record(call, made, ())?;
        self.inner.record_metric(call, metric, value)
    }

    fn increment_metric(
        &self,
        call: Invocation,
        metric: MetricId,
        delta: i64,
    ) -> Result<(), Status> {
        let made = SharedCall::Increment(metric, delta);
        self.record(call, made, ())?;
        self.inner.increment_metric(call, metric, delta)
    }

    fn get_metric(&self, call: Invocation, metric: MetricId) -> Result<u64, Status> {
        let made = SharedCall::GetMetric(metric);
        self.record(call, made, ())?;
        self.inner.get_metric(call, metric)
    }
}
