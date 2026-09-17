//! The scope in which callbacks run.

mod finalize;
mod prologue;

use std::any::Any;
use std::fmt;
use std::marker::PhantomData;

use crate::Error;
use crate::abi::v0_2_1::types::Action;
use crate::abi::v0_2_1::{Callback, ContextId, Guest, Plugin, StreamHost};

/// A group of callbacks that share one stream host.
///
/// [`Guest::enter`] gives you a scope, the callback methods run the guest,
/// and [`CallScope::finish`] gives the stream host back.
/// Between callbacks you reach the stream host through [`CallScope::stream`]
/// and [`CallScope::stream_mut`].
/// Dropping the scope without `finish` drops the stream host.
///
/// A panic in your [`StreamHost`] unwinds through the guest, and the
/// callback that was running never returns.
/// The scope notices that on drop, on `finish`, and on the next callback,
/// and poisons the instance each time, so the guest never runs again on host
/// state that a callback left half updated.
/// A panic elsewhere while the scope lives leaves the instance usable.
/// `std::mem::forget` on a scope leaves the stream host installed until the
/// next [`Guest::enter`].
pub struct CallScope<'a, H: StreamHost> {
    guest: &'a mut Guest,
    stream: PhantomData<H>,
}

impl<H: StreamHost> fmt::Debug for CallScope<'_, H> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CallScope")
            .field("guest", &self.guest)
            .field("stream", &std::any::type_name::<H>())
            .finish()
    }
}

impl<'a, H: StreamHost> CallScope<'a, H> {
    pub(crate) fn new(guest: &'a mut Guest) -> Self {
        Self {
            guest,
            stream: PhantomData,
        }
    }

    /// The guest this scope drives.
    pub fn guest(&self) -> &Guest {
        self.guest
    }

    /// The stream host this scope lends to the guest.
    ///
    /// # Panics
    ///
    /// Panics only if the installed stream host is not the value
    /// [`Guest::enter`] stored, which cannot happen through the public API.
    pub fn stream(&self) -> &H {
        self.guest
            .instance()
            .state()
            .abi()
            .stream_host_as_ref::<H>()
            .unwrap_or_else(|| {
                unreachable!("the scope holds the guest, so its stream host is installed")
            })
    }

    /// The stream host this scope lends to the guest, for changes between
    /// callbacks.
    ///
    /// # Panics
    ///
    /// Panics only if the installed stream host is not the value
    /// [`Guest::enter`] stored, which cannot happen through the public API.
    pub fn stream_mut(&mut self) -> &mut H {
        self.guest
            .instance_mut()
            .state_mut()
            .abi_mut()
            .stream_host_as::<H>()
            .unwrap_or_else(|| {
                unreachable!("the scope holds the guest, so its stream host is installed")
            })
    }

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
    /// Returns [`Error::Poisoned`], [`Error::Context`] when the parent is
    /// unknown or not a root context, [`Error::GuestRejected`],
    /// [`Error::ContextIdsExhausted`], and the errors of a guest call.
    pub fn on_context_create(&mut self, parent: Option<ContextId>) -> Result<ContextId, Error> {
        prologue::live(self.guest)?;
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
        let func = self.guest.callbacks().context_create.clone();
        let parent = parent.map_or(0, ContextId::wire);
        prologue::run(
            self.guest,
            id,
            Callback::ContextCreate,
            func,
            (id.wire(), parent),
            (),
        )?;
        Ok(id)
    }

    /// Calls `proxy_on_vm_start` on a root context.
    ///
    /// The guest is told the length of the VM configuration that
    /// [`HostServices::with_vm_configuration`](crate::runtime::HostServices::with_vm_configuration)
    /// holds, and it reads the bytes from the `VM_CONFIGURATION` buffer.
    /// A `false` answer refuses the whole instance.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Poisoned`], [`Error::Context`] when `root` is unknown
    /// or a stream context, [`Error::GuestRejected`],
    /// [`Error::ValueTooLarge`] for a configuration above `i32::MAX` bytes,
    /// [`Error::UnexpectedReturn`], and the errors of a guest call.
    pub fn on_vm_start(&mut self, root: ContextId) -> Result<bool, Error> {
        prologue::live(self.guest)?;
        prologue::require_root(self.guest, root)?;
        prologue::accepted(self.guest, root)?;
        let length = self
            .guest
            .instance()
            .state()
            .services()
            .vm_configuration()
            .len();
        let size = prologue::wire_size(length)?;
        let func = self.guest.callbacks().vm_start.clone();
        let value = prologue::run(
            self.guest,
            root,
            Callback::VmStart,
            func,
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
    /// [`Guest::plugin`] reads the value back.
    /// A `false` answer refuses this root context and every stream context
    /// under it.
    ///
    /// # Errors
    ///
    /// The same as [`CallScope::on_vm_start`].
    pub fn on_configure(&mut self, root: ContextId, plugin: Plugin) -> Result<bool, Error> {
        prologue::live(self.guest)?;
        prologue::require_root(self.guest, root)?;
        prologue::accepted(self.guest, root)?;
        let size = prologue::wire_size(plugin.configuration().len())?;
        self.guest
            .instance_mut()
            .state_mut()
            .abi_mut()
            .contexts_mut()
            .set_plugin(root, plugin);
        let func = self.guest.callbacks().configure.clone();
        let value = prologue::run(
            self.guest,
            root,
            Callback::Configure,
            func,
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

    /// Calls `proxy_on_request_headers` on a stream context.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Poisoned`], [`Error::Context`] for an unknown
    /// context or a root context, [`Error::GuestRejected`],
    /// [`Error::ValueTooLarge`] for a count above `i32::MAX`,
    /// [`Error::UnexpectedReturn`], and the errors of a guest call.
    pub fn on_request_headers(
        &mut self,
        context: ContextId,
        num_headers: u32,
        end_of_stream: bool,
    ) -> Result<Action, Error> {
        prologue::live(self.guest)?;
        prologue::require_stream(self.guest, context)?;
        prologue::accepted(self.guest, context)?;
        let count = prologue::wire_u32(num_headers)?;
        let callback = Callback::RequestHeaders;
        let func = self.guest.callbacks().request_headers.clone();
        let params = (context.wire(), count, i32::from(end_of_stream));
        let default = i32::from(Action::Continue);
        let value = prologue::run(self.guest, context, callback, func, params, default)?;
        Action::try_from(value).map_err(|_| Error::UnexpectedReturn { callback, value })
    }

    /// Takes the stream host back.
    ///
    /// It works on a poisoned instance as well, because the value is in the
    /// store data whatever the guest did.
    /// If a callback of this scope never returned, the instance is poisoned
    /// here.
    ///
    /// # Panics
    ///
    /// Panics only if the installed stream host is not the value
    /// [`Guest::enter`] stored, which cannot happen through the public API.
    #[must_use]
    pub fn finish(self) -> H {
        let state = self.guest.instance_mut().state_mut();
        if state.abi_mut().current_callback().is_some() {
            state.poison();
        }
        state
            .abi_mut()
            .take_stream_host()
            .and_then(|boxed| {
                let any: Box<dyn Any + Send> = boxed;
                any.downcast::<H>().ok()
            })
            .map_or_else(
                || unreachable!("the scope holds the guest, so its stream host is installed"),
                |stream| *stream,
            )
    }
}

impl<H: StreamHost> Drop for CallScope<'_, H> {
    fn drop(&mut self) {
        let state = self.guest.instance_mut().state_mut();
        let _ = state.abi_mut().take_stream_host();
        if state.abi_mut().current_callback().is_some() {
            state.poison();
            state.abi_mut().set_current_callback(None);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::panic::{AssertUnwindSafe, catch_unwind};

    use super::*;
    use crate::abi::v0_2_1::test_support::{RecordingStream, status};
    use crate::abi::v0_2_1::types::{MapType, Status};
    use crate::abi::v0_2_1::{Access, ContextProblem, ContextState, HostCall, NoStream};
    use crate::header_map::HeaderMap;
    use crate::runtime::test_support::{engine, services, wat_bytes};
    use crate::runtime::{Engine, GuestPtr, Limits, Module};
    use std::time::Duration;

    const RECORDER: &str = r#"(module
        (memory (export "memory") 1)
        (global $answer (mut i32) (i32.const 1))
        (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 4096)
        (func (export "proxy_abi_version_0_2_1"))
        (func (export "set_answer") (param i32) (global.set $answer (local.get 0)))
        (func (export "proxy_on_context_create") (param i32 i32)
            (i32.store (i32.const 0) (local.get 0)) (i32.store (i32.const 4) (local.get 1)))
        (func (export "proxy_on_vm_start") (param i32 i32) (result i32)
            (i32.store (i32.const 8) (local.get 0)) (i32.store (i32.const 12) (local.get 1)) global.get $answer)
        (func (export "proxy_on_configure") (param i32 i32) (result i32)
            (i32.store (i32.const 16) (local.get 0)) (i32.store (i32.const 20) (local.get 1)) global.get $answer)
        (func (export "proxy_on_request_headers") (param i32 i32 i32) (result i32)
            (i32.store (i32.const 24) (local.get 0)) (i32.store (i32.const 28) (local.get 1))
            (i32.store (i32.const 32) (local.get 2)) global.get $answer)
        (func (export "proxy_on_done") (param i32) (result i32)
            (i32.store (i32.const 36) (local.get 0)) global.get $answer)
        (func (export "proxy_on_log") (param i32) (i32.store (i32.const 40) (local.get 0)))
        (func (export "proxy_on_delete") (param i32) (i32.store (i32.const 44) (local.get 0))))"#;

    const SILENT: &str = r#"(module
        (memory (export "memory") 1)
        (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 4096)
        (func (export "proxy_abi_version_0_2_1"))
        (func (export "sentinel") (result i32) (i32.load (i32.const 0)))
        (func (export "_start") (i32.store (i32.const 0) (i32.const 7))))"#;

    const HEADER_WRITER: &str = r#"(module
        (import "env" "proxy_replace_header_map_value" (func $replace (param i32 i32 i32 i32 i32) (result i32)))
        (memory (export "memory") 1)
        (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 4096)
        (func (export "proxy_abi_version_0_2_1"))
        (func (export "write") (result i32)
            i32.const 0 i32.const 100 i32.const 1 i32.const 101 i32.const 1 call $replace)
        (func (export "proxy_on_request_headers") (param i32 i32 i32) (result i32)
            (drop (call $replace (i32.const 0) (i32.const 100) (i32.const 1) (i32.const 101) (i32.const 1)))
            i32.const 0)
        (data (i32.const 100) "kv"))"#;

    const TRAPPER: &str = r#"(module
        (memory (export "memory") 1)
        (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 4096)
        (func (export "proxy_abi_version_0_2_1"))
        (func (export "proxy_on_request_headers") (param i32 i32 i32) (result i32) unreachable))"#;

    const DONE_CALLER: &str = r#"(module
        (import "env" "proxy_done" (func $done (result i32)))
        (memory (export "memory") 1)
        (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 4096)
        (func (export "proxy_abi_version_0_2_1"))
        (func (export "proxy_on_done") (param i32) (result i32)
            (i32.store (i32.const 48) (call $done)) i32.const 0))"#;

    fn guest(engine: &Engine, wat: &str) -> Guest {
        let module = Module::new(engine, &wat_bytes(wat)).unwrap();
        Guest::new(engine, &module, services(), &Limits::default()).unwrap()
    }

    fn guest_with_vm_configuration(engine: &Engine, wat: &str, bytes: &[u8]) -> Guest {
        let module = Module::new(engine, &wat_bytes(wat)).unwrap();
        let services = crate::runtime::HostServices::new(std::sync::Arc::new(
            crate::runtime::test_support::RecordingSink::default(),
        ))
        .with_vm_configuration(bytes.to_vec());
        Guest::new(engine, &module, services, &Limits::default()).unwrap()
    }

    fn recorded(guest: &mut Guest, at: u32) -> u32 {
        guest
            .instance_mut()
            .memory()
            .unwrap()
            .read_u32(GuestPtr::from_address(at))
            .unwrap()
    }

    fn answer(guest: &mut Guest, value: i32) {
        guest
            .instance_mut()
            .call::<i32, ()>("set_answer", value)
            .unwrap();
    }

    fn id(value: u32) -> ContextId {
        ContextId::try_from(value).unwrap()
    }

    fn with_root(engine: &Engine, wat: &str) -> (Guest, ContextId) {
        let mut guest = guest(engine, wat);
        let root = guest.enter_root().on_context_create(None).unwrap();
        (guest, root)
    }

    fn with_stream(engine: &Engine, wat: &str) -> (Guest, ContextId, ContextId) {
        let (mut guest, root) = with_root(engine, wat);
        let stream = guest.enter_root().on_context_create(Some(root)).unwrap();
        (guest, root, stream)
    }

    fn refused_root(engine: &Engine) -> (Guest, ContextId, ContextId, ContextId) {
        let (mut guest, root, stream) = with_stream(engine, RECORDER);
        let other = guest.enter_root().on_context_create(None).unwrap();
        answer(&mut guest, 0);
        assert!(
            !guest
                .enter_root()
                .on_configure(root, Plugin::new())
                .unwrap()
        );
        answer(&mut guest, 1);
        (guest, root, stream, other)
    }

    fn trapped(engine: &Engine) -> (Guest, ContextId) {
        let (mut guest, _, stream) = with_stream(engine, TRAPPER);
        let result = guest
            .enter(RecordingStream::new())
            .on_request_headers(stream, 0, true);
        assert!(matches!(result, Err(Error::Trap { .. })));
        (guest, stream)
    }

    struct Panicking;

    impl StreamHost for Panicking {
        fn header_map(&mut self, _: HostCall, _: MapType) -> Result<&mut dyn HeaderMap, Status> {
            panic!("the stream host failed")
        }
    }

    #[test]
    fn a_root_context_is_numbered_one_and_becomes_effective() {
        // Arrange
        let engine = engine();
        let mut guest = guest(&engine, RECORDER);

        // Act
        let root = guest.enter_root().on_context_create(None).unwrap();

        // Assert
        assert_eq!(root.get(), 1);
        assert_eq!((recorded(&mut guest, 0), recorded(&mut guest, 4)), (1, 0));
        assert_eq!(guest.effective_context(), Some(root));
    }

    #[test]
    fn a_stream_context_is_numbered_next_and_tells_the_guest_its_parent() {
        // Arrange
        let engine = engine();
        let (mut guest, root) = with_root(&engine, RECORDER);

        // Act
        let stream = guest.enter_root().on_context_create(Some(root)).unwrap();

        // Assert
        assert_eq!(stream.get(), 2);
        assert_eq!((recorded(&mut guest, 0), recorded(&mut guest, 4)), (2, 1));
        assert_eq!(guest.context_parent(stream), Some(root));
        assert_eq!(guest.effective_context(), Some(stream));
    }

    #[test]
    fn vm_start_passes_the_root_and_the_length_of_the_vm_configuration() {
        // Arrange
        let engine = engine();
        let mut guest = guest_with_vm_configuration(&engine, RECORDER, b"12345");
        let root = guest.enter_root().on_context_create(None).unwrap();
        let mut scope = guest.enter_root();

        // Act
        let started = scope.on_vm_start(root).unwrap();

        // Assert
        assert!(started);
        assert_eq!(
            (
                recorded(scope.guest_mut(), 8),
                recorded(scope.guest_mut(), 12)
            ),
            (1, 5)
        );
    }

    #[test]
    fn configure_passes_the_root_and_the_length_of_the_plugin_configuration() {
        // Arrange
        let engine = engine();
        let (mut guest, root) = with_root(&engine, RECORDER);
        let plugin = Plugin::new().with_configuration(b"123456".to_vec());
        let mut scope = guest.enter_root();

        // Act
        let configured = scope.on_configure(root, plugin).unwrap();

        // Assert
        assert!(configured);
        assert_eq!(
            (
                recorded(scope.guest_mut(), 16),
                recorded(scope.guest_mut(), 20)
            ),
            (1, 6)
        );
    }

    #[test]
    fn configure_records_the_plugin_on_the_root() {
        // Arrange
        let engine = engine();
        let (mut guest, root, stream) = with_stream(&engine, RECORDER);
        let plugin = Plugin::new().with_name(b"auth".to_vec());

        // Act
        let configured = guest.enter_root().on_configure(root, plugin).unwrap();

        // Assert
        assert!(configured);
        assert_eq!(guest.plugin(root).unwrap().name(), b"auth");
        assert_eq!(guest.plugin(stream).unwrap().name(), b"auth");
    }

    #[test]
    fn root_callbacks_need_a_root_context_and_stream_callbacks_a_stream() {
        // Arrange
        let engine = engine();
        let (mut guest, root, stream) = with_stream(&engine, RECORDER);
        let mut scope = guest.enter(RecordingStream::new());

        // Act
        let results = (
            scope.on_vm_start(stream),
            scope.on_configure(id(9), Plugin::new()),
            scope.on_request_headers(root, 0, true),
        );

        // Assert
        assert!(matches!(
            results.0,
            Err(Error::Context { id, problem: ContextProblem::NotRoot }) if id == stream
        ));
        assert!(matches!(
            results.1,
            Err(Error::Context { id, problem: ContextProblem::Unknown }) if id.get() == 9
        ));
        assert!(matches!(
            results.2,
            Err(Error::Context { id, problem: ContextProblem::NotStream }) if id == root
        ));
    }

    #[test]
    fn an_unexpected_boolean_is_reported() {
        // Arrange
        let engine = engine();
        let (mut guest, root) = with_root(&engine, RECORDER);
        answer(&mut guest, 7);

        // Act
        let result = guest.enter_root().on_configure(root, Plugin::new());

        // Assert
        assert!(matches!(
            result,
            Err(Error::UnexpectedReturn {
                callback: Callback::Configure,
                value: 7
            })
        ));
        assert_eq!(guest.rejected_by(root), None);
    }

    #[test]
    fn a_false_vm_start_refuses_the_whole_instance() {
        // Arrange
        let engine = engine();
        let (mut guest, root) = with_root(&engine, RECORDER);
        answer(&mut guest, 0);
        let refused = guest.enter_root().on_vm_start(root).unwrap();
        answer(&mut guest, 1);

        // Act
        let result = guest.enter_root().on_context_create(None);

        // Assert
        assert!(!refused);
        assert!(matches!(
            result,
            Err(Error::GuestRejected { callback: Callback::VmStart, root: r }) if r == root
        ));
        assert_eq!(guest.rejected_by(root), Some(Callback::VmStart));
        assert_eq!(recorded(&mut guest, 0), 1);
    }

    #[test]
    fn a_false_configure_refuses_that_root_and_its_streams_only() {
        // Arrange
        let engine = engine();
        let (mut guest, root, stream, other) = refused_root(&engine);
        let other_stream = guest.enter_root().on_context_create(Some(other)).unwrap();
        let mut scope = guest.enter(RecordingStream::new());

        // Act
        let results = (
            scope.on_request_headers(stream, 0, true),
            scope.on_request_headers(other_stream, 0, true),
        );

        // Assert
        assert!(matches!(
            results.0,
            Err(Error::GuestRejected { callback: Callback::Configure, root: r }) if r == root
        ));
        assert_eq!(results.1.unwrap(), Action::Pause);
        assert_eq!(scope.guest().rejected_by(root), Some(Callback::Configure));
        assert_eq!(scope.guest().rejected_by(other_stream), None);
    }

    #[test]
    fn a_refused_root_can_still_be_finalized() {
        // Arrange
        let engine = engine();
        let (mut guest, _, stream, _) = refused_root(&engine);
        let mut scope = guest.enter(RecordingStream::new());

        // Act
        let results = (
            scope.on_done(stream),
            scope.on_log(stream),
            scope.on_delete(stream),
        );

        // Assert
        assert!(results.0.unwrap());
        assert!(results.1.is_ok());
        assert!(results.2.is_ok());
        assert_eq!(scope.guest().context_state(stream), None);
    }

    #[test]
    fn request_headers_passes_its_arguments_to_the_guest() {
        // Arrange
        let engine = engine();
        let (mut guest, _, stream) = with_stream(&engine, RECORDER);
        answer(&mut guest, 0);
        let mut scope = guest.enter(RecordingStream::new());

        // Act
        let action = scope.on_request_headers(stream, 3, true).unwrap();

        // Assert
        assert_eq!(action, Action::Continue);
        let seen = (
            recorded(scope.guest_mut(), 24),
            recorded(scope.guest_mut(), 28),
            recorded(scope.guest_mut(), 32),
        );
        assert_eq!(seen, (2, 3, 1));
    }

    #[test]
    fn request_headers_maps_one_to_pause_and_rejects_other_values() {
        // Arrange
        let engine = engine();
        let (mut guest, _, stream) = with_stream(&engine, RECORDER);
        let mut scope = guest.enter(RecordingStream::new());
        let paused = scope.on_request_headers(stream, 0, false).unwrap();
        answer(scope.guest_mut(), 7);

        // Act
        let results = (
            scope.on_request_headers(stream, 0, false),
            scope.on_request_headers(stream, u32::MAX, true),
        );

        // Assert
        assert_eq!(paused, Action::Pause);
        assert!(matches!(
            results.0,
            Err(Error::UnexpectedReturn {
                callback: Callback::RequestHeaders,
                value: 7
            })
        ));
        assert!(matches!(results.1, Err(Error::ValueTooLarge { .. })));
        assert_eq!(recorded(scope.guest_mut(), 32), 0);
    }

    #[test]
    fn log_and_delete_wait_for_done() {
        // Arrange
        let engine = engine();
        let (mut guest, _, stream) = with_stream(&engine, RECORDER);
        let mut scope = guest.enter_root();

        // Act
        let results = (scope.on_log(stream), scope.on_delete(stream));

        // Assert
        assert!(matches!(
            results.0,
            Err(Error::Context {
                problem: ContextProblem::NotDone,
                ..
            })
        ));
        assert!(matches!(
            results.1,
            Err(Error::Context {
                problem: ContextProblem::NotDone,
                ..
            })
        ));
    }

    #[test]
    fn done_records_the_answer_both_ways() {
        // Arrange
        let engine = engine();
        let (mut guest, _, stream) = with_stream(&engine, RECORDER);
        let done = guest.enter_root().on_done(stream).unwrap();
        let after_true = guest.context_state(stream);
        answer(&mut guest, 0);

        // Act
        let again = guest.enter_root().on_done(stream).unwrap();

        // Assert
        assert!(done);
        assert_eq!(after_true, Some(ContextState::Done));
        assert!(!again);
        assert_eq!(guest.context_state(stream), Some(ContextState::Pending));
        assert_eq!(recorded(&mut guest, 36), 2);
    }

    #[test]
    fn log_and_delete_run_on_a_done_context_and_delete_forgets_it() {
        // Arrange
        let engine = engine();
        let (mut guest, _, stream) = with_stream(&engine, RECORDER);
        assert!(guest.enter_root().on_done(stream).unwrap());
        let mut scope = guest.enter_root();

        // Act
        let results = (scope.on_log(stream), scope.on_delete(stream));

        // Assert
        assert!(results.0.is_ok());
        assert!(results.1.is_ok());
        assert_eq!(recorded(scope.guest_mut(), 40), 2);
        assert_eq!(recorded(scope.guest_mut(), 44), 2);
        assert_eq!(scope.guest().context_state(stream), None);
    }

    #[test]
    fn a_root_with_streams_cannot_be_deleted() {
        // Arrange
        let engine = engine();
        let (mut guest, root, _) = with_stream(&engine, RECORDER);
        let mut scope = guest.enter_root();
        assert!(scope.on_done(root).unwrap());

        // Act
        let result = scope.on_delete(root);

        // Assert
        assert!(matches!(
            result,
            Err(Error::Context { id, problem: ContextProblem::HasChildren }) if id == root
        ));
    }

    #[test]
    fn absent_callbacks_answer_their_defaults_without_entering_the_guest() {
        // Arrange
        let engine = engine();
        let mut guest = guest(&engine, SILENT);
        let exported = Callback::ALL
            .iter()
            .any(|callback| guest.exports_callback(*callback));
        let mut scope = guest.enter_root();

        // Act
        let results = (
            scope.on_context_create(None).unwrap(),
            scope.on_vm_start(id(1)).unwrap(),
            scope.on_configure(id(1), Plugin::new()).unwrap(),
            scope.on_context_create(Some(id(1))).unwrap(),
            scope.on_request_headers(id(2), 0, true).unwrap(),
            scope.on_done(id(2)).unwrap(),
            scope.on_log(id(2)),
            scope.on_delete(id(2)),
        );

        // Assert
        assert!(!exported);
        assert_eq!(results.0.get(), 1);
        assert!(results.1 && results.2);
        assert_eq!(results.3.get(), 2);
        assert_eq!(results.4, Action::Continue);
        assert!(results.5);
        assert!(results.6.is_ok() && results.7.is_ok());
        assert_eq!(
            scope
                .guest_mut()
                .instance_mut()
                .call::<(), i32>("sentinel", ())
                .unwrap(),
            7
        );
        assert!(!scope.guest().instance().is_poisoned());
    }

    #[test]
    fn proxy_done_inside_on_done_sees_an_active_context() {
        // Arrange
        let engine = engine();
        let (mut guest, _, stream) = with_stream(&engine, DONE_CALLER);

        // Act
        let done = guest.enter_root().on_done(stream).unwrap();

        // Assert
        assert!(!done);
        assert_eq!(
            status(recorded(&mut guest, 48).cast_signed()),
            Status::NotFound
        );
        assert_eq!(guest.context_state(stream), Some(ContextState::Pending));
    }

    #[test]
    fn enter_installs_the_stream_and_the_accessors_see_the_change() {
        // Arrange
        let engine = engine();
        let (mut guest, _, stream) = with_stream(&engine, HEADER_WRITER);
        let mut scope = guest.enter(RecordingStream::new());

        // Act
        let action = scope.on_request_headers(stream, 0, true).unwrap();

        // Assert
        assert_eq!(action, Action::Continue);
        assert_eq!(
            scope.stream().pairs(MapType::HttpRequestHeaders),
            vec![("k".into(), "v".into())]
        );
        let call = scope.stream_mut().calls()[0].0;
        assert_eq!(call.context, stream);
        assert_eq!(call.callback, Some(Callback::RequestHeaders));
        assert_eq!(call.access, Access::Write);
        assert_eq!(scope.guest().effective_context(), Some(stream));
    }

    #[test]
    fn finish_returns_the_stream_with_the_change() {
        // Arrange
        let engine = engine();
        let (mut guest, _, stream) = with_stream(&engine, HEADER_WRITER);
        let mut scope = guest.enter(RecordingStream::new());
        assert_eq!(
            scope.on_request_headers(stream, 0, true).unwrap(),
            Action::Continue
        );

        // Act
        let recording = scope.finish();

        // Assert
        assert_eq!(
            recording.pairs(MapType::HttpRequestHeaders),
            vec![("k".into(), "v".into())]
        );
        assert_eq!(recording.calls().len(), 1);
        assert!(!guest.instance().is_poisoned());
    }

    #[test]
    fn a_dropped_scope_leaves_no_stream_host() {
        // Arrange
        let engine = engine();
        let (mut guest, _, _) = with_stream(&engine, HEADER_WRITER);
        drop(guest.enter(RecordingStream::new()));

        // Act
        let result = guest
            .instance_mut()
            .call::<(), i32>("write", ())
            .map(status);

        // Assert
        assert_eq!(result.unwrap(), Status::BadArgument);
    }

    #[test]
    fn a_panic_in_the_stream_host_poisons_the_instance_when_the_scope_unwinds() {
        // Arrange
        let engine = engine();
        let (mut guest, _, stream) = with_stream(&engine, HEADER_WRITER);

        // Act
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            guest
                .enter(Panicking)
                .on_request_headers(stream, 0, true)
                .map(|_| ())
        }));

        // Assert
        assert!(outcome.is_err());
        assert!(guest.instance().is_poisoned());
        assert!(matches!(
            guest.enter_root().on_done(stream),
            Err(Error::Poisoned)
        ));
    }

    #[test]
    fn a_caught_panic_poisons_the_instance_on_the_next_callback_and_on_finish() {
        // Arrange
        let engine = engine();
        let (mut guest, _, stream) = with_stream(&engine, HEADER_WRITER);
        let mut scope = guest.enter(Panicking);
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            scope.on_request_headers(stream, 0, true)
        }));
        let unpoisoned = !scope.guest().instance().is_poisoned();

        // Act
        let next = scope.on_done(stream);

        // Assert
        assert!(outcome.is_err());
        assert!(unpoisoned);
        assert!(matches!(next, Err(Error::Poisoned)));
        let _stream = scope.finish();
        assert!(guest.instance().is_poisoned());
    }

    #[test]
    fn a_panic_outside_a_callback_leaves_the_instance_usable() {
        // Arrange
        let engine = engine();
        let (mut guest, _, stream) = with_stream(&engine, RECORDER);

        // Act
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            let _scope = guest.enter(RecordingStream::new());
            panic!("embedder code failed between callbacks")
        }));

        // Assert
        assert!(outcome.is_err());
        assert!(!guest.instance().is_poisoned());
        assert!(guest.enter_root().on_done(stream).unwrap());
    }

    #[test]
    fn a_trap_poisons_the_instance_for_every_later_callback() {
        // Arrange
        let engine = engine();
        let (mut guest, stream) = trapped(&engine);
        let mut scope = guest.enter(RecordingStream::new());

        // Act
        let results = (
            scope.on_context_create(None),
            scope.on_vm_start(id(9)),
            scope.on_configure(id(9), Plugin::new()),
            scope.on_request_headers(id(9), 0, true),
            scope.on_done(id(9)),
            scope.on_log(id(9)),
            scope.on_delete(id(9)),
        );

        // Assert
        assert!(matches!(results.0, Err(Error::Poisoned)));
        assert!(matches!(results.1, Err(Error::Poisoned)));
        assert!(matches!(results.2, Err(Error::Poisoned)));
        assert!(matches!(results.3, Err(Error::Poisoned)));
        assert!(matches!(results.4, Err(Error::Poisoned)));
        assert!(matches!(results.5, Err(Error::Poisoned)));
        assert!(matches!(results.6, Err(Error::Poisoned)));
        assert_eq!(
            scope.guest().context_state(stream),
            Some(ContextState::Active)
        );
    }

    #[test]
    fn finish_returns_the_stream_of_a_poisoned_instance() {
        // Arrange
        let engine = engine();
        let (mut guest, _) = trapped(&engine);
        let scope = guest
            .enter(RecordingStream::new().with_map(MapType::HttpResponseHeaders, &[("a", "1")]));

        // Act
        let recording = scope.finish();

        // Assert
        assert_eq!(
            recording.pairs(MapType::HttpResponseHeaders),
            vec![("a".into(), "1".into())]
        );
        assert!(guest.instance().is_poisoned());
    }

    #[test]
    fn no_stream_is_a_stream_host_that_serves_nothing() {
        // Arrange
        let engine = engine();
        let (mut guest, _, stream) = with_stream(&engine, HEADER_WRITER);
        let mut scope = guest.enter(NoStream);

        // Act
        let action = scope.on_request_headers(stream, 0, true);

        // Assert
        assert_eq!(action.unwrap(), Action::Continue);
        assert_eq!(scope.finish(), NoStream);
    }

    #[test]
    fn a_scope_debugs_its_guest_and_stream_type() {
        // Arrange
        let engine = engine();
        let (mut guest, _) = with_root(&engine, RECORDER);
        let scope = guest.enter(NoStream);

        // Act
        let text = format!("{scope:?}");

        // Assert
        assert!(text.starts_with(
            "CallScope { guest: Guest { abi: V0_2_1, effective_context: Some(ContextId(1))"
        ));
        assert!(text.ends_with("NoStream\" }"));
    }

    impl<H: StreamHost> CallScope<'_, H> {
        fn guest_mut(&mut self) -> &mut Guest {
            self.guest
        }
    }

    const CONFIGURATION_READER: &str = r#"(module
        (import "env" "proxy_get_buffer_bytes" (func $get (param i32 i32 i32 i32 i32) (result i32)))
        (memory (export "memory") 1)
        (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 4096)
        (func (export "proxy_abi_version_0_2_1"))
        (func (export "proxy_on_configure") (param i32 i32) (result i32)
            (i32.store (i32.const 0)
                (call $get (i32.const 7) (i32.const 0) (i32.const -1)
                    (i32.const 64) (i32.const 68)))
            i32.const 1))"#;

    #[test]
    fn a_guest_reads_its_plugin_configuration_inside_configure() {
        // Arrange
        let engine = engine();
        let (mut guest, root) = with_root(&engine, CONFIGURATION_READER);
        let plugin = Plugin::new().with_configuration(b"plugin bytes".to_vec());

        // Act
        let configured = guest.enter_root().on_configure(root, plugin).unwrap();

        // Assert
        assert!(configured);
        assert_eq!(recorded(&mut guest, 0), 0);
        let (address, size) = (recorded(&mut guest, 64), recorded(&mut guest, 68));
        let slice = crate::runtime::GuestSlice::new(GuestPtr::from_address(address), size).unwrap();
        assert_eq!(
            guest.instance_mut().memory().unwrap().read(slice).unwrap(),
            b"plugin bytes"
        );
    }

    const TICKER: &str = r#"(module
        (import "env" "proxy_set_tick_period_milliseconds" (func $tick (param i32) (result i32)))
        (memory (export "memory") 1)
        (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 4096)
        (func (export "proxy_abi_version_0_2_1"))
        (func (export "proxy_on_configure") (param i32 i32) (result i32)
            (i32.store (i32.const 0) (call $tick (i32.const 250)))
            i32.const 1))"#;

    #[test]
    fn a_period_a_guest_sets_is_reported_through_the_guest() {
        // Arrange
        let engine = engine();
        let (mut guest, root) = with_root(&engine, TICKER);

        // Act
        let configured = guest
            .enter_root()
            .on_configure(root, Plugin::new())
            .unwrap();

        // Assert
        assert!(configured);
        assert_eq!(recorded(&mut guest, 0), 0);
        assert_eq!(guest.tick_period(root), Some(Duration::from_millis(250)));
    }
}
