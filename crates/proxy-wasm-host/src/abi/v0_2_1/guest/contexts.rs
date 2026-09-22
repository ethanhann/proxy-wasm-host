//! What a guest reports about its contexts.

use std::time::Duration;

use crate::abi::v0_2_1::AbiAccess;
use crate::abi::v0_2_1::{
    Callback, CalloutId, Changes, ContextId, ContextProblem, ContextState, ContextType, Guest,
    GuestError, OpenCallout, PluginConfig, QueueId, StreamKind,
};

impl Guest {
    /// The callback that refused `context`, if one did.
    ///
    /// `VmStart` means the whole instance is refused.
    /// `Configure` means the root context of `context` is refused, with
    /// every stream context under it.
    pub fn rejected_by(&self, context: ContextId) -> Option<Callback> {
        self.instance.state().abi().contexts().rejection_of(context)
    }

    /// How far `context` is through its finalization, or `None` for a
    /// context this guest does not hold.
    ///
    /// A move from `Pending` to `Done` without a callback of yours means
    /// the guest called `proxy_done`.
    pub fn context_state(&self, context: ContextId) -> Option<ContextState> {
        self.instance.state().abi().contexts().state(context)
    }

    /// The family of stream `context` serves.
    ///
    /// A stream context takes the callbacks of one family, and the crate
    /// records the family at the first callback that reaches the guest.
    /// The answer is `None` for a root context, for a context this guest
    /// does not hold, and for a stream context that took no stream callback
    /// and got no declaration.
    /// [`Guest::expect_stream_kind`] writes the record before the first
    /// callback.
    pub fn context_stream_kind(&self, context: ContextId) -> Option<StreamKind> {
        self.instance.state().abi().contexts().stream_kind(context)
    }

    /// Declares the family of stream that `context` serves.
    ///
    /// A guest chooses the family of a stream context inside its own SDK,
    /// and both SDKs stop with a panic when a callback of the other family
    /// reaches them.
    /// The crate cannot read that choice, so a first callback of the wrong
    /// family poisons the guest.
    /// Call this after you create a stream context, and a callback of the
    /// other family is then refused from the first one.
    ///
    /// A guest you build after a trap starts with no record, so declare the
    /// family again for each context of the new guest.
    ///
    /// # Errors
    ///
    /// Returns [`GuestError::Context`] with
    /// [`ContextProblem::Unknown`](crate::abi::v0_2_1::ContextProblem) for a
    /// context this guest does not hold, with `NotStream` for a root
    /// context, and with `WrongStreamKind` for a context that already serves
    /// the other family.
    /// A declaration that repeats the family of the context answers `Ok`.
    pub fn expect_stream_kind(
        &mut self,
        context: ContextId,
        kind: StreamKind,
    ) -> Result<(), GuestError> {
        let problem = |problem| GuestError::Context {
            id: context,
            problem,
        };
        match self.context_type(context) {
            None => return Err(problem(ContextProblem::Unknown)),
            Some(ContextType::Root) => return Err(problem(ContextProblem::NotStream)),
            Some(ContextType::Stream) => {}
        }
        let had = self
            .instance
            .state_mut()
            .abi_mut()
            .contexts_mut()
            .set_stream_kind(context, kind);
        match had {
            Some(recorded) if recorded != kind => {
                self.instance
                    .state_mut()
                    .abi_mut()
                    .contexts_mut()
                    .set_stream_kind(context, recorded);
                Err(problem(ContextProblem::WrongStreamKind {
                    recorded,
                    attempted: kind,
                }))
            }
            _ => Ok(()),
        }
    }

    /// Whether `context` is a root context or a stream context.
    pub fn context_type(&self, context: ContextId) -> Option<ContextType> {
        self.instance.state().abi().contexts().context_type(context)
    }

    /// The root context of a stream context, or `None` for a root context
    /// and for a context this guest does not hold.
    pub fn context_parent(&self, context: ContextId) -> Option<ContextId> {
        self.instance.state().abi().contexts().parent(context)
    }

    /// The plugin of the root context of `context`.
    ///
    /// [`CallScope::on_configure`](crate::abi::v0_2_1::CallScope::on_configure) records it, so this is `None` until that
    /// callback has run on the root.
    pub fn plugin(&self, context: ContextId) -> Option<&PluginConfig> {
        self.instance.state().abi().contexts().plugin(context)
    }

    /// The tick period the guest asked for on `root`.
    ///
    /// A guest sets it with `proxy_set_tick_period_milliseconds`, and a
    /// period of zero clears it.
    /// A guest can only reach the root it is serving, so a period it sets in
    /// any callback lands on that root.
    /// The crate records the value and runs no timer.
    /// [`Guest::take_changes`] tells you when a root set one, and you call
    /// [`CallScope::on_tick`](crate::abi::v0_2_1::CallScope::on_tick) from a
    /// timer of your own.
    pub fn tick_period(&self, root: ContextId) -> Option<Duration> {
        self.instance.state().abi().contexts().tick_period(root)
    }

    /// The context the guest's host functions act on.
    ///
    /// Every callback sets it to its own context, and the guest can change
    /// it with `proxy_set_effective_context`.
    pub fn effective_context(&self) -> Option<ContextId> {
        self.instance.state().abi().contexts().effective()
    }

    /// Every callout the guest has open, in identifier order.
    ///
    /// This answers on a poisoned guest, so when a guest traps you can end
    /// the requests that wait for one of its callouts.
    /// A gRPC stream of a poisoned guest is still running at your side, and
    /// the crate ends none of them, so cancel each one before you build a
    /// new guest.
    /// A callout whose delivery trapped is in the list, because the guest
    /// did not complete it.
    pub fn open_callouts(&self) -> Vec<OpenCallout> {
        let callouts = self.instance.state().abi().callouts();
        callouts
            .iter()
            .map(|(id, entry)| entry.report(id))
            .collect()
    }

    /// One open callout, or `None` when `callout` is not open.
    ///
    /// When a result arrives, this tells you the context to name in
    /// [`CallScope::on_http_call_response`](crate::abi::v0_2_1::CallScope::on_http_call_response)
    /// or in a gRPC delivery, the kind of the callout, and whether the
    /// caller is a root, so you need no record of your own.
    /// `None` for a late response is the usual result after the context of
    /// the request was deleted.
    pub fn open_callout(&self, callout: CalloutId) -> Option<OpenCallout> {
        let callouts = self.instance.state().abi().callouts();
        callouts.get(callout).map(|entry| entry.report(callout))
    }

    /// How many callouts the guest has open.
    ///
    /// Compare it with
    /// [`VmServices::max_open_callouts`](crate::abi::v0_2_1::VmServices::max_open_callouts)
    /// to see how near the guest is to the maximum.
    pub fn open_callout_count(&self) -> usize {
        self.instance.state().abi().callouts().len()
    }

    /// The roots of this guest whose contexts registered `queue`.
    ///
    /// A guest that resolved a queue did not register it.
    /// The set is empty after you replace the shared services, because a
    /// queue identifier belongs to the store that gave it.
    pub fn queue_registrants(&self, queue: QueueId) -> Vec<ContextId> {
        self.instance.state().abi().registrants(queue)
    }

    /// What the guest changed since you last asked, which this call empties.
    ///
    /// Read it after a group of callbacks to learn that a root set a tick
    /// period or registered a queue.
    pub fn take_changes(&mut self) -> Changes {
        self.instance.state_mut().abi_mut().take_changes()
    }
}
