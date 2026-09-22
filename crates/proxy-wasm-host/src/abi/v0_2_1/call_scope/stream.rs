//! The callbacks of an HTTP stream context.
//!
//! A stream context serves one family of stream, and the crate records the
//! family at the first callback that reaches the guest.
//! A callback of the other family is refused, because both guest SDKs stop
//! with a panic on it.
//! The TCP family is in `tcp.rs`.

use wasmtime::{TypedFunc, WasmParams};

use crate::abi::v0_2_1::AbiAccess;
use crate::abi::v0_2_1::call_scope::{CallScope, prologue};
use crate::abi::v0_2_1::types::{Action, BufferType};
use crate::abi::v0_2_1::{
    Callback, ContextId, ContextProblem, GuestError, StreamKind, StreamState,
};

impl<H: StreamState> CallScope<'_, H> {
    /// Refuses a stream callback that names another context than the one
    /// this scope serves, and records the context of the first one.
    ///
    /// The stream state you lent belongs to one request, and a callback of
    /// another request would give the guest the data of the wrong one.
    fn require_served(&mut self, context: ContextId) -> Result<(), GuestError> {
        match self.served {
            Some(lent) if lent != context => Err(GuestError::Context {
                id: context,
                problem: ContextProblem::OtherStream { lent },
            }),
            _ => {
                self.served = Some(context);
                Ok(())
            }
        }
    }

    /// Declares the family of stream that `context` serves, as
    /// [`Guest::expect_stream_kind`](crate::abi::v0_2_1::Guest::expect_stream_kind)
    /// does.
    ///
    /// You create a stream context and declare its family in one scope, so
    /// the record holds from the first callback of that context.
    ///
    /// # Errors
    ///
    /// The errors of
    /// [`Guest::expect_stream_kind`](crate::abi::v0_2_1::Guest::expect_stream_kind).
    pub fn expect_stream_kind(
        &mut self,
        context: ContextId,
        kind: StreamKind,
    ) -> Result<(), GuestError> {
        self.guest.expect_stream_kind(context, kind)
    }

    /// Runs one stream callback that answers an action.
    ///
    /// The checks run in the order every callback of a context uses, and the
    /// family is recorded last, so a callback that a check refuses records
    /// none.
    /// `announced` names the buffer whose length the crate compares with the
    /// count while the callback runs.
    pub(super) fn stream_action<P: WasmParams>(
        &mut self,
        context: ContextId,
        kind: StreamKind,
        callback: Callback,
        announced: Option<(BufferType, u32)>,
        func: Option<TypedFunc<P, i32>>,
        params: P,
    ) -> Result<Action, GuestError> {
        self.guest.require_live()?;
        prologue::require_stream(self.guest, context)?;
        prologue::accepted(self.guest, context)?;
        prologue::require_stream_kind(self.guest, context, kind)?;
        self.require_served(context)?;
        prologue::record_stream_kind(self.guest, context, kind);
        let default = i32::from(Action::Continue);
        self.guest
            .instance_mut()
            .state_mut()
            .abi_mut()
            .set_announced(announced);
        let value = prologue::run(self.guest, context, callback, func, params, default);
        self.guest
            .instance_mut()
            .state_mut()
            .abi_mut()
            .set_announced(None);
        let value = value?;
        Action::try_from(value).map_err(|_| GuestError::UnexpectedReturn { callback, value })
    }

    /// Runs one stream callback of the TCP family that answers nothing.
    pub(super) fn stream_event<P: WasmParams>(
        &mut self,
        context: ContextId,
        callback: Callback,
        func: Option<TypedFunc<P, ()>>,
        params: P,
    ) -> Result<(), GuestError> {
        let kind = StreamKind::Tcp;
        self.guest.require_live()?;
        prologue::require_stream(self.guest, context)?;
        prologue::accepted(self.guest, context)?;
        prologue::require_stream_kind(self.guest, context, kind)?;
        self.require_served(context)?;
        prologue::record_stream_kind(self.guest, context, kind);
        prologue::run(self.guest, context, callback, func, params, ())?;
        Ok(())
    }

    /// Calls `proxy_on_request_headers` on a stream context of the HTTP
    /// family.
    ///
    /// The context takes the callbacks of an HTTP stream from here on, and a
    /// callback of the TCP family is refused.
    ///
    /// # Errors
    ///
    /// Returns [`GuestError::Context`] for an unknown context, a root
    /// context, or a context that serves the TCP family,
    /// [`GuestError::GuestRejected`], and [`GuestError::UnexpectedReturn`].
    /// Returns [`GuestError::Runtime`] with
    /// [`Error::ValueTooLarge`](crate::Error::ValueTooLarge) for a count
    /// above `i32::MAX`, and the
    /// [common runtime failures](CallScope#the-common-runtime-failures).
    pub fn on_request_headers(
        &mut self,
        context: ContextId,
        num_headers: u32,
        end_of_stream: bool,
    ) -> Result<Action, GuestError> {
        let count = prologue::wire_u32(num_headers)?;
        let func = self.guest.callbacks().request_headers.clone();
        let params = (context.wire(), count, i32::from(end_of_stream));
        self.stream_action(
            context,
            StreamKind::Http,
            Callback::RequestHeaders,
            None,
            func,
            params,
        )
    }

    /// Calls `proxy_on_request_body` on a stream context of the HTTP family.
    ///
    /// `body_size` is the number of bytes the guest can read from the
    /// `HTTP_REQUEST_BODY` buffer, so give the length your stream state
    /// holds.
    /// The crate reports a length that differs through `tracing` at the warn
    /// level when the guest reads the buffer, and the guest reads what your
    /// stream state holds.
    ///
    /// # Errors
    ///
    /// The errors of [`CallScope::on_request_headers`].
    pub fn on_request_body(
        &mut self,
        context: ContextId,
        body_size: u32,
        end_of_stream: bool,
    ) -> Result<Action, GuestError> {
        let size = prologue::wire_u32(body_size)?;
        let func = self.guest.callbacks().request_body.clone();
        let params = (context.wire(), size, i32::from(end_of_stream));
        self.stream_action(
            context,
            StreamKind::Http,
            Callback::RequestBody,
            Some((BufferType::HttpRequestBody, body_size)),
            func,
            params,
        )
    }

    /// Calls `proxy_on_request_trailers` on a stream context of the HTTP
    /// family.
    ///
    /// # Errors
    ///
    /// The errors of [`CallScope::on_request_headers`].
    pub fn on_request_trailers(
        &mut self,
        context: ContextId,
        num_trailers: u32,
    ) -> Result<Action, GuestError> {
        let count = prologue::wire_u32(num_trailers)?;
        let func = self.guest.callbacks().request_trailers.clone();
        let params = (context.wire(), count);
        self.stream_action(
            context,
            StreamKind::Http,
            Callback::RequestTrailers,
            None,
            func,
            params,
        )
    }

    /// Calls `proxy_on_response_headers` on a stream context of the HTTP
    /// family.
    ///
    /// # Errors
    ///
    /// The errors of [`CallScope::on_request_headers`].
    pub fn on_response_headers(
        &mut self,
        context: ContextId,
        num_headers: u32,
        end_of_stream: bool,
    ) -> Result<Action, GuestError> {
        let count = prologue::wire_u32(num_headers)?;
        let func = self.guest.callbacks().response_headers.clone();
        let params = (context.wire(), count, i32::from(end_of_stream));
        self.stream_action(
            context,
            StreamKind::Http,
            Callback::ResponseHeaders,
            None,
            func,
            params,
        )
    }

    /// Calls `proxy_on_response_body` on a stream context of the HTTP
    /// family.
    ///
    /// `body_size` is the number of bytes the guest can read from the
    /// `HTTP_RESPONSE_BODY` buffer, and the crate warns when your stream
    /// state holds another length.
    ///
    /// # Errors
    ///
    /// The errors of [`CallScope::on_request_headers`].
    pub fn on_response_body(
        &mut self,
        context: ContextId,
        body_size: u32,
        end_of_stream: bool,
    ) -> Result<Action, GuestError> {
        let size = prologue::wire_u32(body_size)?;
        let func = self.guest.callbacks().response_body.clone();
        let params = (context.wire(), size, i32::from(end_of_stream));
        self.stream_action(
            context,
            StreamKind::Http,
            Callback::ResponseBody,
            Some((BufferType::HttpResponseBody, body_size)),
            func,
            params,
        )
    }

    /// Calls `proxy_on_response_trailers` on a stream context of the HTTP
    /// family.
    ///
    /// # Errors
    ///
    /// The errors of [`CallScope::on_request_headers`].
    pub fn on_response_trailers(
        &mut self,
        context: ContextId,
        num_trailers: u32,
    ) -> Result<Action, GuestError> {
        let count = prologue::wire_u32(num_trailers)?;
        let func = self.guest.callbacks().response_trailers.clone();
        let params = (context.wire(), count);
        self.stream_action(
            context,
            StreamKind::Http,
            Callback::ResponseTrailers,
            None,
            func,
            params,
        )
    }
}

#[cfg(test)]
pub(super) mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::Error;
    use crate::abi::v0_2_1::test_support::{RecordingSink, RecordingStream, engine, wat_bytes};
    use crate::abi::v0_2_1::{Guest, Host, VmServices};
    use crate::runtime::{GuestPtr, Limits, Module};

    /// A guest that records each HTTP callback it gets.
    ///
    /// Each callback writes its parameters at its own base and counts itself
    /// in the fourth word there.
    /// The bases are 100 for the request headers, 200 for the request body,
    /// 300 for the request trailers, 400 for the response headers, 500 for
    /// the response body, and 600 for the response trailers.
    /// `set_answer` gives the action that every callback answers.
    const HTTP: &str = r#"(module
        (memory (export "memory") 1)
        (global $answer (mut i32) (i32.const 0))
        (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 8192)
        (func (export "proxy_abi_version_0_2_1"))
        (func (export "set_answer") (param i32) (global.set $answer (local.get 0)))
        (func $record (param $base i32) (param $a i32) (param $b i32) (param $c i32) (result i32)
            (i32.store (local.get $base) (local.get $a))
            (i32.store (i32.add (local.get $base) (i32.const 4)) (local.get $b))
            (i32.store (i32.add (local.get $base) (i32.const 8)) (local.get $c))
            (i32.store (i32.add (local.get $base) (i32.const 12))
                (i32.add (i32.load (i32.add (local.get $base) (i32.const 12))) (i32.const 1)))
            global.get $answer)
        (func (export "proxy_on_request_headers") (param i32 i32 i32) (result i32)
            (call $record (i32.const 100) (local.get 0) (local.get 1) (local.get 2)))
        (func (export "proxy_on_request_body") (param i32 i32 i32) (result i32)
            (call $record (i32.const 200) (local.get 0) (local.get 1) (local.get 2)))
        (func (export "proxy_on_request_trailers") (param i32 i32) (result i32)
            (call $record (i32.const 300) (local.get 0) (local.get 1) (i32.const 0)))
        (func (export "proxy_on_response_headers") (param i32 i32 i32) (result i32)
            (call $record (i32.const 400) (local.get 0) (local.get 1) (local.get 2)))
        (func (export "proxy_on_response_body") (param i32 i32 i32) (result i32)
            (call $record (i32.const 500) (local.get 0) (local.get 1) (local.get 2)))
        (func (export "proxy_on_response_trailers") (param i32 i32) (result i32)
            (call $record (i32.const 600) (local.get 0) (local.get 1) (i32.const 0))))"#;

    /// A guest that exports no callback of either family.
    const SILENT: &str = r#"(module
        (memory (export "memory") 1)
        (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 8192)
        (func (export "proxy_abi_version_0_2_1")))"#;

    pub(crate) fn guest_of(wat: &str) -> Guest {
        let engine = engine();
        let host = Host::new(&engine).unwrap();
        let module = Module::new(&engine, &wat_bytes(wat)).unwrap();
        let services = VmServices::new(Arc::new(RecordingSink::default()));
        Guest::new(&host, &module, services, &Limits::default()).unwrap()
    }

    /// A guest of `wat` with a root context and a stream context.
    pub(crate) fn with_stream(wat: &str) -> (Guest, ContextId, ContextId) {
        let mut guest = guest_of(wat);
        let root = guest.enter_root().on_context_create(None).unwrap();
        let stream = guest.enter_root().on_context_create(Some(root)).unwrap();
        (guest, root, stream)
    }

    pub(crate) fn word(guest: &mut Guest, at: u32) -> u32 {
        guest
            .instance_mut()
            .memory()
            .unwrap()
            .read_u32(GuestPtr::from_address(at))
            .unwrap()
    }

    pub(crate) fn answer(guest: &mut Guest, value: i32) {
        guest
            .instance_mut()
            .call::<i32, ()>("set_answer", value)
            .unwrap();
    }

    #[test]
    fn each_http_callback_reaches_the_guest_with_its_parameters() {
        // Arrange
        let (mut guest, _, stream) = with_stream(HTTP);
        let mut scope = guest.enter(RecordingStream::new());

        // Act
        let answers = [
            scope.on_request_headers(stream, 1, false),
            scope.on_request_body(stream, 2, true),
            scope.on_request_trailers(stream, 3),
            scope.on_response_headers(stream, 4, false),
            scope.on_response_body(stream, 5, true),
            scope.on_response_trailers(stream, 6),
        ];

        // Assert
        assert!(
            answers
                .iter()
                .all(|answer| matches!(answer, Ok(Action::Continue))),
            "{answers:?}"
        );
        drop(scope.finish());
        let recorded = [100, 200, 300, 400, 500, 600].map(|base| {
            (
                word(&mut guest, base),
                word(&mut guest, base + 4),
                word(&mut guest, base + 8),
                word(&mut guest, base + 12),
            )
        });
        let context = stream.wire().cast_unsigned();
        assert_eq!(recorded[0], (context, 1, 0, 1));
        assert_eq!(recorded[1], (context, 2, 1, 1));
        assert_eq!(recorded[2], (context, 3, 0, 1));
        assert_eq!(recorded[3], (context, 4, 0, 1));
        assert_eq!(recorded[4], (context, 5, 1, 1));
        assert_eq!(recorded[5], (context, 6, 0, 1));
    }

    #[test]
    fn an_http_callback_answers_the_action_of_the_guest() {
        // Arrange
        let (mut guest, _, stream) = with_stream(HTTP);
        answer(&mut guest, i32::from(Action::Pause));
        let mut scope = guest.enter(RecordingStream::new());

        // Act
        let answers = [
            scope.on_request_body(stream, 0, false),
            scope.on_response_trailers(stream, 0),
        ];

        // Assert
        assert!(
            answers
                .iter()
                .all(|answer| matches!(answer, Ok(Action::Pause))),
            "{answers:?}"
        );
    }

    #[test]
    fn an_answer_that_is_not_an_action_is_an_unexpected_return() {
        // Arrange
        let (mut guest, _, stream) = with_stream(HTTP);
        answer(&mut guest, 7);
        let mut scope = guest.enter(RecordingStream::new());

        // Act
        let answer = scope.on_response_headers(stream, 0, false);

        // Assert
        assert!(
            matches!(
                answer,
                Err(GuestError::UnexpectedReturn {
                    callback: Callback::ResponseHeaders,
                    value: 7
                })
            ),
            "{answer:?}"
        );
    }

    #[test]
    fn a_guest_that_exports_no_http_callback_continues() {
        // Arrange
        let (mut guest, _, stream) = with_stream(SILENT);
        let mut scope = guest.enter(RecordingStream::new());

        // Act
        let answers = [
            scope.on_request_body(stream, 0, false),
            scope.on_request_trailers(stream, 0),
            scope.on_response_headers(stream, 0, false),
            scope.on_response_body(stream, 0, false),
            scope.on_response_trailers(stream, 0),
        ];

        // Assert
        assert!(
            answers
                .iter()
                .all(|answer| matches!(answer, Ok(Action::Continue))),
            "{answers:?}"
        );
    }

    #[test]
    fn an_http_callback_is_refused_on_a_root_an_unknown_context_and_a_poisoned_guest() {
        // Arrange
        let (mut guest, root, stream) = with_stream(HTTP);
        let unknown = ContextId::try_from(99).unwrap();
        let mut scope = guest.enter(RecordingStream::new());
        let refusals = [
            scope.on_request_body(root, 0, false),
            scope.on_request_body(unknown, 0, false),
        ];
        drop(scope.finish());
        guest.instance_mut().state_mut().poison();
        let mut scope = guest.enter(RecordingStream::new());

        // Act
        let poisoned = scope.on_request_body(stream, 0, false);

        // Assert
        assert!(
            matches!(
                refusals[0],
                Err(GuestError::Context {
                    problem: ContextProblem::NotStream,
                    ..
                })
            ),
            "{refusals:?}"
        );
        assert!(
            matches!(
                refusals[1],
                Err(GuestError::Context {
                    problem: ContextProblem::Unknown,
                    ..
                })
            ),
            "{refusals:?}"
        );
        assert!(
            matches!(poisoned, Err(GuestError::Runtime(Error::Poisoned))),
            "{poisoned:?}"
        );
    }

    #[test]
    fn a_count_above_the_wire_maximum_is_refused_before_the_guest_runs() {
        // Arrange
        let (mut guest, _, stream) = with_stream(HTTP);
        let too_large = i32::MAX.cast_unsigned() + 1;
        let mut scope = guest.enter(RecordingStream::new());

        // Act
        let answer = scope.on_request_body(stream, too_large, false);

        // Assert
        assert!(
            matches!(
                answer,
                Err(GuestError::Runtime(Error::ValueTooLarge { .. }))
            ),
            "{answer:?}"
        );
        drop(scope.finish());
        assert_eq!(word(&mut guest, 212), 0, "the guest did not run");
        assert_eq!(
            guest.context_stream_kind(stream),
            None,
            "no family recorded"
        );
    }

    #[test]
    fn a_refused_root_records_no_family() {
        // Arrange
        let (mut guest, root, stream) = with_stream(HTTP);
        guest
            .instance_mut()
            .state_mut()
            .abi_mut()
            .contexts_mut()
            .reject(root);
        let mut scope = guest.enter(RecordingStream::new());

        // Act
        let answer = scope.on_request_headers(stream, 0, false);

        // Assert
        assert!(
            matches!(answer, Err(GuestError::GuestRejected { .. })),
            "{answer:?}"
        );
        drop(scope.finish());
        assert_eq!(guest.context_stream_kind(stream), None);
    }

    #[test]
    fn a_context_of_one_family_refuses_the_callbacks_of_the_other() {
        // Arrange
        let (mut guest, _, stream) = with_stream(HTTP);
        let mut scope = guest.enter(RecordingStream::new());
        scope.on_request_headers(stream, 0, false).unwrap();

        // Act
        let answer = scope.on_downstream_data(stream, 0, false);

        // Assert
        assert!(
            matches!(
                answer,
                Err(GuestError::Context {
                    problem: ContextProblem::WrongStreamKind {
                        recorded: StreamKind::Http,
                        attempted: StreamKind::Tcp
                    },
                    ..
                })
            ),
            "{answer:?}"
        );
        drop(scope.finish());
        assert_eq!(guest.context_stream_kind(stream), Some(StreamKind::Http));
    }

    #[test]
    fn two_stream_contexts_of_one_root_hold_their_own_families() {
        // Arrange
        let (mut guest, root, first) = with_stream(HTTP);
        let second = guest.enter_root().on_context_create(Some(root)).unwrap();
        let mut scope = guest.enter(RecordingStream::new());
        scope.on_request_headers(first, 0, false).unwrap();
        drop(scope.finish());
        let mut scope = guest.enter(RecordingStream::new());

        // Act
        let answer = scope.on_new_connection(second);

        // Assert
        assert!(matches!(answer, Ok(Action::Continue)), "{answer:?}");
        drop(scope.finish());
        assert_eq!(guest.context_stream_kind(first), Some(StreamKind::Http));
        assert_eq!(guest.context_stream_kind(second), Some(StreamKind::Tcp));
    }

    #[test]
    fn a_scope_serves_the_stream_context_of_its_first_callback_alone() {
        // Arrange
        let (mut guest, root, first) = with_stream(HTTP);
        let second = guest.enter_root().on_context_create(Some(root)).unwrap();
        let mut scope = guest.enter(RecordingStream::new());
        scope.on_request_headers(first, 0, false).unwrap();

        // Act
        let answer = scope.on_request_body(second, 0, false);

        // Assert
        assert!(
            matches!(
                answer,
                Err(GuestError::Context {
                    problem: ContextProblem::OtherStream { lent },
                    ..
                }) if lent == first
            ),
            "{answer:?}"
        );
        drop(scope.finish());
        assert_eq!(word(&mut guest, 212), 0, "the guest did not run");
        assert_eq!(
            guest.context_stream_kind(second),
            None,
            "the refused callback recorded no family"
        );
    }

    #[test]
    fn a_new_scope_serves_a_new_stream_context() {
        // Arrange
        let (mut guest, root, first) = with_stream(HTTP);
        let second = guest.enter_root().on_context_create(Some(root)).unwrap();
        let mut scope = guest.enter(RecordingStream::new());
        scope.on_request_headers(first, 0, false).unwrap();
        drop(scope.finish());
        let mut scope = guest.enter(RecordingStream::new());

        // Act
        let answer = scope.on_request_headers(second, 0, false);

        // Assert
        assert!(matches!(answer, Ok(Action::Continue)), "{answer:?}");
        drop(scope.finish());
        assert_eq!(word(&mut guest, 112), 2, "both contexts reached the guest");
    }

    #[test]
    fn a_scope_declares_the_family_of_the_context_it_creates() {
        // Arrange
        let (mut guest, root, _) = with_stream(HTTP);
        let mut scope = guest.enter(RecordingStream::new());
        let stream = scope.on_context_create(Some(root)).unwrap();

        // Act
        let declared = scope.expect_stream_kind(stream, StreamKind::Http);

        // Assert
        assert!(declared.is_ok(), "{declared:?}");
        let refused = scope.on_new_connection(stream);
        assert!(
            matches!(
                refused,
                Err(GuestError::Context {
                    problem: ContextProblem::WrongStreamKind { .. },
                    ..
                })
            ),
            "{refused:?}"
        );
        drop(scope.finish());
        assert_eq!(guest.context_stream_kind(stream), Some(StreamKind::Http));
    }
}
