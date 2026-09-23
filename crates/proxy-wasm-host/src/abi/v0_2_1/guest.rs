//! A running guest bound to ABI v0.2.1.

mod callbacks;
mod contexts;
mod counting;
mod exports;
pub(crate) mod identity;
mod recovery;
pub(crate) mod start;

use std::fmt;
use std::sync::Arc;

use crate::abi::AbiVersion;
use crate::abi::v0_2_1::AbiAccess;
use crate::abi::v0_2_1::VmServices;
use crate::abi::v0_2_1::guest_spec::BuildCounters;
use crate::abi::v0_2_1::{CallScope, Callback, GuestError, Host, NoStream, StreamState};
use crate::runtime::{Instance, Limits, Module};
use callbacks::Callbacks;
use identity::GuestId;

/// A running guest and the ABI conversation with it.
///
/// `Guest` wraps an instance, checks the module's ABI version before it
/// is instantiated, resolves the callbacks once, and owns the context table.
/// You drive the guest through a [`CallScope`].
///
/// A root context is created first, because both SDKs look it up by the
/// first parameter of `proxy_on_vm_start`.
/// The stream state you give to [`Guest::enter`] is owned for the duration of
/// the scope and must be `'static`, so move your request state in and take it
/// back with [`CallScope::finish`].
/// [`Guest::with`] does both for you, and it gives the request back when
/// your code returns early.
///
/// A `Guest` is `Send` and is not `Sync`, so one thread drives it at a time
/// and you may move it between threads.
/// A trap, a spent limit, or a panic in your stream state poisons the guest,
/// and [`Guest::is_poisoned`] then reports `true`.
/// To recover, build a new `Guest` from the same [`Module`] with
/// [`GuestSpec`](crate::abi::v0_2_1::GuestSpec).
/// The module keeps the compiled code, so the new guest does not compile it
/// again.
///
/// For example, a guest with no callbacks runs the whole lifecycle with the
/// default answers:
///
/// ```
/// use proxy_wasm_host::abi::v0_2_1::types::{Action, MapType, Status};
/// use proxy_wasm_host::abi::v0_2_1::{Access, Guest, Invocation, PluginConfig, StreamState};
/// use proxy_wasm_host::abi::v0_2_1::{GuestError, Host, LogSink, VmServices};
/// use proxy_wasm_host::{Engine, HeaderMap, Limits, Module, VecHeaderMap};
///
/// struct Stderr;
/// impl LogSink for Stderr {
///     fn log(
///         &self,
///         _: proxy_wasm_host::abi::v0_2_1::LogContext<'_>,
///         _: proxy_wasm_host::abi::v0_2_1::types::LogLevel,
///         message: &[u8],
///     ) {
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
/// # fn main() -> Result<(), GuestError> {
/// let wat = r#"(module
///     (memory (export "memory") 1)
///     (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 1024)
///     (func (export "proxy_abi_version_0_2_1")))"#;
/// let engine = Engine::new()?;
/// let module = Module::new(&engine, &wat::parse_str(wat).unwrap())?;
/// let services = VmServices::new(std::sync::Arc::new(Stderr));
/// let host = Host::new(&engine)?;
/// let mut guest = Guest::new(&host, &module, services, &Limits::default())?;
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
    /// The stream state a scope left behind, which no host function can
    /// reach, because it lives here rather than in the store.
    detached: Option<Box<dyn StreamState>>,
    instance: Instance,
    abi: AbiVersion,
    id: GuestId,
    callbacks: Callbacks,
    /// The counters of the spec that built this guest, which a guest built
    /// by [`Guest::new`] does not have.
    counters: Option<Arc<BuildCounters>>,
}

impl fmt::Debug for Guest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Guest")
            .field("id", &self.id)
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
    /// Returns [`GuestError::UnsupportedAbi`] before any instantiation when
    /// the module exports no accepted `proxy_abi_version_*` marker.
    /// Returns [`GuestError::Runtime`] with any error of the instantiation,
    /// and with [`Error::ExportTypeMismatch`](crate::Error::ExportTypeMismatch)
    /// when a callback is exported with another type than the ABI gives it.
    pub fn new(
        host: &Host,
        module: &Module,
        services: VmServices,
        limits: &Limits,
    ) -> Result<Self, GuestError> {
        let abi = AbiVersion::detect(module.abi_exports())?;
        let mut instance = Instance::new(
            host.engine(),
            host.linker(),
            module,
            crate::abi::v0_2_1::state(services),
            limits,
        )?;
        let callbacks = Callbacks::resolve(&mut instance)?;
        let id = instance.state().abi().guest();
        Ok(Self {
            detached: None,
            instance,
            abi,
            id,
            callbacks,
            counters: None,
        })
    }

    /// The ABI version the module advertises.
    pub fn abi(&self) -> AbiVersion {
        self.abi
    }

    /// The identity of this guest in this process.
    ///
    /// Your [`Callouts`](crate::abi::v0_2_1::Callouts) service receives the
    /// same value on its [`Invocation`](crate::abi::v0_2_1::Invocation), so
    /// one service serves several guests and tells their callouts apart.
    pub fn id(&self) -> GuestId {
        self.id
    }

    pub(crate) fn instance(&self) -> &Instance {
        &self.instance
    }

    pub(crate) fn instance_mut(&mut self) -> &mut Instance {
        &mut self.instance
    }

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

    /// The services this guest runs against.
    pub fn services(&self) -> &VmServices {
        self.instance.state().abi().services()
    }

    /// The services, for a change between calls.
    ///
    /// Replacing the shared services drops the queue and metric identifiers
    /// the guest obtained, because an identifier belongs to the store that
    /// issued it.
    pub fn services_mut(&mut self) -> &mut VmServices {
        self.instance.state_mut().abi_mut().services_mut()
    }

    /// Whether the guest exports `callback`.
    ///
    /// A callback the guest does not export returns its default answer
    /// without entering the guest.
    pub fn exports_callback(&self, callback: Callback) -> bool {
        self.callbacks.exports(callback)
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

    pub(crate) fn callbacks(&self) -> &Callbacks {
        &self.callbacks
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::v0_2_1::ContextId;
    use crate::abi::v0_2_1::test_support::{RecordingStream, engine, services, wat_bytes};
    use crate::abi::v0_2_1::types::LogLevel;
    use crate::{Engine, Error};

    fn guest(engine: &Engine, wat: &str) -> Result<Guest, GuestError> {
        let module = Module::new(engine, &wat_bytes(wat))?;
        Guest::new(
            &Host::new(engine).unwrap(),
            &module,
            services(),
            &Limits::default(),
        )
    }

    const MARKED: &str = r#"(module
        (memory (export "memory") 1)
        (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 1024)
        (func (export "proxy_abi_version_0_2_1")))"#;

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
            Err(GuestError::UnsupportedAbi(unsupported))
                if unsupported.found == ["proxy_abi_version_0_1_0"]
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
            Err(GuestError::Runtime(Error::ExportTypeMismatch { name }))
                if name == "proxy_on_request_headers"
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
            format!(
                "Guest {{ id: {:?}, abi: V0_2_1, effective_context: None, poisoned: false, .. }}",
                guest.id()
            )
        );
    }

    #[test]
    fn a_change_through_services_mut_is_read_back_through_services() {
        // Arrange
        let engine = engine();
        let mut guest = guest(&engine, MARKED).unwrap();
        let before = guest.services().log_level();

        // Act
        guest.services_mut().set_log_level(LogLevel::Critical);

        // Assert
        assert_ne!(before, LogLevel::Critical);
        assert_eq!(guest.services().log_level(), LogLevel::Critical);
    }

    #[test]
    fn a_body_that_returns_early_still_yields_the_stream_state() {
        // Arrange
        let engine = engine();
        let wat = r#"(module
            (import "env" "proxy_replace_header_map_value" (func $replace (param i32 i32 i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 4096)
            (func (export "proxy_abi_version_0_2_1"))
            (func (export "proxy_on_request_headers") (param i32 i32 i32) (result i32)
                (drop (call $replace (i32.const 0) (i32.const 100) (i32.const 1) (i32.const 101) (i32.const 1)))
                i32.const 0)
            (data (i32.const 100) "kv"))"#;
        let mut guest = guest(&engine, wat).unwrap();
        let root = guest.enter_root().on_context_create(None).unwrap();
        let stream = guest.enter_root().on_context_create(Some(root)).unwrap();

        // Act
        let (answer, recording) = guest.with(RecordingStream::new(), |scope| {
            let action = scope.on_request_headers(stream, 0, true)?;
            Err::<(), GuestError>(GuestError::from(Error::Config {
                message: format!("stopping after {action:?}"),
            }))
        });

        // Assert
        assert!(
            matches!(&answer, Err(GuestError::Runtime(Error::Config { message })) if message == "stopping after Continue")
        );
        assert_eq!(recording.calls().len(), 1);
        assert!(guest.take_stream_any().is_none());
        assert!(!guest.is_poisoned());
    }
}
