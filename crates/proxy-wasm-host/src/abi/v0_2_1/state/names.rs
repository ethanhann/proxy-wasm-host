//! The names a guest obtained identifiers for, and the start of its VM.

use crate::abi::v0_2_1::state::AbiState;

/// A name a guest sent to obtain a queue or a metric identifier.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum SharedName {
    Registered(Vec<u8>),
    Resolved(Vec<u8>, Vec<u8>),
    Metric(i32, Vec<u8>),
}

impl AbiState {
    /// Whether this guest already obtained an identifier for `name`.
    ///
    /// A name the guest holds costs no share of the shared name limit, so
    /// the limit is checked only for a name that is new to the guest.
    pub(crate) fn holds_shared_name(&self, name: &SharedName) -> bool {
        self.held_names.contains(name)
    }

    /// Records that this guest obtained an identifier for `name`.
    pub(crate) fn hold_shared_name(&mut self, name: SharedName) {
        self.held_names.insert(name);
    }

    /// Whether `proxy_on_vm_start` already ran and was accepted.
    pub(crate) fn vm_started(&self) -> bool {
        self.vm_started
    }

    /// Records that `proxy_on_vm_start` ran and was accepted.
    pub(crate) fn mark_vm_started(&mut self) {
        self.vm_started = true;
    }
}
