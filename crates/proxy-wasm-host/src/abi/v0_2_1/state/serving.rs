//! What the ABI state lends to a guest for the time of one callback.

use crate::abi::v0_2_1::AbiState;
use crate::abi::v0_2_1::payload::Delivery;
use crate::abi::v0_2_1::types::BufferType;

impl AbiState {
    /// Records the buffer and the size that the running data callback
    /// announced, or clears the record.
    ///
    /// A guest that reads that buffer inside the callback gets what the
    /// stream state holds, and the crate reports a length that differs.
    pub(crate) fn set_announced(&mut self, announced: Option<(BufferType, u32)>) {
        self.announced = announced;
    }

    /// The size the running data callback announced for `buffer`, which the
    /// first read of that buffer takes.
    ///
    /// The record is taken rather than read, so one callback reports at most
    /// one difference however many times the guest reads.
    pub(crate) fn take_announced(&mut self, buffer: BufferType) -> Option<u32> {
        let (kind, size) = self.announced?;
        if kind != buffer {
            return None;
        }
        self.announced = None;
        Some(size)
    }

    /// The result the running callback delivers, which is `None` outside a
    /// delivery.
    pub(crate) fn delivery(&self) -> Option<&Delivery> {
        self.delivery.as_ref()
    }

    /// The delivered result as a header map read needs it.
    pub(crate) fn delivery_mut(&mut self) -> Option<&mut Delivery> {
        self.delivery.as_mut()
    }

    /// Installs the result of a delivery, or clears it with `None`.
    pub(crate) fn set_delivery(&mut self, delivery: Option<Delivery>) {
        self.delivery = delivery;
    }
}
