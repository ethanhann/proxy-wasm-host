//! What a guest reports about its contexts.

use std::time::Duration;

use crate::abi::v0_2_1::AbiAccess;
use crate::abi::v0_2_1::{Callback, ContextId, ContextState, ContextType, Guest, PluginConfig};

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
    /// The crate records the value and runs no timer, so read it after the
    /// callbacks of a root and drive `proxy_on_tick` from your own timer.
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
}
