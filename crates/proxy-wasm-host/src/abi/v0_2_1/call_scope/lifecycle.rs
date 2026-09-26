//! The callbacks that start a context, an instance, and a plugin.

use crate::abi::v0_2_1::AbiAccess;
use crate::abi::v0_2_1::GuestError;
use crate::abi::v0_2_1::call_scope::{CallScope, prologue};
use crate::abi::v0_2_1::{Callback, ContextId, PluginConfig, StreamState};

impl<H: StreamState> CallScope<'_, H> {
    /// Creates a context and calls `proxy_on_context_create`.
    ///
    /// A `None` parent creates a root context, and a root parent creates a
    /// stream context.
    /// The identifier is allocated here and never reused.
    /// One instance can allocate about four billion contexts, which is about
    /// twelve hours at one hundred thousand requests per second, after which
    /// you create a new instance.
    ///
    /// # Errors
    ///
    /// Returns [`GuestError::Context`] when the parent is unknown or not a
    /// root context, [`GuestError::GuestRejected`],
    /// [`GuestError::ContextIdsExhausted`], and the
    /// [common runtime failures](CallScope#the-common-runtime-failures).
    pub fn on_context_create(
        &mut self,
        parent: Option<ContextId>,
    ) -> Result<ContextId, GuestError> {
        self.guest.require_live()?;
        match parent {
            Some(parent) => {
                prologue::require_root(self.guest, parent)?;
                prologue::accepted(self.guest, parent)?;
            }
            None => prologue::vm_accepted(self.guest)?,
        }
        let id = self
            .guest
            .instance_mut()
            .state_mut()
            .abi_mut()
            .contexts_mut()
            .create(parent)?;
        let parent = parent.map_or(0, ContextId::wire);
        prologue::run(
            self.guest,
            id,
            Callback::ContextCreate,
            |callbacks| callbacks.context_create.as_ref(),
            (id.wire(), parent),
            (),
        )?;
        Ok(id)
    }

    /// Calls `proxy_on_vm_start` on a root context.
    ///
    /// The guest is told the length of the VM configuration that
    /// [`VmServices::with_vm_configuration`](crate::abi::v0_2_1::VmServices::with_vm_configuration)
    /// holds, and it reads the bytes from the `VM_CONFIGURATION` buffer.
    /// A `false` answer refuses the whole instance.
    ///
    /// # Errors
    ///
    /// Returns [`GuestError::Context`] when `root` is unknown or a stream
    /// context, [`GuestError::GuestRejected`], and
    /// [`GuestError::UnexpectedReturn`].
    /// Returns [`GuestError::Runtime`] with
    /// [`Error::ValueTooLarge`](crate::Error::ValueTooLarge) for a
    /// configuration above `i32::MAX` bytes, and the
    /// [common runtime failures](CallScope#the-common-runtime-failures).
    pub fn on_vm_start(&mut self, root: ContextId) -> Result<bool, GuestError> {
        self.guest.require_live()?;
        prologue::require_root(self.guest, root)?;
        prologue::accepted(self.guest, root)?;
        let length = self
            .guest
            .instance()
            .state()
            .abi()
            .services()
            .vm_configuration()
            .len();
        let size = prologue::wire_size(length)?;
        let value = prologue::run(
            self.guest,
            root,
            Callback::VmStart,
            |callbacks| callbacks.vm_start.as_ref(),
            (root.wire(), size),
            1,
        )?;
        let accepted = prologue::boolean(Callback::VmStart, value)?;
        if !accepted {
            self.guest
                .instance_mut()
                .state_mut()
                .abi_mut()
                .contexts_mut()
                .reject_vm(root);
        }
        Ok(accepted)
    }

    /// Calls `proxy_on_configure` on a root context with the plugin it
    /// serves.
    ///
    /// The crate records `plugin` on the root context before it calls the
    /// guest, so the guest reads the bytes from the `PLUGIN_CONFIGURATION`
    /// buffer inside the callback, and it is told their length.
    /// [`Guest::plugin`](crate::abi::v0_2_1::Guest::plugin) reads the value
    /// back.
    /// A `false` answer refuses this root context and every stream context
    /// under it.
    /// The guest decides, because the crate reads nothing in the bytes of
    /// [`PluginConfig`](crate::abi::v0_2_1::PluginConfig).
    /// A caller that drives the callbacks by hand reads the boolean here.
    /// A caller that uses [`Guest::start`](crate::abi::v0_2_1::Guest::start)
    /// reads the same refusal as
    /// [`Started::Refused`](crate::abi::v0_2_1::Started::Refused).
    ///
    /// # Errors
    ///
    /// Returns [`GuestError::Context`] when `root` is unknown or a stream
    /// context, [`GuestError::GuestRejected`], and
    /// [`GuestError::UnexpectedReturn`].
    /// Returns [`GuestError::Runtime`] with
    /// [`Error::ValueTooLarge`](crate::Error::ValueTooLarge) for a
    /// configuration above `i32::MAX` bytes, and the
    /// [common runtime failures](CallScope#the-common-runtime-failures).
    pub fn on_configure(
        &mut self,
        root: ContextId,
        plugin: PluginConfig,
    ) -> Result<bool, GuestError> {
        self.guest.require_live()?;
        prologue::require_root(self.guest, root)?;
        prologue::accepted(self.guest, root)?;
        let size = prologue::wire_size(plugin.configuration().len())?;
        self.guest
            .instance_mut()
            .state_mut()
            .abi_mut()
            .contexts_mut()
            .set_plugin(root, plugin);
        let value = prologue::run(
            self.guest,
            root,
            Callback::Configure,
            |callbacks| callbacks.configure.as_ref(),
            (root.wire(), size),
            1,
        )?;
        let accepted = prologue::boolean(Callback::Configure, value)?;
        if !accepted {
            self.guest
                .instance_mut()
                .state_mut()
                .abi_mut()
                .contexts_mut()
                .reject(root);
        }
        Ok(accepted)
    }
}
