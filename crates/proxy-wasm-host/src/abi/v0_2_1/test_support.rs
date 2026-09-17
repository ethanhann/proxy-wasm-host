//! Test doubles that the ABI layer's tests share.

use std::any::Any;
use std::borrow::Cow;
use std::collections::HashMap;
use std::ops::ControlFlow;

use crate::NotAllowed;
use crate::abi::v0_2_1::host_functions::Failure;
use crate::abi::v0_2_1::types::{MapType, Status};
use crate::abi::v0_2_1::{HostCall, StreamHost};
use crate::codec::pairs::PairVisitor;
use crate::header_map::{HeaderMap, VecHeaderMap};
use crate::runtime::HostState;

/// The status an `i32` from a host function wrapper stands for.
pub(crate) fn status(value: i32) -> Status {
    Status::try_from(value).unwrap()
}

/// The status a body's result stands for, when it did not unwind.
pub(crate) fn outcome(result: Result<(), Failure>) -> Status {
    match result {
        Ok(()) => Status::Ok,
        Err(Failure::Status(status)) => status,
        Err(Failure::Unwind(error)) => panic!("the body unwound: {error}"),
    }
}

/// A header map that refuses every write.
struct ReadOnly(VecHeaderMap);

impl HeaderMap for ReadOnly {
    fn get(&self, key: &[u8]) -> Option<Cow<'_, [u8]>> {
        self.0.get(key)
    }

    fn for_each_pair(&self, f: &mut PairVisitor<'_>) -> ControlFlow<()> {
        self.0.for_each_pair(f)
    }

    fn set(&mut self, _: &[u8], _: &[u8]) -> Result<(), NotAllowed> {
        Err(NotAllowed)
    }

    fn add(&mut self, _: &[u8], _: &[u8]) -> Result<(), NotAllowed> {
        Err(NotAllowed)
    }

    fn remove(&mut self, _: &[u8]) -> Result<(), NotAllowed> {
        Err(NotAllowed)
    }

    fn replace_all(&mut self, _: &[(&[u8], &[u8])]) -> Result<(), NotAllowed> {
        Err(NotAllowed)
    }
}

fn owned(pairs: &[(&str, &str)]) -> VecHeaderMap {
    pairs
        .iter()
        .map(|(key, value)| (key.as_bytes().to_vec(), value.as_bytes().to_vec()))
        .collect()
}

/// A stream host that serves the maps it was given and records every call.
pub(crate) struct RecordingStream {
    maps: HashMap<MapType, Box<dyn HeaderMap + Send>>,
    refusals: HashMap<MapType, Status>,
    calls: Vec<(HostCall, MapType)>,
}

impl RecordingStream {
    /// A stream with an empty request header map.
    pub(crate) fn new() -> Self {
        let stream = Self {
            maps: HashMap::new(),
            refusals: HashMap::new(),
            calls: Vec::new(),
        };
        stream.with_map(MapType::HttpRequestHeaders, &[])
    }

    pub(crate) fn with_map(mut self, map: MapType, pairs: &[(&str, &str)]) -> Self {
        self.maps.insert(map, Box::new(owned(pairs)));
        self
    }

    pub(crate) fn with_read_only_map(mut self, map: MapType, pairs: &[(&str, &str)]) -> Self {
        self.maps.insert(map, Box::new(ReadOnly(owned(pairs))));
        self
    }

    pub(crate) fn refusing(mut self, map: MapType, status: Status) -> Self {
        self.refusals.insert(map, status);
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

    /// The request header pairs of the stream installed in `state`.
    pub(crate) fn pairs_in(state: &mut HostState) -> Vec<(String, String)> {
        state
            .stream_host_as::<Self>()
            .expect("a RecordingStream is installed")
            .pairs(MapType::HttpRequestHeaders)
    }

    pub(crate) fn calls(&self) -> &[(HostCall, MapType)] {
        &self.calls
    }

    /// Takes the stream out of `state`.
    pub(crate) fn take(state: &mut HostState) -> Self {
        let boxed = state
            .take_stream_host()
            .expect("a stream host is installed");
        let any: Box<dyn Any + Send> = boxed;
        *any.downcast::<Self>()
            .expect("the stream host is a RecordingStream")
    }
}

impl StreamHost for RecordingStream {
    fn header_map(&mut self, call: HostCall, map: MapType) -> Result<&mut dyn HeaderMap, Status> {
        self.calls.push((call, map));
        if let Some(status) = self.refusals.get(&map) {
            return Err(*status);
        }
        self.maps
            .get_mut(&map)
            .map(|map| map.as_mut() as &mut dyn HeaderMap)
            .ok_or(Status::BadArgument)
    }
}
