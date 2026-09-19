//! What the guest is doing when it calls a host function.

use crate::abi::v0_2_1::{Callback, ContextId, StreamState};

/// Whether a host function reads or writes the value it asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Access {
    /// The guest reads.
    Read,
    /// The guest writes.
    Write,
}

/// What the guest is doing when it calls a host function.
///
/// The ABI allows each map and each buffer only in named callbacks, and only
/// the crate knows which callback is running.
/// A [`StreamState`] method receives this so that you can apply those rules.
/// The two methods that name a resource the guest can read or write receive
/// an [`Access`] beside it.
/// Build one with [`Invocation::new`] when you test your own stream state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Invocation {
    /// The effective context, which the guest may have changed with
    /// `proxy_set_effective_context`.
    pub context: ContextId,
    /// The callback that is running, or `None` when the guest called from
    /// its start sequence or from a raw call.
    pub callback: Option<Callback>,
}

impl Invocation {
    /// A call on `context` with no callback running.
    pub fn new(context: ContextId) -> Self {
        Self {
            context,
            callback: None,
        }
    }

    /// The callback this call runs inside.
    #[must_use]
    pub fn with_callback(mut self, callback: Callback) -> Self {
        self.callback = Some(callback);
        self
    }
}

/// A stream state that serves nothing, for the callbacks of a root context.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct NoStream;

impl StreamState for NoStream {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_invocation_has_the_context_and_no_callback() {
        // Arrange
        let context = ContextId::try_from(3).unwrap();

        // Act
        let invocation = Invocation::new(context);

        // Assert
        assert_eq!(invocation.context, context);
        assert_eq!(invocation.callback, None);
    }

    #[test]
    fn with_callback_adds_the_callback_and_keeps_the_context() {
        // Arrange
        let context = ContextId::try_from(3).unwrap();

        // Act
        let invocation = Invocation::new(context).with_callback(Callback::Done);

        // Assert
        assert_eq!(invocation.context, context);
        assert_eq!(invocation.callback, Some(Callback::Done));
    }
}
