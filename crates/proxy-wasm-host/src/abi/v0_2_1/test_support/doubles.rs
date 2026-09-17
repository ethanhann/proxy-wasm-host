//! The header maps and buffers that the stream host double lends.

use std::borrow::Cow;
use std::ops::ControlFlow;
use std::sync::{Arc, Mutex};

use crate::buffer::clamp_range;
use crate::codec::pairs::PairVisitor;
use crate::header_map::{HeaderMap, VecHeaderMap};
use crate::{Buffer, NotAllowed};

/// A header map that refuses every write.
pub(crate) struct ReadOnlyMap(pub(crate) VecHeaderMap);

impl HeaderMap for ReadOnlyMap {
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

/// A buffer that refuses every write.
pub(crate) struct ReadOnlyBuffer(pub(crate) Vec<u8>);

impl Buffer for ReadOnlyBuffer {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn copy_range_into(&self, start: usize, max_size: usize, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.0[clamp_range(self.0.len(), start, max_size)]);
    }

    fn replace(&mut self, _: usize, _: usize, _: &[u8]) -> Result<(), NotAllowed> {
        Err(NotAllowed)
    }
}

/// The ranges a buffer was asked for, shared with the stream that owns it.
pub(crate) type Ranges = Arc<Mutex<Vec<(usize, usize)>>>;

/// A buffer that records every range a host function asked it for, and
/// indexes without clamping.
///
/// The crate promises an implementation that the range is inside the buffer,
/// so a range it did not clamp panics here instead of passing.
pub(crate) struct RecordingBuffer {
    bytes: Vec<u8>,
    ranges: Ranges,
}

impl RecordingBuffer {
    pub(crate) fn new(bytes: &[u8], ranges: &Ranges) -> Self {
        Self {
            bytes: bytes.to_vec(),
            ranges: Arc::clone(ranges),
        }
    }
}

impl Buffer for RecordingBuffer {
    fn len(&self) -> usize {
        self.bytes.len()
    }

    fn copy_range_into(&self, start: usize, max_size: usize, out: &mut Vec<u8>) {
        self.ranges.lock().unwrap().push((start, max_size));
        out.extend_from_slice(&self.bytes[start..start + max_size]);
    }

    fn replace(&mut self, start: usize, size: usize, value: &[u8]) -> Result<(), NotAllowed> {
        self.ranges.lock().unwrap().push((start, size));
        self.bytes
            .splice(start..start + size, value.iter().copied());
        Ok(())
    }
}

pub(crate) fn owned(pairs: &[(&str, &str)]) -> VecHeaderMap {
    pairs
        .iter()
        .map(|(key, value)| (key.as_bytes().to_vec(), value.as_bytes().to_vec()))
        .collect()
}
