//! The callbacks that end a context.

use crate::Error;
use crate::abi::v0_2_1::call_scope::{CallScope, prologue};
use crate::abi::v0_2_1::{Callback, ContextId, ContextState, StreamHost};

impl<H: StreamHost> CallScope<'_, H> {
    /// Calls `proxy_on_done`.
    ///
    /// `true` marks the context done.
    /// `false` marks it pending, and the guest calls `proxy_done` later, which
    /// [`Guest::context_state`](super::Guest::context_state) shows as `Done`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Poisoned`], [`Error::Context`] for an unknown
    /// context, [`Error::UnexpectedReturn`], and the errors of a guest call.
    pub fn on_done(&mut self, context: ContextId) -> Result<bool, Error> {
        prologue::live(self.guest)?;
        prologue::require(self.guest, context)?;
        let func = self.guest.callbacks().done.clone();
        let value = prologue::run(self.guest, context, Callback::Done, func, context.wire(), 1)?;
        let done = prologue::boolean(Callback::Done, value)?;
        let state = if done {
            ContextState::Done
        } else {
            ContextState::Pending
        };
        self.guest
            .instance_mut()
            .state_mut()
            .abi_mut()
            .contexts_mut()
            .set_state(context, state);
        Ok(done)
    }

    /// Calls `proxy_on_log` on a done context.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Poisoned`], [`Error::Context`] for an unknown
    /// context or one that is not done, and the errors of a guest call.
    pub fn on_log(&mut self, context: ContextId) -> Result<(), Error> {
        prologue::live(self.guest)?;
        prologue::require_done(self.guest, context)?;
        let func = self.guest.callbacks().log.clone();
        prologue::run(self.guest, context, Callback::Log, func, context.wire(), ())
    }

    /// Calls `proxy_on_delete` on a done context and forgets the context.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Poisoned`], [`Error::Context`] for an unknown
    /// context, one that is not done, or a root context that still has
    /// stream contexts, and the errors of a guest call.
    pub fn on_delete(&mut self, context: ContextId) -> Result<(), Error> {
        prologue::live(self.guest)?;
        prologue::require_deletable(self.guest, context)?;
        let func = self.guest.callbacks().delete.clone();
        prologue::run(
            self.guest,
            context,
            Callback::Delete,
            func,
            context.wire(),
            (),
        )?;
        self.guest
            .instance_mut()
            .state_mut()
            .abi_mut()
            .contexts_mut()
            .remove(context);
        Ok(())
    }
}
