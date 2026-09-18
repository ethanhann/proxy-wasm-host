//! The data stored in every instance's wasmtime store.

use std::any::Any;

use wasmtime::{Memory, StoreLimits, TypedFunc};

use crate::runtime::VmServices;

/// The store data of one instance.
///
/// The runtime keeps the cached memory handle, the guest allocator, the store
/// limits, and the poison flag here, next to the services the embedder
/// supplied.
/// Everything the ABI layer keeps is in one opaque slot, which the ABI layer
/// fills and only the ABI layer reads inside.
/// The type is crate private, so nothing outside the crate can clear the
/// poison flag or replace the cached handles.
pub(crate) struct HostState {
    services: VmServices,
    store_limits: StoreLimits,
    memory: Option<Memory>,
    allocator: Option<TypedFunc<i32, i32>>,
    poisoned: bool,
    abi: Box<dyn Any + Send>,
}

impl HostState {
    pub(crate) fn new(services: VmServices, abi: Box<dyn Any + Send>) -> Self {
        Self {
            services,
            store_limits: StoreLimits::default(),
            memory: None,
            allocator: None,
            poisoned: false,
            abi,
        }
    }

    pub(crate) fn services(&self) -> &VmServices {
        &self.services
    }

    pub(crate) fn services_mut(&mut self) -> &mut VmServices {
        &mut self.services
    }

    pub(crate) fn is_poisoned(&self) -> bool {
        self.poisoned
    }

    pub(crate) fn memory(&self) -> Option<Memory> {
        self.memory
    }

    pub(crate) fn allocator(&self) -> Option<TypedFunc<i32, i32>> {
        self.allocator.clone()
    }

    pub(crate) fn store_limits(&mut self) -> &mut StoreLimits {
        &mut self.store_limits
    }

    pub(crate) fn set_memory(&mut self, memory: Memory) {
        self.memory = Some(memory);
    }

    pub(crate) fn set_allocator(&mut self, allocator: TypedFunc<i32, i32>) {
        self.allocator = Some(allocator);
    }

    pub(crate) fn set_store_limits(&mut self, limits: StoreLimits) {
        self.store_limits = limits;
    }

    pub(crate) fn poison(&mut self) {
        self.poisoned = true;
    }

    /// The slot the ABI layer filled, which only that layer reads inside.
    pub(crate) fn abi_slot(&self) -> &(dyn Any + Send) {
        self.abi.as_ref()
    }

    /// The slot the ABI layer filled, which only that layer reads inside.
    pub(crate) fn abi_slot_mut(&mut self) -> &mut (dyn Any + Send) {
        self.abi.as_mut()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::runtime::test_support::RecordingSink;

    #[test]
    fn poison_is_observable() {
        // Arrange
        let mut state = HostState::new(
            VmServices::new(Arc::new(RecordingSink::default())),
            crate::abi::state(),
        );

        // Act
        state.poison();

        // Assert
        assert!(state.is_poisoned());
    }

    #[test]
    fn a_new_state_holds_nothing_and_is_not_poisoned() {
        // Arrange
        let services = VmServices::new(Arc::new(RecordingSink::default()));

        // Act
        let state = HostState::new(services, crate::abi::state());

        // Assert
        assert!(state.memory().is_none());
        assert!(state.allocator().is_none());
        assert!(!state.is_poisoned());
    }
}
