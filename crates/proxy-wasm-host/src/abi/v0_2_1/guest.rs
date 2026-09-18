//! A running guest bound to ABI v0.2.1.

mod callbacks;

use std::fmt;
use std::time::Duration;

use crate::Error;
use crate::abi::AbiVersion;
use crate::abi::v0_2_1::AbiAccess;
use crate::abi::v0_2_1::{
    CallScope, Callback, ContextId, ContextState, ContextType, NoStream, PluginConfig, StreamState,
};
use crate::runtime::{Engine, Instance, Limits, Module, VmServices};
use callbacks::Callbacks;

/// A running guest and the ABI conversation with it.
///
/// `Guest` wraps an [`Instance`], checks the module's ABI version before it
/// is instantiated, resolves the callbacks once, and owns the context table.
/// You drive the guest through a [`CallScope`].
///
/// A root context is created first, because both SDKs look it up by the
/// first parameter of `proxy_on_vm_start`.
/// The stream state you give to [`Guest::enter`] is owned for the duration of
/// the scope and must be `'static`, so move your request state in and take it
/// back with [`CallScope::finish`].
///
/// For example, a guest with no callbacks runs the whole lifecycle with the
/// default answers:
///
/// ```
/// use proxy_wasm_host::abi::v0_2_1::types::{Action, MapType, Status};
/// use proxy_wasm_host::abi::v0_2_1::{Access, Guest, Invocation, PluginConfig, StreamState};
/// use proxy_wasm_host::runtime::{Engine, VmServices, Limits, LogSink, Module};
/// use proxy_wasm_host::{HeaderMap, VecHeaderMap};
///
/// struct Stderr;
/// impl LogSink for Stderr {
///     fn log(&self, _: proxy_wasm_host::abi::v0_2_1::types::LogLevel, message: &[u8]) {
///         eprintln!("{}", String::from_utf8_lossy(message));
///     }
/// }
///
/// struct Request {
///     headers: VecHeaderMap,
/// }
/// impl StreamState for Request {
///     fn header_map(&mut self, _: Invocation, _: Access, map: MapType) -> Result<&mut dyn HeaderMap, Status> {
///         match map {
///             MapType::HttpRequestHeaders => Ok(&mut self.headers),
///             _ => Err(Status::NotFound),
///         }
///     }
/// }
///
/// # fn main() -> Result<(), proxy_wasm_host::Error> {
/// let wat = r#"(module
///     (memory (export "memory") 1)
///     (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 1024)
///     (func (export "proxy_abi_version_0_2_1")))"#;
/// let engine = Engine::new()?;
/// let module = Module::new(&engine, &wat::parse_str(wat).unwrap())?;
/// let services = VmServices::new(std::sync::Arc::new(Stderr));
/// let mut guest = Guest::new(&engine, &module, services, &Limits::default())?;
///
/// let mut root_scope = guest.enter_root();
/// let root = root_scope.on_context_create(None)?;
/// assert!(root_scope.on_vm_start(root)?);
/// assert!(root_scope.on_configure(root, PluginConfig::new())?);
/// drop(root_scope);
///
/// let request = Request { headers: VecHeaderMap::default() };
/// let mut scope = guest.enter(request);
/// let stream = scope.on_context_create(Some(root))?;
/// assert_eq!(scope.on_request_headers(stream, 0, true)?, Action::Continue);
/// let request = scope.finish();
///
/// // The proxy does its own work with the request here, then lends it again.
/// let mut scope = guest.enter(request);
/// assert!(scope.on_done(stream)?);
/// scope.on_log(stream)?;
/// scope.on_delete(stream)?;
/// let request = scope.finish();
/// assert!(request.headers.is_empty());
/// # Ok(())
/// # }
/// ```
pub struct Guest {
    instance: Instance,
    abi: AbiVersion,
    callbacks: Callbacks,
}

impl fmt::Debug for Guest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Guest")
            .field("abi", &self.abi)
            .field("effective_context", &self.effective_context())
            .field("poisoned", &self.instance.is_poisoned())
            .finish_non_exhaustive()
    }
}

impl Guest {
    /// Checks the module's ABI version, instantiates it, and resolves its
    /// callbacks.
    ///
    /// # Errors
    ///
    /// Returns [`Error::UnsupportedAbi`] before any instantiation when the
    /// module exports no accepted `proxy_abi_version_*` marker, any error
    /// [`Instance::new`] returns, and [`Error::ExportTypeMismatch`] when a
    /// callback is exported with another type than the ABI gives it.
    pub fn new(
        engine: &Engine,
        module: &Module,
        services: VmServices,
        limits: &Limits,
    ) -> Result<Self, Error> {
        let abi = module.abi()?;
        let mut instance = Instance::new(engine, module, services, limits)?;
        let callbacks = Callbacks::resolve(&mut instance)?;
        Ok(Self {
            instance,
            abi,
            callbacks,
        })
    }

    /// The ABI version the module advertises.
    pub fn abi(&self) -> AbiVersion {
        self.abi
    }

    /// The instance this guest runs in.
    pub fn instance(&self) -> &Instance {
        &self.instance
    }

    /// The instance, for a raw call to an export.
    ///
    /// Calling a `proxy_on_*` export through this handle bypasses the
    /// context table, so the guest's view of the contexts and this guest's
    /// view no longer agree.
    /// Use it for exports the ABI does not name.
    pub fn instance_mut(&mut self) -> &mut Instance {
        &mut self.instance
    }

    /// Whether the guest exports `callback`.
    ///
    /// A callback the guest does not export returns its default answer
    /// without entering the guest.
    pub fn exports_callback(&self, callback: Callback) -> bool {
        self.callbacks.exports(callback)
    }

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
    /// [`CallScope::on_configure`] records it, so this is `None` until that
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

    /// Lends `stream` to the guest for a group of callbacks.
    ///
    /// The guest owns the value until [`CallScope::finish`] returns it or
    /// the scope drops, and the value must be `'static`, because wasmtime
    /// requires that of store data.
    /// Move your request state in and take it back out rather than lending a
    /// borrow.
    /// The value replaces any stream state a forgotten scope left installed.
    pub fn enter<H: StreamState>(&mut self, stream: H) -> CallScope<'_, H> {
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
    /// If your root does either, enter the scope with a value of your own
    /// through [`Guest::enter`] instead of this shortcut.
    pub fn enter_root(&mut self) -> CallScope<'_, NoStream> {
        self.enter(NoStream)
    }

    pub(crate) fn callbacks(&self) -> &Callbacks {
        &self.callbacks
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::test_support::{engine, services, wat_bytes};

    fn guest(engine: &Engine, wat: &str) -> Result<Guest, Error> {
        let module = Module::new(engine, &wat_bytes(wat))?;
        Guest::new(engine, &module, services(), &Limits::default())
    }

    fn assert_send<T: Send>() {}

    const _: () = {
        let _ = assert_send::<Guest>;
    };

    #[test]
    fn the_abi_marker_is_checked_before_instantiation() {
        // Arrange
        let engine = engine();
        let accepted = r#"(module
            (memory (export "memory") 1)
            (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 1024)
            (func (export "proxy_abi_version_0_2_1")))"#;
        let rejected = r#"(module
            (memory (export "memory") 1)
            (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 1024)
            (func (export "proxy_abi_version_0_1_0"))
            (func (export "_start") unreachable))"#;

        // Act
        let results = (guest(&engine, accepted), guest(&engine, rejected));

        // Assert
        assert_eq!(results.0.unwrap().abi(), AbiVersion::V0_2_1);
        assert!(matches!(
            results.1,
            Err(Error::UnsupportedAbi { found }) if found == ["proxy_abi_version_0_1_0"]
        ));
    }

    #[test]
    fn a_callback_with_the_wrong_type_fails_construction() {
        // Arrange
        let engine = engine();
        let wat = r#"(module
            (memory (export "memory") 1)
            (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 1024)
            (func (export "proxy_abi_version_0_2_1"))
            (func (export "proxy_on_request_headers") (param i32) (result i32) i32.const 0))"#;

        // Act
        let result = guest(&engine, wat);

        // Assert
        assert!(matches!(
            result,
            Err(Error::ExportTypeMismatch { name }) if name == "proxy_on_request_headers"
        ));
    }

    #[test]
    fn exported_callbacks_are_reported_and_a_new_guest_holds_no_context() {
        // Arrange
        let engine = engine();
        let wat = r#"(module
            (memory (export "memory") 1)
            (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 1024)
            (func (export "proxy_abi_version_0_2_1"))
            (func (export "proxy_on_done") (param i32) (result i32) i32.const 1))"#;
        let guest = guest(&engine, wat).unwrap();
        let one = ContextId::try_from(1).unwrap();

        // Act
        let exports = (
            guest.exports_callback(Callback::Done),
            guest.exports_callback(Callback::Log),
        );

        // Assert
        assert_eq!(exports, (true, false));
        assert_eq!(guest.context_state(one), None);
        assert_eq!(guest.context_type(one), None);
        assert_eq!(guest.context_parent(one), None);
        assert_eq!(guest.effective_context(), None);
        assert_eq!(guest.rejected_by(one), None);
        assert!(!guest.instance().is_poisoned());
        assert_eq!(
            format!("{guest:?}"),
            "Guest { abi: V0_2_1, effective_context: None, poisoned: false, .. }"
        );
    }
}
