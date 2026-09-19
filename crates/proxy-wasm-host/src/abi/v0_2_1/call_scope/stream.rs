//! The callbacks of a stream context.

use crate::abi::v0_2_1::call_scope::{CallScope, prologue};
use crate::abi::v0_2_1::types::Action;
use crate::abi::v0_2_1::{Callback, ContextId, GuestError, StreamState};

impl<H: StreamState> CallScope<'_, H> {
    /// Calls `proxy_on_request_headers` on a stream context.
    ///
    /// # Errors
    ///
    /// Returns [`GuestError::Context`] for an unknown context or a root
    /// context, [`GuestError::GuestRejected`], and
    /// [`GuestError::UnexpectedReturn`].
    /// Returns [`GuestError::Runtime`] with
    /// [`Error::ValueTooLarge`](crate::Error::ValueTooLarge) for a count
    /// above `i32::MAX`, and the
    /// [common runtime failures](CallScope#the-common-runtime-failures).
    pub fn on_request_headers(
        &mut self,
        context: ContextId,
        num_headers: u32,
        end_of_stream: bool,
    ) -> Result<Action, GuestError> {
        self.guest.require_live()?;
        prologue::require_stream(self.guest, context)?;
        prologue::accepted(self.guest, context)?;
        let count = prologue::wire_u32(num_headers)?;
        let callback = Callback::RequestHeaders;
        let func = self.guest.callbacks().request_headers.clone();
        let params = (context.wire(), count, i32::from(end_of_stream));
        let default = i32::from(Action::Continue);
        let value = prologue::run(self.guest, context, callback, func, params, default)?;
        Action::try_from(value).map_err(|_| GuestError::UnexpectedReturn { callback, value })
    }
}
