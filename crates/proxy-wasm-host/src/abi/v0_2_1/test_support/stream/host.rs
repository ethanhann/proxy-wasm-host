//! What the recording stream answers a host function.

use std::borrow::Cow;

use crate::Buffer;
use crate::abi::v0_2_1::types::{BufferType, MapType, Status, StreamType};
use crate::abi::v0_2_1::{
    Access, CalloutStatus, ForeignCall, Invocation, LocalResponse, StreamState,
};
use crate::header_map::HeaderMap;

use super::{Operation, Path, RecordingStream};

impl StreamState for RecordingStream {
    fn header_map(
        &mut self,
        call: Invocation,
        access: Access,
        map: MapType,
    ) -> Result<&mut dyn HeaderMap, Status> {
        self.calls.push((call, access, map));
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

    fn buffer(
        &mut self,
        call: Invocation,
        access: Access,
        buffer: BufferType,
    ) -> Result<&mut dyn Buffer, Status> {
        self.buffer_calls.push((call, access, buffer));
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

    fn continue_stream(&mut self, call: Invocation, stream: StreamType) -> Result<(), Status> {
        self.operations.push((call, Operation::Continue(stream)));
        self.operation_answer()
    }

    fn close_stream(&mut self, call: Invocation, stream: StreamType) -> Result<(), Status> {
        self.operations.push((call, Operation::Close(stream)));
        self.operation_answer()
    }

    fn callout_status(&mut self, call: Invocation) -> Result<CalloutStatus<'_>, Status> {
        self.callout_calls.push(call);
        if self.refuse_with_ok {
            return Err(Status::Ok);
        }
        let (code, message) = self.callout.as_ref().ok_or(Status::Unimplemented)?;
        Ok(CalloutStatus::new(*code, Cow::Borrowed(message)))
    }

    fn property(&mut self, call: Invocation, path: &[&[u8]]) -> Result<Vec<u8>, Status> {
        let owned: Path = path.iter().map(|part| part.to_vec()).collect();
        self.property_reads.push((call, owned.clone()));
        if self.refuse_with_ok {
            return Err(Status::Ok);
        }
        self.properties.get(&owned).cloned().ok_or(Status::NotFound)
    }

    fn set_property(
        &mut self,
        call: Invocation,
        path: &[&[u8]],
        value: &[u8],
    ) -> Result<(), Status> {
        let owned: Path = path.iter().map(|part| part.to_vec()).collect();
        self.property_writes.push((call, owned, value.to_vec()));
        if self.refuse_with_ok {
            return Err(Status::Ok);
        }
        Ok(())
    }

    fn call_foreign_function(
        &mut self,
        call: Invocation,
        request: ForeignCall<'_>,
    ) -> Result<Vec<u8>, Status> {
        let name = request.name.clone().into_owned();
        self.foreign_calls.push((call, request.into_owned()));
        if self.refuse_with_ok {
            return Err(Status::Ok);
        }
        self.foreign.get(&name).cloned().ok_or(Status::NotFound)
    }

    fn send_local_response(
        &mut self,
        call: Invocation,
        response: LocalResponse<'_>,
    ) -> Result<(), Status> {
        self.local_response = Some((call, response.into_owned()));
        if self.refuse_with_ok {
            return Err(Status::Ok);
        }
        Ok(())
    }
}
