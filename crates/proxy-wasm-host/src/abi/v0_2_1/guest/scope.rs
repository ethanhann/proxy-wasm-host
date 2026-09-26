//! The scopes through which a guest runs a group of callbacks.

use crate::abi::v0_2_1::AbiAccess;
use crate::abi::v0_2_1::{CallScope, Guest, NoStream, StreamState};

impl Guest {
    /// Runs a group of callbacks and gives the stream state back.
    ///
    /// Sometimes a callback fails in the middle of a group, and you want the
    /// request back with the error.
    /// With [`Guest::enter`], a question mark between `enter` and
    /// [`CallScope::finish`] returns from your function before `finish` runs,
    /// and the request stays on the guest until you call
    /// [`Guest::take_stream`].
    /// With this method, a question mark returns from the closure only, and
    /// the tuple has both the answer of the closure and the request.
    ///
    /// For example, `request` comes back when `on_request_headers` fails:
    ///
    /// ```
    /// # use proxy_wasm_host::abi::v0_2_1::types::Action;
    /// # use proxy_wasm_host::abi::v0_2_1::{ContextId, Guest, GuestError, StreamState};
    /// fn headers<H: StreamState>(
    ///     guest: &mut Guest,
    ///     stream: ContextId,
    ///     request: H,
    /// ) -> (Result<Action, GuestError>, H) {
    ///     guest.with(request, |scope| {
    ///         let action = scope.on_request_headers(stream, 0, false)?;
    ///         scope.on_done(stream)?;
    ///         Ok(action)
    ///     })
    /// }
    /// ```
    pub fn with<H: StreamState, R>(
        &mut self,
        stream: H,
        body: impl FnOnce(&mut CallScope<'_, H>) -> R,
    ) -> (R, H) {
        let mut scope = self.enter(stream);
        let answer = body(&mut scope);
        (answer, scope.finish())
    }

    /// Lends `stream` to the guest for a group of callbacks.
    ///
    /// The guest owns the value until [`CallScope::finish`] returns it or
    /// the scope drops, and the value must be `'static`, because wasmtime
    /// requires that of store data.
    /// Move your request state in and take it back out rather than lending a
    /// borrow.
    /// For the callbacks of a root context, which have no request,
    /// [`Guest::enter_root`] enters with [`NoStream`].
    /// The value replaces any stream state a forgotten scope left installed.
    /// A value that a dropped scope left for [`Guest::take_stream`] is dropped
    /// here, so take it back before you enter again.
    pub fn enter<H: StreamState>(&mut self, stream: H) -> CallScope<'_, H> {
        self.discard_detached();
        self.instance
            .state_mut()
            .abi_mut()
            .set_stream_state(Box::new(stream));
        CallScope::new(self)
    }

    /// A scope with no stream, for the callbacks of a root context.
    ///
    /// [`NoStream`] serves nothing, so a root context that reads a property
    /// or calls a foreign function reports the unavailable status of that
    /// family to the guest.
    /// A callout from a root works in this scope, because your
    /// [`Callouts`](crate::abi::v0_2_1::Callouts) service receives it and no
    /// stream state is asked.
    /// If your root does either, enter the scope with a value of your own
    /// through [`Guest::enter`] instead of this shortcut.
    pub fn enter_root(&mut self) -> CallScope<'_, NoStream> {
        self.enter(NoStream)
    }
}
