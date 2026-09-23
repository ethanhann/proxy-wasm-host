//! What a guest reports to the spec that built it.
//!
//! An embedder knows that it called `build`. It does not know that the build
//! followed a trap, because the crate holds that fact, so a guest records it
//! here when it is dropped.

use std::sync::Arc;

use crate::abi::v0_2_1::Guest;
use crate::abi::v0_2_1::guest_spec::BuildCounters;

impl Guest {
    /// Counts this guest in the spec that built it.
    pub(crate) fn count_on(&mut self, counters: Arc<BuildCounters>) {
        self.counters = Some(counters);
    }
}

impl Drop for Guest {
    fn drop(&mut self) {
        if self.instance.is_poisoned()
            && let Some(counters) = &self.counters
        {
            counters.record_poison();
        }
    }
}
