//! The data stored in every instance's wasmtime store.

use std::any::Any;

use wasmtime::{Memory, StoreLimits, TypedFunc};

use crate::codec::pairs::PairLimits;

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
    store_limits: StoreLimits,
    pair_limits: PairLimits,
    memory: Option<Memory>,
    allocator: Option<TypedFunc<i32, i32>>,
    poisoned: bool,
    abi: Box<dyn Any + Send>,
}

impl HostState {
    pub(crate) fn new(abi: Box<dyn Any + Send>) -> Self {
        Self {
            store_limits: StoreLimits::default(),
            pair_limits: PairLimits::default(),
            memory: None,
            allocator: None,
            poisoned: false,
            abi,
        }
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

    /// What one map a guest sends may hold.
    pub(crate) fn pair_limits(&self) -> PairLimits {
        self.pair_limits
    }

    pub(crate) fn set_pair_limits(&mut self, pair_limits: PairLimits) {
        self.pair_limits = pair_limits;
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
    use super::*;

    #[test]
    fn poison_is_observable() {
        // Arrange
        let mut state = HostState::new(Box::new(()));

        // Act
        state.poison();

        // Assert
        assert!(state.is_poisoned());
    }

    #[test]
    fn a_new_state_holds_nothing_and_is_not_poisoned() {
        // Arrange
        let abi = Box::new(());

        // Act
        let state = HostState::new(abi);

        // Assert
        assert!(state.memory().is_none());
        assert!(state.allocator().is_none());
        assert!(!state.is_poisoned());
    }
}
