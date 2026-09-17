//! The stream host double that the ABI layer's tests share.

use std::any::Any;
use std::borrow::Cow;
use std::collections::HashMap;

use super::doubles::{Ranges, ReadOnlyBuffer, ReadOnlyMap, RecordingBuffer, owned};
use crate::Buffer;
use crate::abi::v0_2_1::types::{BufferType, MapType, Status, StreamType};
use crate::abi::v0_2_1::{CalloutStatus, HostCall, LocalResponse, StreamHost};
use crate::header_map::HeaderMap;
use crate::runtime::HostState;

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
    calls: Vec<(HostCall, MapType)>,
    buffers: HashMap<BufferType, Box<dyn Buffer + Send>>,
    buffer_refusals: HashMap<BufferType, Status>,
    buffer_calls: Vec<(HostCall, BufferType)>,
    operations: Vec<(HostCall, Operation)>,
    operation_refusal: Option<Status>,
    callout: Option<(u32, Vec<u8>)>,
    callout_calls: Vec<HostCall>,
    local_response: Option<(HostCall, LocalResponse<'static>)>,
    ranges: Ranges,
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

    pub(crate) fn calls(&self) -> &[(HostCall, MapType)] {
        &self.calls
    }

    pub(crate) fn buffer_calls(&self) -> &[(HostCall, BufferType)] {
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

impl StreamHost for RecordingStream {
    fn header_map(&mut self, call: HostCall, map: MapType) -> Result<&mut dyn HeaderMap, Status> {
        self.calls.push((call, map));
        if self.refuse_with_ok {
            return Err(Status::Ok);
        }
        if let Some(status) = self.map_refusals.get(&map) {
            return Err(*status);
        }
        self.maps
            .get_mut(&map)
            .map(|map| map.as_mut() as &mut dyn HeaderMap)
            .ok_or(Status::BadArgument)
    }

    fn buffer(&mut self, call: HostCall, buffer: BufferType) -> Result<&mut dyn Buffer, Status> {
        self.buffer_calls.push((call, buffer));
        if self.refuse_with_ok {
            return Err(Status::Ok);
        }
        if let Some(status) = self.buffer_refusals.get(&buffer) {
            return Err(*status);
        }
        self.buffers
            .get_mut(&buffer)
            .map(|buffer| buffer.as_mut() as &mut dyn Buffer)
            .ok_or(Status::NotFound)
    }

    fn continue_stream(&mut self, call: HostCall, stream: StreamType) -> Result<(), Status> {
        self.operations.push((call, Operation::Continue(stream)));
        self.operation_answer()
    }

    fn close_stream(&mut self, call: HostCall, stream: StreamType) -> Result<(), Status> {
        self.operations.push((call, Operation::Close(stream)));
        self.operation_answer()
    }

    fn callout_status(&mut self, call: HostCall) -> Result<CalloutStatus<'_>, Status> {
        self.callout_calls.push(call);
        if self.refuse_with_ok {
            return Err(Status::Ok);
        }
        let (code, message) = self.callout.as_ref().ok_or(Status::Unimplemented)?;
        Ok(CalloutStatus::new(*code, Cow::Borrowed(message)))
    }

    fn send_local_response(
        &mut self,
        call: HostCall,
        response: LocalResponse<'_>,
    ) -> Result<(), Status> {
        self.local_response = Some((call, response.into_owned()));
        if self.refuse_with_ok {
            return Err(Status::Ok);
        }
        Ok(())
    }
}
