//! The value a log sink receives, built from the state of a call.

use crate::abi::v0_2_1::AbiAccess;
use crate::abi::v0_2_1::payload::Delivery;
use crate::abi::v0_2_1::{Invocation, LogContext};
use crate::runtime::HostState;

/// Where one log line came from.
///
/// The call is read from the running callback and not from the context
/// table, because the table keeps the last effective context after a
/// callback returns.
/// The plugin values are read from the root of that context, which has a
/// plugin only after its configuration ran.
pub(crate) fn log_context(state: &HostState) -> LogContext<'_> {
    let abi = state.abi();
    let mut context = LogContext::new(abi.services().vm_id(), abi.guest());
    let Some(callback) = abi.current_callback() else {
        return context;
    };
    let Some(effective) = abi.contexts().effective() else {
        return context;
    };
    let mut call = Invocation::new(abi.guest(), effective).with_callback(callback);
    if let Some(callout) = abi.delivery().and_then(Delivery::callout) {
        call = call.with_callout(callout);
    }
    context = context.with_call(call);
    if let Some(plugin) = abi
        .contexts()
        .root_of(effective)
        .and_then(|root| abi.contexts().plugin(root))
    {
        context = context.with_plugin(plugin.name(), plugin.root_id());
    }
    context
}
