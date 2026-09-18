//! The stream host double that the ABI layer's tests share.

mod host;

use std::any::Any;
use std::collections::HashMap;

use super::doubles::{Ranges, ReadOnlyBuffer, ReadOnlyMap, RecordingBuffer, owned};
use crate::Buffer;
use crate::abi::v0_2_1::types::{BufferType, MapType, Status, StreamType};
use crate::abi::v0_2_1::{Access, ForeignCall, HostCall, LocalResponse};
use crate::header_map::HeaderMap;
use crate::runtime::HostState;

/// The segments of one property path.
pub(crate) type Path = Vec<Vec<u8>>;

/// One write a body made to a property.
pub(crate) type PropertyWrite = (HostCall, Path, Vec<u8>);

/// A stream operation the guest asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Operation {
    Continue(StreamType),
    Close(StreamType),
}

/// A stream host that serves what it was given and records every call.
pub(crate) struct RecordingStream {
    maps: HashMap<MapType, Box<dyn HeaderMap + Send>>,
    map_refusals: HashMap<MapType, Status>,
    calls: Vec<(HostCall, Access, MapType)>,
    buffers: HashMap<BufferType, Box<dyn Buffer + Send>>,
    buffer_refusals: HashMap<BufferType, Status>,
    buffer_calls: Vec<(HostCall, Access, BufferType)>,
    operations: Vec<(HostCall, Operation)>,
    operation_refusal: Option<Status>,
    callout: Option<(u32, Vec<u8>)>,
    callout_calls: Vec<HostCall>,
    local_response: Option<(HostCall, LocalResponse<'static>)>,
    ranges: Ranges,
    properties: HashMap<Path, Vec<u8>>,
    property_reads: Vec<(HostCall, Path)>,
    property_writes: Vec<PropertyWrite>,
    foreign: HashMap<Vec<u8>, Vec<u8>>,
    foreign_calls: Vec<(HostCall, ForeignCall<'static>)>,
    refuse_with_ok: bool,
}

impl RecordingStream {
    /// A stream with an empty request header map.
    pub(crate) fn new() -> Self {
        let stream = Self {
            maps: HashMap::new(),
            map_refusals: HashMap::new(),
            calls: Vec::new(),
            buffers: HashMap::new(),
            buffer_refusals: HashMap::new(),
            buffer_calls: Vec::new(),
            operations: Vec::new(),
            operation_refusal: None,
            callout: None,
            callout_calls: Vec::new(),
            local_response: None,
            ranges: Ranges::default(),
            properties: HashMap::new(),
            property_reads: Vec::new(),
            property_writes: Vec::new(),
            foreign: HashMap::new(),
            foreign_calls: Vec::new(),
            refuse_with_ok: false,
        };
        stream.with_map(MapType::HttpRequestHeaders, &[])
    }

    pub(crate) fn with_map(mut self, map: MapType, pairs: &[(&str, &str)]) -> Self {
        self.maps.insert(map, Box::new(owned(pairs)));
        self
    }

    pub(crate) fn with_read_only_map(mut self, map: MapType, pairs: &[(&str, &str)]) -> Self {
        self.maps.insert(map, Box::new(ReadOnlyMap(owned(pairs))));
        self
    }

    pub(crate) fn refusing(mut self, map: MapType, status: Status) -> Self {
        self.map_refusals.insert(map, status);
        self
    }

    pub(crate) fn with_buffer(mut self, buffer: BufferType, bytes: &[u8]) -> Self {
        self.buffers.insert(buffer, Box::new(bytes.to_vec()));
        self
    }

    pub(crate) fn with_read_only_buffer(mut self, buffer: BufferType, bytes: &[u8]) -> Self {
        self.buffers
            .insert(buffer, Box::new(ReadOnlyBuffer(bytes.to_vec())));
        self
    }

    pub(crate) fn with_recording_buffer(mut self, buffer: BufferType, bytes: &[u8]) -> Self {
        self.buffers
            .insert(buffer, Box::new(RecordingBuffer::new(bytes, &self.ranges)));
        self
    }

    pub(crate) fn refusing_buffer(mut self, buffer: BufferType, status: Status) -> Self {
        self.buffer_refusals.insert(buffer, status);
        self
    }

    pub(crate) fn refusing_operation(mut self, status: Status) -> Self {
        self.operation_refusal = Some(status);
        self
    }

    pub(crate) fn with_property(mut self, path: &[&[u8]], value: &[u8]) -> Self {
        let owned = path.iter().map(|part| part.to_vec()).collect();
        self.properties.insert(owned, value.to_vec());
        self
    }

    pub(crate) fn with_foreign_function(mut self, name: &[u8], result: &[u8]) -> Self {
        self.foreign.insert(name.to_vec(), result.to_vec());
        self
    }

    pub(crate) fn property_reads(&self) -> &[(HostCall, Path)] {
        &self.property_reads
    }

    pub(crate) fn property_writes(&self) -> &[PropertyWrite] {
        &self.property_writes
    }

    pub(crate) fn foreign_calls(&self) -> &[(HostCall, ForeignCall<'static>)] {
        &self.foreign_calls
    }

    pub(crate) fn with_callout_status(mut self, code: u32, message: &[u8]) -> Self {
        self.callout = Some((code, message.to_vec()));
        self
    }

    /// Makes every method report success for something it never touched.
    pub(crate) fn refusing_with_ok(mut self) -> Self {
        self.refuse_with_ok = true;
        self
    }

    /// The pairs of one map as strings.
    pub(crate) fn pairs(&self, map: MapType) -> Vec<(String, String)> {
        self.maps
            .get(&map)
            .map(|map| {
                map.pairs()
                    .into_iter()
                    .map(|(key, value)| {
                        (
                            String::from_utf8(key).unwrap(),
                            String::from_utf8(value).unwrap(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The bytes of one buffer.
    pub(crate) fn bytes(&self, buffer: BufferType) -> Vec<u8> {
        self.buffers
            .get(&buffer)
            .map(|buffer| buffer.copy_range(0, usize::MAX))
            .unwrap_or_default()
    }

    /// Every range a recording buffer was asked for.
    pub(crate) fn ranges(&self) -> Vec<(usize, usize)> {
        self.ranges.lock().unwrap().clone()
    }

    /// The request header pairs of the stream installed in `state`.
    pub(crate) fn pairs_in(state: &mut HostState) -> Vec<(String, String)> {
        state
            .abi_mut()
            .stream_host_as::<Self>()
            .expect("a RecordingStream is installed")
            .pairs(MapType::HttpRequestHeaders)
    }

    /// The bytes of one buffer of the stream installed in `state`.
    pub(crate) fn bytes_in(state: &mut HostState, buffer: BufferType) -> Vec<u8> {
        state
            .abi_mut()
            .stream_host_as::<Self>()
            .expect("a RecordingStream is installed")
            .bytes(buffer)
    }

    pub(crate) fn calls(&self) -> &[(HostCall, Access, MapType)] {
        &self.calls
    }

    pub(crate) fn buffer_calls(&self) -> &[(HostCall, Access, BufferType)] {
        &self.buffer_calls
    }

    pub(crate) fn operations(&self) -> &[(HostCall, Operation)] {
        &self.operations
    }

    pub(crate) fn callout_calls(&self) -> &[HostCall] {
        &self.callout_calls
    }

    pub(crate) fn local_response(&self) -> Option<&(HostCall, LocalResponse<'static>)> {
        self.local_response.as_ref()
    }

    /// Takes the stream out of `state`.
    pub(crate) fn take(state: &mut HostState) -> Self {
        let boxed = state
            .abi_mut()
            .take_stream_host()
            .expect("a stream host is installed");
        let any: Box<dyn Any + Send> = boxed;
        *any.downcast::<Self>()
            .expect("the stream host is a RecordingStream")
    }
}

impl RecordingStream {
    fn operation_answer(&self) -> Result<(), Status> {
        if self.refuse_with_ok {
            return Err(Status::Ok);
        }
        match self.operation_refusal {
            Some(status) => Err(status),
            None => Ok(()),
        }
    }
}
