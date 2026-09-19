//! The callbacks that end a context.

use crate::Error;
use crate::abi::v0_2_1::AbiAccess;
use crate::abi::v0_2_1::call_scope::{CallScope, prologue};
use crate::abi::v0_2_1::{Callback, ContextId, ContextState, NoStream, StreamState};

impl<H: StreamState> CallScope<'_, H> {
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
        self.guest.require_live()?;
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
        self.guest.require_live()?;
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
        self.guest.require_live()?;
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

impl<H: StreamState> Drop for CallScope<'_, H> {
    fn drop(&mut self) {
        let held = self
            .guest
            .instance_mut()
            .state_mut()
            .abi_mut()
            .take_stream_state();
        // A root scope is routinely dropped rather than finished, and
        // `NoStream` holds nothing to read back, so detaching it would report
        // on the ordinary lifecycle.
        if let Some(held) = held.filter(|held| !is_no_stream(held.as_ref())) {
            self.guest.detach(held);
        }
        let state = self.guest.instance_mut().state_mut();
        if state.abi_mut().current_callback().is_some() {
            state.poison();
            state.abi_mut().set_current_callback(None);
        }
    }
}

fn is_no_stream(stream: &dyn StreamState) -> bool {
    let any: &dyn std::any::Any = stream;
    any.is::<NoStream>()
}
