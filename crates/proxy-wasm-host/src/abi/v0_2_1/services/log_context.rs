//! Where one log line came from.

use std::borrow::Cow;

use crate::abi::v0_2_1::{GuestId, Invocation};

/// Where one log line came from.
///
/// One process runs many guests, one guest runs many roots, and one root
/// serves many requests, so a line on its own does not say who wrote it.
/// [`LogSink::log`](crate::abi::v0_2_1::LogSink::log) receives this beside
/// the level and the message.
/// You read the fields your own log format prints.
///
/// For example, a sink that writes one line for each message:
///
/// ```
/// use proxy_wasm_host::abi::v0_2_1::types::LogLevel;
/// use proxy_wasm_host::abi::v0_2_1::{LogContext, LogSink};
///
/// struct Lines;
///
/// impl LogSink for Lines {
///     fn log(&self, context: LogContext<'_>, level: LogLevel, message: &[u8]) {
///         let plugin = context.plugin_name.unwrap_or(std::borrow::Cow::Borrowed(b"<unconfigured>"));
///         println!(
///             "{level:?} [{}] {}",
///             String::from_utf8_lossy(&plugin),
///             String::from_utf8_lossy(message)
///         );
///     }
/// }
/// ```
///
/// The crate builds the value with borrowed bytes, so it lives until the call
/// that wrote the line returns.
/// A sink that keeps its lines calls [`LogContext::into_owned`] first.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct LogContext<'a> {
    /// The VM id you gave
    /// [`VmServices::with_vm_id`](crate::abi::v0_2_1::VmServices::with_vm_id).
    ///
    /// Guests that share a VM id share their shared data, their queues, and
    /// their metrics.
    pub vm_id: Cow<'a, [u8]>,
    /// The guest that wrote the line.
    ///
    /// Two guests built from one plugin differ here and nowhere else.
    pub guest: GuestId,
    /// The name of the plugin of the root the line belongs to.
    ///
    /// A root learns its plugin when its configuration runs, so a line from
    /// `proxy_on_vm_start` or from the creation of a root has none.
    /// A line written when no callback is running has none either, because
    /// the crate reads the plugin through the call.
    pub plugin_name: Option<Cow<'a, [u8]>>,
    /// The root id of that same plugin, which is absent in the same cases.
    pub root_id: Option<Cow<'a, [u8]>>,
    /// The call that was running.
    ///
    /// It holds the effective context, the callback, and the callout of a
    /// delivery.
    /// A sink that groups its lines by request reads the context from it.
    /// A guest can also write a line when no callback is running, through
    /// [`Guest::call_export`](crate::abi::v0_2_1::Guest::call_export) or from
    /// its own start up code, and this is `None` there.
    pub call: Option<Invocation>,
}

impl<'a> LogContext<'a> {
    /// A line from `guest` in the virtual machine `vm_id`, with no plugin
    /// and no call.
    ///
    /// Build one this way when you test a sink of your own.
    pub fn new(vm_id: impl Into<Cow<'a, [u8]>>, guest: GuestId) -> Self {
        Self {
            vm_id: vm_id.into(),
            guest,
            plugin_name: None,
            root_id: None,
            call: None,
        }
    }

    /// The same line, from the plugin with this name and this root id.
    #[must_use]
    pub fn with_plugin(
        mut self,
        name: impl Into<Cow<'a, [u8]>>,
        root_id: impl Into<Cow<'a, [u8]>>,
    ) -> Self {
        self.plugin_name = Some(name.into());
        self.root_id = Some(root_id.into());
        self
    }

    /// The same line, written while `call` was running.
    #[must_use]
    pub fn with_call(mut self, call: Invocation) -> Self {
        self.call = Some(call);
        self
    }

    /// The same line, with no borrow of the call that wrote it.
    ///
    /// A sink that writes each line at once needs no copy.
    /// A sink that batches its lines, puts them on a queue, or sends them to
    /// another thread cannot hold the borrowed value.
    /// This copies the VM id, and the plugin name and the root id when they
    /// are present, so it allocates once, twice, or three times.
    #[must_use]
    pub fn into_owned(self) -> LogContext<'static> {
        LogContext {
            vm_id: Cow::Owned(self.vm_id.into_owned()),
            guest: self.guest,
            plugin_name: self.plugin_name.map(|name| Cow::Owned(name.into_owned())),
            root_id: self.root_id.map(|id| Cow::Owned(id.into_owned())),
            call: self.call,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::v0_2_1::{Callback, CalloutId, ContextId};

    #[test]
    fn an_owned_context_holds_what_the_borrowed_one_held() {
        // Arrange
        let callout = CalloutId::try_from(3).unwrap();
        let call = Invocation::new(GuestId::next(), ContextId::try_from(7).unwrap())
            .with_callback(Callback::RequestHeaders)
            .with_callout(callout);
        let borrowed = LogContext::new(b"vm-1".as_slice(), call.guest)
            .with_plugin(b"authz".as_slice(), b"main".as_slice())
            .with_call(call);

        // Act
        let owned = borrowed.clone().into_owned();

        // Assert
        assert_eq!(owned, borrowed);
        assert!(matches!(owned.vm_id, Cow::Owned(_)));
        assert!(matches!(owned.plugin_name, Some(Cow::Owned(_))));
        assert!(matches!(owned.root_id, Some(Cow::Owned(_))));
    }

    #[test]
    fn a_context_with_no_plugin_and_no_call_owns_two_absences() {
        // Arrange
        let borrowed = LogContext::new(b"vm-1".as_slice(), GuestId::next());

        // Act
        let owned = borrowed.into_owned();

        // Assert
        assert_eq!(owned.plugin_name, None);
        assert_eq!(owned.root_id, None);
        assert_eq!(owned.call, None);
    }
}
