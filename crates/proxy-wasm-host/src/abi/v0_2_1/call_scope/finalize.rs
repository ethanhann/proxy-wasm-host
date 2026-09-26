//! The callbacks that end a context.

use crate::abi::v0_2_1::AbiAccess;
use crate::abi::v0_2_1::GuestError;
use crate::abi::v0_2_1::call_scope::delivery::{deliver_grpc_close, deliver_http_response};
use crate::abi::v0_2_1::call_scope::{CallScope, prologue};
use crate::abi::v0_2_1::{
    Callback, CalloutId, CalloutKind, ContextId, ContextState, GrpcStatus, Guest, HttpCallResponse,
    NoStream, StreamState,
};

impl<H: StreamState> CallScope<'_, H> {
    /// Calls `proxy_on_done`.
    ///
    /// `true` marks the context done.
    /// `false` marks it pending, and the guest calls `proxy_done` later, which
    /// [`Guest::context_state`](super::Guest::context_state) shows as `Done`.
    ///
    /// # Errors
    ///
    /// Returns [`GuestError::Context`] for an unknown context,
    /// [`GuestError::UnexpectedReturn`], and the
    /// [common runtime failures](CallScope#the-common-runtime-failures).
    pub fn on_done(&mut self, context: ContextId) -> Result<bool, GuestError> {
        self.guest.require_live()?;
        prologue::require(self.guest, context)?;
        let value = prologue::run(
            self.guest,
            context,
            Callback::Done,
            |callbacks| callbacks.done.as_ref(),
            context.wire(),
            1,
        )?;
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
    /// Returns [`GuestError::Context`] for an unknown context or one that is
    /// not done, and the
    /// [common runtime failures](CallScope#the-common-runtime-failures).
    pub fn on_log(&mut self, context: ContextId) -> Result<(), GuestError> {
        self.guest.require_live()?;
        prologue::require_done(self.guest, context)?;
        prologue::run(
            self.guest,
            context,
            Callback::Log,
            |callbacks| callbacks.log.as_ref(),
            context.wire(),
            (),
        )?;
        Ok(())
    }

    /// Calls `proxy_on_delete` on a done context and forgets the context.
    ///
    /// A callout that the context still has open ends here, and the answer
    /// holds the identifier of each one, so that you can end your own record
    /// of the request you sent for it.
    /// The crate calls no method of your
    /// [`Callouts`](crate::abi::v0_2_1::Callouts) service for them, so the
    /// answer is the one signal you get.
    /// The crate delivers a failure for each one in identifier order, so
    /// that the guest drops its own record of the callout.
    /// An HTTP call gets `proxy_on_http_call_response` with three counts of
    /// zero, and a gRPC callout gets `proxy_on_grpc_close` with the code
    /// one, which gRPC names `CANCELLED`.
    /// A callout that the guest ends from inside one of those callbacks gets
    /// no delivery and is not in the answer.
    /// The context opens no new callout while this runs, and a callout
    /// function of the guest answers `INTERNAL_FAILURE`.
    /// For a stream context the failures come after `proxy_on_delete`, where
    /// a guest SDK finds no context and runs none of your plugin's code.
    /// For a root context they come before it.
    /// If you want the guest to handle the end of a callout while its stream
    /// is alive, deliver [`HttpCallResponse::failed`](crate::abi::v0_2_1::HttpCallResponse::failed)
    /// before you call this.
    ///
    /// # Errors
    ///
    /// Returns [`GuestError::Context`] for an unknown context, one that is
    /// not done, or a root context that still has stream contexts, and the
    /// [common runtime failures](CallScope#the-common-runtime-failures).
    /// After a failure, [`Guest::open_callouts`](crate::abi::v0_2_1::Guest::open_callouts)
    /// tells you the callouts that did not end, which includes those of the
    /// context you deleted.
    /// The guest is poisoned in that case, so you end your own request for
    /// each one and build a new guest.
    pub fn on_delete(&mut self, context: ContextId) -> Result<Vec<CalloutId>, GuestError> {
        self.guest.require_live()?;
        prologue::require_deletable(self.guest, context)?;
        self.mark_deleting(Some(context));
        let ended = self.delete(context);
        self.mark_deleting(None);
        ended
    }

    fn mark_deleting(&mut self, context: Option<ContextId>) {
        self.guest
            .instance_mut()
            .state_mut()
            .abi_mut()
            .set_deleting(context);
    }

    fn delete(&mut self, context: ContextId) -> Result<Vec<CalloutId>, GuestError> {
        let root = self.guest.context_parent(context);
        let mut ended = match root {
            None => fail_open_callouts(self.guest, context, context)?,
            Some(_) => Vec::new(),
        };
        prologue::run(
            self.guest,
            context,
            Callback::Delete,
            |callbacks| callbacks.delete.as_ref(),
            context.wire(),
            (),
        )?;
        let abi = self.guest.instance_mut().state_mut().abi_mut();
        abi.contexts_mut().remove(context);
        if root.is_none() {
            abi.forget_registrant(context);
        }
        if let Some(root) = root {
            ended.extend(fail_open_callouts(self.guest, context, root)?);
        }
        ended.extend(drop_open_callouts(self.guest, context));
        Ok(ended)
    }
}

/// The gRPC status code of a callout that the crate ends by itself, which
/// gRPC names `CANCELLED`.
pub(super) const CANCELLED: u32 = 1;

/// Delivers a failure for every callout `context` still has open, in
/// identifier order, on `root`.
///
/// A guest SDK drops its own record of a callout only on a delivery, so a
/// callout that ended in silence would stay in the guest for its life.
/// A refused root gets no guest call, and its entries are removed.
/// The table is read again before each delivery, because a guest can end a
/// callout from inside one of these callbacks.
/// The answer holds the identifiers of the callouts that ended.
pub(super) fn fail_open_callouts(
    guest: &mut Guest,
    context: ContextId,
    root: ContextId,
) -> Result<Vec<CalloutId>, GuestError> {
    let open = guest.instance().state().abi().callouts().made_by(context);
    if guest.rejected_by(root).is_some() {
        return Ok(drop_open_callouts(guest, context));
    }
    let mut ended = Vec::new();
    for (callout, kind) in open {
        if guest
            .instance()
            .state()
            .abi()
            .callouts()
            .get(callout)
            .is_none()
        {
            continue;
        }
        match kind {
            CalloutKind::HttpCall => {
                deliver_http_response(guest, root, callout, HttpCallResponse::failed())?;
            }
            CalloutKind::GrpcCall | CalloutKind::GrpcStream => {
                deliver_grpc_close(guest, root, callout, GrpcStatus::new(CANCELLED, ""))?;
            }
        }
        ended.push(callout);
    }
    Ok(ended)
}

/// Removes every callout `context` has open, with no guest call, and answers
/// their identifiers.
///
/// A deleted context can get no delivery, so a callout that the guest made
/// from a failure delivery of that deletion ends here.
pub(super) fn drop_open_callouts(guest: &mut Guest, context: ContextId) -> Vec<CalloutId> {
    let abi = guest.instance_mut().state_mut().abi_mut();
    let open = abi.callouts().made_by(context);
    for (callout, _) in &open {
        abi.callouts_mut().remove(*callout);
    }
    open.into_iter().map(|(callout, _)| callout).collect()
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
