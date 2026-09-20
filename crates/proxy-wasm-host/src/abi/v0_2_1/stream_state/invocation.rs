//! What the guest is doing when it calls a host function.

use crate::abi::v0_2_1::{Callback, CalloutId, ContextId, GuestId, StreamState};

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
    /// The guest that is running.
    ///
    /// The contexts and the callouts of every guest start at one, so a
    /// service that serves two guests keys its own record by this value and
    /// the callout.
    pub guest: GuestId,
    /// The effective context, which the guest may have changed with
    /// `proxy_set_effective_context`.
    pub context: ContextId,
    /// The callback that is running, or `None` when the guest called from
    /// its start sequence or from a raw call.
    pub callback: Option<Callback>,
    /// The callout whose result the running callback delivers, or `None`
    /// outside a delivery.
    pub callout: Option<CalloutId>,
}

impl Invocation {
    /// A call of `guest` on `context` with no callback running.
    ///
    /// [`Guest::id`](crate::abi::v0_2_1::Guest::id) gives you the identity of
    /// a guest you built, and [`GuestId::next`] gives you one where no guest
    /// runs.
    pub fn new(guest: GuestId, context: ContextId) -> Self {
        Self {
            guest,
            context,
            callback: None,
            callout: None,
        }
    }

    /// The callback this call runs inside.
    #[must_use]
    pub fn with_callback(mut self, callback: Callback) -> Self {
        self.callback = Some(callback);
        self
    }

    /// The callout whose result the callback delivers.
    #[must_use]
    pub fn with_callout(mut self, callout: CalloutId) -> Self {
        self.callout = Some(callout);
        self
    }
}

/// A stream state that serves nothing, for the callbacks of a root context.
///
/// A callout that a root context makes needs no stream state, because the
/// [`Callouts`](crate::abi::v0_2_1::Callouts) service of the guest receives
/// it.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct NoStream;

impl StreamState for NoStream {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_invocation_has_the_guest_the_context_and_no_callback() {
        // Arrange
        let context = ContextId::try_from(3).unwrap();
        let guest = GuestId::next();

        // Act
        let invocation = Invocation::new(guest, context);

        // Assert
        assert_eq!(invocation.guest, guest);
        assert_eq!(invocation.context, context);
        assert_eq!(invocation.callback, None);
        assert_eq!(invocation.callout, None);
    }

    #[test]
    fn with_callback_adds_the_callback_and_keeps_the_context() {
        // Arrange
        let context = ContextId::try_from(3).unwrap();
        let guest = GuestId::next();

        // Act
        let invocation = Invocation::new(guest, context).with_callback(Callback::Done);

        // Assert
        assert_eq!(invocation.context, context);
        assert_eq!(invocation.callback, Some(Callback::Done));
    }
}
