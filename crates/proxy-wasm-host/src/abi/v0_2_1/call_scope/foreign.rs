//! The callback through which a host calls a function of its own in a
//! guest.

use std::borrow::Cow;

use crate::abi::v0_2_1::call_scope::delivery::{Delivered, deliver};
use crate::abi::v0_2_1::call_scope::{CallScope, prologue};
use crate::abi::v0_2_1::payload::Delivery;
use crate::abi::v0_2_1::{Callback, ContextId, GuestError, StreamState};

impl<H: StreamState> CallScope<'_, H> {
    /// Calls `proxy_on_foreign_function` with the arguments of your own
    /// function.
    ///
    /// The ABI names no registry of functions, so `function_id` is a number
    /// that you and the plugin agree on.
    /// The guest reads `arguments` from the `FOREIGN_FUNCTION_ARGUMENTS`
    /// buffer while the callback runs, and it reads them in this callback
    /// alone.
    /// `proxy_get_status` answers `NOT_FOUND` inside the callback, because
    /// the ABI gives it no status.
    ///
    /// `context` may be a root context or a stream context.
    /// The scope lends one stream state whatever context you name, so a call
    /// on a root from inside a request scope lets the guest read the buffers
    /// and the properties of that request.
    ///
    /// The answer says that the guest ran.
    /// It does not say that a plugin handled `function_id`, because the ABI
    /// gives the callback no answer.
    ///
    /// # Errors
    ///
    /// Returns [`GuestError::Context`] for an unknown context and
    /// [`GuestError::GuestRejected`] when the guest refused the root of the
    /// context.
    /// Returns [`GuestError::Runtime`] with
    /// [`Error::ValueTooLarge`](crate::Error::ValueTooLarge) for arguments
    /// above `i32::MAX` bytes, and the
    /// [common runtime failures](CallScope#the-common-runtime-failures).
    pub fn on_foreign_function(
        &mut self,
        context: ContextId,
        function_id: u32,
        arguments: Cow<'_, [u8]>,
    ) -> Result<(), GuestError> {
        self.guest.require_live()?;
        prologue::require(self.guest, context)?;
        prologue::accepted(self.guest, context)?;
        let size = prologue::wire_size(arguments.len())?;
        let func = self.guest.callbacks().foreign_function.clone();
        deliver(
            self.guest,
            Delivered {
                root: context,
                callout: None,
                delivery: Delivery::foreign_arguments(arguments),
                callback: Callback::ForeignFunction,
                func,
                params: (context.wire(), function_id.cast_signed(), size),
                ends: false,
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::v0_2_1::AbiAccess;
    use crate::abi::v0_2_1::ContextProblem;
    use crate::abi::v0_2_1::call_scope::stream::tests::{guest_of, with_stream, word};
    use crate::abi::v0_2_1::test_support::{RecordingStream, status};
    use crate::abi::v0_2_1::types::Status;

    /// A guest that records the foreign function callback and probes what it
    /// can read inside it.
    ///
    /// Words 0 to 2 are the three parameters and word 12 counts the calls.
    /// Word 16 and word 20 are the status and the size of buffer 8.
    /// Word 24 is the status of `proxy_get_status`.
    /// Word 28 is the status of a read of map 6, and word 32 of buffer 4.
    /// Word 76 is the status of a read of map 0, which the stream state
    /// serves.
    /// `probe` reads buffer 8 outside the callback into word 36.
    const FOREIGN: &str = r#"(module
        (import "env" "proxy_get_buffer_status" (func $buffer (param i32 i32 i32) (result i32)))
        (import "env" "proxy_get_status" (func $status (param i32 i32 i32) (result i32)))
        (import "env" "proxy_get_header_map_size" (func $map (param i32 i32) (result i32)))
        (memory (export "memory") 1)
        (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 8192)
        (func (export "proxy_abi_version_0_2_1"))
        (func (export "proxy_on_foreign_function") (param i32 i32 i32)
            (i32.store (i32.const 0) (local.get 0))
            (i32.store (i32.const 4) (local.get 1))
            (i32.store (i32.const 8) (local.get 2))
            (i32.store (i32.const 12) (i32.add (i32.load (i32.const 12)) (i32.const 1)))
            (i32.store (i32.const 16) (call $buffer (i32.const 8) (i32.const 20) (i32.const 40)))
            (i32.store (i32.const 24) (call $status (i32.const 44) (i32.const 48) (i32.const 52)))
            (i32.store (i32.const 28) (call $map (i32.const 6) (i32.const 56)))
            (i32.store (i32.const 32) (call $buffer (i32.const 4) (i32.const 60) (i32.const 64)))
            (i32.store (i32.const 76) (call $map (i32.const 0) (i32.const 80))))
        (func (export "probe") (result i32)
            (i32.store (i32.const 36) (call $buffer (i32.const 8) (i32.const 68) (i32.const 72)))
            i32.const 0))"#;

    fn arguments() -> Cow<'static, [u8]> {
        Cow::Borrowed(b"hello")
    }

    #[test]
    fn a_foreign_call_reaches_a_stream_context_with_its_arguments() {
        // Arrange
        let (mut guest, _, stream) = with_stream(FOREIGN);
        let mut scope = guest.enter(RecordingStream::new());

        // Act
        let answer = scope.on_foreign_function(stream, 7, arguments());

        // Assert
        assert!(answer.is_ok(), "{answer:?}");
        drop(scope.finish());
        let recorded = [
            word(&mut guest, 0),
            word(&mut guest, 4),
            word(&mut guest, 8),
        ];
        assert_eq!(
            recorded,
            [stream.wire().cast_unsigned(), 7, 5],
            "the context, the function, and the size"
        );
        assert_eq!(status(word(&mut guest, 16).cast_signed()), Status::Ok);
        assert_eq!(word(&mut guest, 20), 5, "buffer eight holds the arguments");
    }

    #[test]
    fn a_foreign_call_reaches_a_root_context() {
        // Arrange
        let mut guest = guest_of(FOREIGN);
        let root = guest.enter_root().on_context_create(None).unwrap();
        let mut scope = guest.enter_root();

        // Act
        let answer = scope.on_foreign_function(root, 1, arguments());

        // Assert
        assert!(answer.is_ok(), "{answer:?}");
        drop(scope);
        assert_eq!(word(&mut guest, 0), root.wire().cast_unsigned());
        assert_eq!(word(&mut guest, 12), 1);
    }

    #[test]
    fn a_function_identifier_above_the_signed_maximum_reaches_the_guest_whole() {
        // Arrange
        let (mut guest, _, stream) = with_stream(FOREIGN);
        let mut scope = guest.enter(RecordingStream::new());

        // Act
        let answer = scope.on_foreign_function(stream, u32::MAX, arguments());

        // Assert
        assert!(answer.is_ok(), "{answer:?}");
        drop(scope.finish());
        assert_eq!(word(&mut guest, 4), u32::MAX);
    }

    #[test]
    fn the_status_and_the_other_delivery_values_are_absent_inside_the_callback() {
        // Arrange
        let (mut guest, _, stream) = with_stream(FOREIGN);
        let mut scope = guest.enter(RecordingStream::new());

        // Act
        let answer = scope.on_foreign_function(stream, 1, arguments());

        // Assert
        assert!(answer.is_ok(), "{answer:?}");
        drop(scope.finish());
        assert_eq!(
            status(word(&mut guest, 24).cast_signed()),
            Status::NotFound,
            "the ABI gives this callback no status"
        );
        assert_eq!(
            status(word(&mut guest, 28).cast_signed()),
            Status::BadArgument,
            "a callout map is not available"
        );
        assert_eq!(
            status(word(&mut guest, 32).cast_signed()),
            Status::NotFound,
            "the body of an HTTP call response is not available"
        );
    }

    #[test]
    fn the_arguments_are_gone_when_the_callback_returns() {
        // Arrange
        let (mut guest, _, stream) = with_stream(FOREIGN);
        let mut scope = guest.enter(RecordingStream::new());
        scope.on_foreign_function(stream, 1, arguments()).unwrap();
        drop(scope.finish());

        // Act
        let result = guest.call_export::<(), i32>("probe", ());

        // Assert
        assert!(result.is_ok(), "{result:?}");
        assert_eq!(status(word(&mut guest, 36).cast_signed()), Status::NotFound);
    }

    #[test]
    fn a_foreign_call_is_refused_for_an_unknown_context_and_a_refused_root() {
        // Arrange
        let (mut guest, root, stream) = with_stream(FOREIGN);
        let unknown = ContextId::try_from(99).unwrap();
        guest
            .instance_mut()
            .state_mut()
            .abi_mut()
            .contexts_mut()
            .reject(root);
        let mut scope = guest.enter(RecordingStream::new());

        // Act
        let answers = [
            scope.on_foreign_function(unknown, 1, arguments()),
            scope.on_foreign_function(stream, 1, arguments()),
        ];

        // Assert
        assert!(
            matches!(
                answers[0],
                Err(GuestError::Context {
                    problem: ContextProblem::Unknown,
                    ..
                })
            ),
            "{answers:?}"
        );
        assert!(
            matches!(answers[1], Err(GuestError::GuestRejected { .. })),
            "{answers:?}"
        );
        drop(scope.finish());
        assert_eq!(word(&mut guest, 12), 0, "the guest did not run");
    }

    #[test]
    fn a_host_function_inside_the_callback_sees_no_callout() {
        // Arrange
        let (mut guest, _, stream) = with_stream(FOREIGN);
        let mut scope = guest.enter(RecordingStream::new());

        // Act
        let answer = scope.on_foreign_function(stream, 1, arguments());

        // Assert
        assert!(answer.is_ok(), "{answer:?}");
        let stream_state = scope.finish();
        let calls = stream_state.calls();
        assert!(!calls.is_empty(), "the guest read a map of the embedder");
        assert!(
            calls.iter().all(|call| call.0.callout.is_none()),
            "{calls:?}"
        );
    }
}
