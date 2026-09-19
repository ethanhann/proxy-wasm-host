//! The stream state a scope left behind.
//!
//! A scope that drops without a finish leaves its value on the guest rather
//! than in the store, so no host function can reach it and a caller that
//! returned early can still read what it lent.

use crate::abi::v0_2_1::{Guest, StreamState};

impl Guest {
    /// Keeps the stream state a scope left behind.
    pub(crate) fn detach(&mut self, stream: Box<dyn StreamState>) {
        if self.detached.is_some() {
            tracing::warn!("a stream state left by an earlier scope was replaced");
        }
        self.detached = Some(stream);
    }

    /// Drops a stream state that nobody took back before the next scope.
    pub(crate) fn discard_detached(&mut self) {
        if self.detached.take().is_some() {
            tracing::warn!("a stream state left by an earlier scope was dropped untaken");
        }
    }

    /// The stream state a scope left behind, as the type it was entered with.
    ///
    /// A scope that drops without [`crate::abi::v0_2_1::CallScope::finish`]
    /// leaves its value here, so if you returned early through the question
    /// mark operator, you can still read the request you lent.
    /// A value stays until the next [`Guest::enter`] or until this guest
    /// drops.
    ///
    /// A type that does not match leaves the value where it is, so a later
    /// call with the right type still finds it.
    /// [`Guest::take_stream_any`] reads it without naming the type.
    pub fn take_stream<H: StreamState>(&mut self) -> Option<H> {
        let held: &dyn std::any::Any = self.detached.as_deref()?;
        if !held.is::<H>() {
            tracing::warn!(
                expected = std::any::type_name::<H>(),
                "the detached stream state is another type, and it is left where it is"
            );
            return None;
        }
        let boxed: Box<dyn std::any::Any + Send> = self.detached.take()?;
        match boxed.downcast::<H>() {
            Ok(value) => Some(*value),
            Err(_) => unreachable!("the type was checked before the value was taken"),
        }
    }

    /// The stream state a scope left behind, without naming its type.
    pub fn take_stream_any(&mut self) -> Option<Box<dyn StreamState>> {
        self.detached.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::v0_2_1::test_support::{RecordingStream, status};
    use crate::abi::v0_2_1::test_support::{engine, services, wat_bytes};
    use crate::abi::v0_2_1::types::Status;
    use crate::abi::v0_2_1::{ContextId, NoStream};
    use crate::runtime::{Engine, Limits, Module};

    /// A guest that writes a request header in its callback and through an
    /// export of its own.
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

    /// A stream state with no data that is not [`NoStream`].
    #[derive(Debug, PartialEq)]
    struct Bare;

    impl StreamState for Bare {}

    fn with_stream(engine: &Engine) -> (Guest, ContextId) {
        let module = Module::new(engine, &wat_bytes(HEADER_WRITER)).unwrap();
        let mut guest = Guest::new(
            &crate::abi::v0_2_1::Host::new(engine).unwrap(),
            &module,
            services(),
            &Limits::default(),
        )
        .unwrap();
        let root = guest.enter_root().on_context_create(None).unwrap();
        let stream = guest.enter_root().on_context_create(Some(root)).unwrap();
        (guest, stream)
    }

    /// A guest whose dropped scope ran one callback on a recording stream.
    fn with_detached(engine: &Engine) -> Guest {
        let (mut guest, stream) = with_stream(engine);
        let mut scope = guest.enter(RecordingStream::new());
        scope.on_request_headers(stream, 0, true).unwrap();
        drop(scope);
        guest
    }

    #[test]
    fn a_dropped_scope_leaves_the_stream_state_recoverable() {
        // Arrange
        let engine = engine();
        let mut guest = with_detached(&engine);

        // Act
        let recovered = guest.take_stream::<RecordingStream>();

        // Assert
        assert_eq!(recovered.map(|s| s.calls().len()), Some(1));
        assert!(guest.take_stream_any().is_none());
    }

    #[test]
    fn a_take_that_names_the_wrong_type_leaves_the_value() {
        // Arrange
        let engine = engine();
        let mut guest = with_detached(&engine);

        // Act
        let refused = guest.take_stream::<Bare>();

        // Assert
        assert_eq!(refused, None);
        assert!(guest.take_stream::<RecordingStream>().is_some());
    }

    #[test]
    fn the_untyped_take_returns_a_value_the_typed_take_refused() {
        // Arrange
        let engine = engine();
        let mut guest = with_detached(&engine);
        let refused = guest.take_stream::<Bare>();

        // Act
        let taken = guest.take_stream_any();

        // Assert
        assert_eq!(refused, None);
        let any: Box<dyn std::any::Any> = taken.unwrap();
        let recording = any.downcast::<RecordingStream>().unwrap();
        assert_eq!(recording.calls().len(), 1);
        assert!(guest.take_stream_any().is_none());
    }

    #[test]
    fn a_root_scope_that_drops_detaches_nothing() {
        // Arrange
        let engine = engine();
        let (mut guest, _) = with_stream(&engine);
        drop(guest.enter_root());

        // Act
        let detached = guest.take_stream_any();

        // Assert
        assert!(detached.is_none());
    }

    #[test]
    fn a_stream_state_of_your_own_with_no_data_is_still_detached() {
        // Arrange
        let engine = engine();
        let (mut guest, _) = with_stream(&engine);
        drop(guest.enter(Bare));

        // Act
        let recovered = guest.take_stream::<Bare>();

        // Assert
        assert_eq!(recovered, Some(Bare));
        assert_eq!(guest.take_stream::<NoStream>(), None);
    }

    #[test]
    fn the_next_enter_drops_a_detached_value() {
        // Arrange
        let engine = engine();
        let mut guest = with_detached(&engine);
        drop(guest.enter_root());

        // Act
        let recovered = guest.take_stream::<RecordingStream>();

        // Assert
        assert!(recovered.is_none());
        assert!(guest.take_stream_any().is_none());
    }

    #[test]
    fn a_host_function_cannot_reach_a_detached_value() {
        // Arrange
        let engine = engine();
        let mut guest = with_detached(&engine);

        // Act
        let answer = guest.call_export::<(), i32>("write", ());

        // Assert
        assert_ne!(status(answer.unwrap()), Status::Ok);
        let recovered = guest.take_stream::<RecordingStream>().unwrap();
        assert_eq!(recovered.calls().len(), 1);
    }
}
