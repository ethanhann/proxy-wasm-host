//! The stream state a scope left behind.
//!
//! A scope that drops without a finish leaves its value on the guest rather
//! than in the store, so no host function can reach it and an embedder that
//! returned early can still read what it lent.

#[allow(unused_imports)]
use crate::abi::v0_2_1::CallScope;
use crate::abi::v0_2_1::{Guest, StreamState};

impl Guest {
    /// Keeps the stream state a scope left behind.
    pub(crate) fn detach(&mut self, stream: Box<dyn StreamState>) {
        if self.detached.is_some() {
            tracing::warn!("a stream state left by an earlier scope was replaced");
        }
        self.detached = Some(stream);
    }

    /// The stream state a scope left behind, as the type it was entered with.
    ///
    /// A scope that drops without [`CallScope::finish`] leaves its value
    /// here, so an embedder that returned early through the question mark
    /// operator can still read the request it lent.
    /// A value stays until the next [`Guest::enter`] or until this guest
    /// drops.
    ///
    /// A type that does not match leaves the value where it is, so a later
    /// call with the right type still finds it.
    /// [`Guest::take_stream_any`] reads it without naming the type.
    pub fn take_stream<H: StreamState>(&mut self) -> Option<H> {
        let held: &dyn std::any::Any = self.detached.as_deref()?;
        if !held.is::<H>() {
            tracing::warn!(
                expected = std::any::type_name::<H>(),
                "the detached stream state is another type, and it is left where it is"
            );
            return None;
        }
        let boxed: Box<dyn std::any::Any + Send> = self.detached.take()?;
        boxed.downcast::<H>().ok().map(|value| *value)
    }

    /// The stream state a scope left behind, without naming its type.
    pub fn take_stream_any(&mut self) -> Option<Box<dyn StreamState>> {
        self.detached.take()
    }
}
