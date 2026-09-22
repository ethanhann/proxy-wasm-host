//! The callbacks of a TCP stream context.
//!
//! A TCP stream carries bytes in two directions.
//! The downstream side is the connection between the client and the proxy,
//! and the upstream side is the connection between the proxy and the
//! backend.
//! One stream context serves both sides.
//!
//! | Callback | The guest reads |
//! |---|---|
//! | `proxy_on_new_connection` | nothing |
//! | `proxy_on_downstream_data` | the `DOWNSTREAM_DATA` buffer |
//! | `proxy_on_upstream_data` | the `UPSTREAM_DATA` buffer |
//! | `proxy_on_downstream_connection_close` | nothing |
//! | `proxy_on_upstream_connection_close` | nothing |
//!
//! A context that took a callback of the HTTP family refuses every callback
//! here, because both guest SDKs stop with a panic on it.

use crate::abi::v0_2_1::call_scope::{CallScope, prologue};
use crate::abi::v0_2_1::types::{Action, BufferType, PeerType};
use crate::abi::v0_2_1::{Callback, ContextId, GuestError, StreamKind, StreamState};

impl<H: StreamState> CallScope<'_, H> {
    /// Calls `proxy_on_new_connection` on a stream context of the TCP
    /// family.
    ///
    /// The context takes the callbacks of a TCP stream from here on, and a
    /// callback of the HTTP family is refused.
    ///
    /// # Errors
    ///
    /// Returns [`GuestError::Context`] for an unknown context, a root
    /// context, or a context that serves the HTTP family,
    /// [`GuestError::GuestRejected`], [`GuestError::UnexpectedReturn`], and
    /// the [common runtime failures](CallScope#the-common-runtime-failures).
    pub fn on_new_connection(&mut self, context: ContextId) -> Result<Action, GuestError> {
        let func = self.guest.callbacks().new_connection.clone();
        self.stream_action(
            context,
            StreamKind::Tcp,
            Callback::NewConnection,
            None,
            func,
            context.wire(),
        )
    }

    /// Calls `proxy_on_downstream_data` on a stream context of the TCP
    /// family.
    ///
    /// `data_size` is the number of bytes the guest can read from the
    /// `DOWNSTREAM_DATA` buffer, so give the length your stream state holds.
    /// The crate reports a length that differs through `tracing` at the warn
    /// level when the guest reads the buffer, and the guest reads what your
    /// stream state holds.
    ///
    /// # Errors
    ///
    /// The errors of [`CallScope::on_new_connection`], and
    /// [`Error::ValueTooLarge`](crate::Error::ValueTooLarge) for a size
    /// above `i32::MAX`.
    pub fn on_downstream_data(
        &mut self,
        context: ContextId,
        data_size: u32,
        end_of_stream: bool,
    ) -> Result<Action, GuestError> {
        let size = prologue::wire_u32(data_size)?;
        let func = self.guest.callbacks().downstream_data.clone();
        let params = (context.wire(), size, i32::from(end_of_stream));
        self.stream_action(
            context,
            StreamKind::Tcp,
            Callback::DownstreamData,
            Some((BufferType::DownstreamData, data_size)),
            func,
            params,
        )
    }

    /// Calls `proxy_on_upstream_data` on a stream context of the TCP family.
    ///
    /// `data_size` is the number of bytes the guest can read from the
    /// `UPSTREAM_DATA` buffer, and the crate warns when your stream state
    /// holds another length.
    ///
    /// # Errors
    ///
    /// The errors of [`CallScope::on_downstream_data`].
    pub fn on_upstream_data(
        &mut self,
        context: ContextId,
        data_size: u32,
        end_of_stream: bool,
    ) -> Result<Action, GuestError> {
        let size = prologue::wire_u32(data_size)?;
        let func = self.guest.callbacks().upstream_data.clone();
        let params = (context.wire(), size, i32::from(end_of_stream));
        self.stream_action(
            context,
            StreamKind::Tcp,
            Callback::UpstreamData,
            Some((BufferType::UpstreamData, data_size)),
            func,
            params,
        )
    }

    /// Calls `proxy_on_downstream_connection_close` on a stream context of
    /// the TCP family.
    ///
    /// `peer` says which side closed the connection, and
    /// [`PeerType::Unknown`] is the value for a side you cannot name.
    ///
    /// # Errors
    ///
    /// Returns [`GuestError::Context`] for an unknown context, a root
    /// context, or a context that serves the HTTP family,
    /// [`GuestError::GuestRejected`], and the
    /// [common runtime failures](CallScope#the-common-runtime-failures).
    pub fn on_downstream_connection_close(
        &mut self,
        context: ContextId,
        peer: PeerType,
    ) -> Result<(), GuestError> {
        let func = self.guest.callbacks().downstream_connection_close.clone();
        let params = (context.wire(), i32::from(peer));
        self.stream_event(context, Callback::DownstreamConnectionClose, func, params)
    }

    /// Calls `proxy_on_upstream_connection_close` on a stream context of the
    /// TCP family.
    ///
    /// # Errors
    ///
    /// The errors of [`CallScope::on_downstream_connection_close`].
    pub fn on_upstream_connection_close(
        &mut self,
        context: ContextId,
        peer: PeerType,
    ) -> Result<(), GuestError> {
        let func = self.guest.callbacks().upstream_connection_close.clone();
        let params = (context.wire(), i32::from(peer));
        self.stream_event(context, Callback::UpstreamConnectionClose, func, params)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::v0_2_1::AbiAccess;
    use crate::abi::v0_2_1::call_scope::stream::tests::{answer, guest_of, with_stream, word};
    use crate::abi::v0_2_1::test_support::RecordingStream;
    use crate::abi::v0_2_1::{ContextProblem, GuestError};

    /// A guest that records each TCP callback it gets.
    ///
    /// The bases are 100 for the new connection, 200 for the downstream
    /// data, 300 for the upstream data, 400 for the downstream close, and
    /// 500 for the upstream close.
    const TCP: &str = r#"(module
        (memory (export "memory") 1)
        (global $answer (mut i32) (i32.const 0))
        (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 8192)
        (func (export "proxy_abi_version_0_2_1"))
        (func (export "set_answer") (param i32) (global.set $answer (local.get 0)))
        (func $record (param $base i32) (param $a i32) (param $b i32) (param $c i32)
            (i32.store (local.get $base) (local.get $a))
            (i32.store (i32.add (local.get $base) (i32.const 4)) (local.get $b))
            (i32.store (i32.add (local.get $base) (i32.const 8)) (local.get $c))
            (i32.store (i32.add (local.get $base) (i32.const 12))
                (i32.add (i32.load (i32.add (local.get $base) (i32.const 12))) (i32.const 1))))
        (func (export "proxy_on_new_connection") (param i32) (result i32)
            (call $record (i32.const 100) (local.get 0) (i32.const 0) (i32.const 0))
            global.get $answer)
        (func (export "proxy_on_downstream_data") (param i32 i32 i32) (result i32)
            (call $record (i32.const 200) (local.get 0) (local.get 1) (local.get 2))
            global.get $answer)
        (func (export "proxy_on_upstream_data") (param i32 i32 i32) (result i32)
            (call $record (i32.const 300) (local.get 0) (local.get 1) (local.get 2))
            global.get $answer)
        (func (export "proxy_on_downstream_connection_close") (param i32 i32)
            (call $record (i32.const 400) (local.get 0) (local.get 1) (i32.const 0)))
        (func (export "proxy_on_upstream_connection_close") (param i32 i32)
            (call $record (i32.const 500) (local.get 0) (local.get 1) (i32.const 0))))"#;

    #[test]
    fn each_tcp_callback_reaches_the_guest_with_its_parameters() {
        // Arrange
        let (mut guest, _, stream) = with_stream(TCP);
        let mut scope = guest.enter(RecordingStream::new());

        // Act
        let answers = (
            [
                scope.on_new_connection(stream),
                scope.on_downstream_data(stream, 4, false),
                scope.on_upstream_data(stream, 5, true),
            ],
            [
                scope.on_downstream_connection_close(stream, PeerType::Local),
                scope.on_upstream_connection_close(stream, PeerType::Remote),
            ],
        );

        // Assert
        assert!(
            answers
                .0
                .iter()
                .all(|answer| matches!(answer, Ok(Action::Continue))),
            "{answers:?}"
        );
        assert!(answers.1.iter().all(Result::is_ok), "{answers:?}");
        drop(scope.finish());
        let context = stream.wire().cast_unsigned();
        assert_eq!((word(&mut guest, 100), word(&mut guest, 112)), (context, 1));
        assert_eq!(
            (word(&mut guest, 204), word(&mut guest, 208)),
            (4, 0),
            "the downstream size and its end flag"
        );
        assert_eq!(
            (word(&mut guest, 304), word(&mut guest, 308)),
            (5, 1),
            "the upstream size and its end flag"
        );
        assert_eq!(word(&mut guest, 404), 1, "the peer of the downstream close");
        assert_eq!(word(&mut guest, 504), 2, "the peer of the upstream close");
    }

    #[test]
    fn a_tcp_callback_answers_the_action_of_the_guest() {
        // Arrange
        let (mut guest, _, stream) = with_stream(TCP);
        answer(&mut guest, i32::from(Action::Pause));
        let mut scope = guest.enter(RecordingStream::new());

        // Act
        let answer = scope.on_new_connection(stream);

        // Assert
        assert!(matches!(answer, Ok(Action::Pause)), "{answer:?}");
    }

    #[test]
    fn a_guest_that_exports_no_tcp_callback_continues_and_answers_ok() {
        // Arrange
        let silent = r#"(module
            (memory (export "memory") 1)
            (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 8192)
            (func (export "proxy_abi_version_0_2_1")))"#;
        let mut guest = guest_of(silent);
        let root = guest.enter_root().on_context_create(None).unwrap();
        let stream = guest.enter_root().on_context_create(Some(root)).unwrap();
        let mut scope = guest.enter(RecordingStream::new());

        // Act
        let answers = (
            scope.on_new_connection(stream),
            scope.on_downstream_connection_close(stream, PeerType::Unknown),
        );

        // Assert
        assert!(matches!(answers.0, Ok(Action::Continue)), "{answers:?}");
        assert!(answers.1.is_ok(), "{answers:?}");
    }

    #[test]
    fn a_context_of_the_tcp_family_refuses_an_http_callback() {
        // Arrange
        let (mut guest, _, stream) = with_stream(TCP);
        let mut scope = guest.enter(RecordingStream::new());
        scope.on_new_connection(stream).unwrap();

        // Act
        let answer = scope.on_request_headers(stream, 0, false);

        // Assert
        assert!(
            matches!(
                answer,
                Err(GuestError::Context {
                    problem: ContextProblem::WrongStreamKind {
                        recorded: StreamKind::Tcp,
                        attempted: StreamKind::Http
                    },
                    ..
                })
            ),
            "{answer:?}"
        );
        drop(scope.finish());
        assert_eq!(guest.context_stream_kind(stream), Some(StreamKind::Tcp));
    }

    #[test]
    fn a_declared_family_holds_from_the_first_callback() {
        // Arrange
        let (mut guest, _, stream) = with_stream(TCP);
        let declared = guest.expect_stream_kind(stream, StreamKind::Tcp);
        let mut scope = guest.enter(RecordingStream::new());

        // Act
        let answer = scope.on_request_headers(stream, 0, false);

        // Assert
        assert!(declared.is_ok(), "{declared:?}");
        assert!(
            matches!(
                answer,
                Err(GuestError::Context {
                    problem: ContextProblem::WrongStreamKind {
                        recorded: StreamKind::Tcp,
                        attempted: StreamKind::Http
                    },
                    ..
                })
            ),
            "{answer:?}"
        );
    }

    #[test]
    fn a_declaration_refuses_a_root_an_unknown_context_and_a_second_family() {
        // Arrange
        let (mut guest, root, stream) = with_stream(TCP);
        let unknown = ContextId::try_from(99).unwrap();
        guest.expect_stream_kind(stream, StreamKind::Tcp).unwrap();

        // Act
        let answers = [
            guest.expect_stream_kind(root, StreamKind::Tcp),
            guest.expect_stream_kind(unknown, StreamKind::Tcp),
            guest.expect_stream_kind(stream, StreamKind::Http),
            guest.expect_stream_kind(stream, StreamKind::Tcp),
        ];

        // Assert
        assert!(
            matches!(
                answers[0],
                Err(GuestError::Context {
                    problem: ContextProblem::NotStream,
                    ..
                })
            ),
            "{answers:?}"
        );
        assert!(
            matches!(
                answers[1],
                Err(GuestError::Context {
                    problem: ContextProblem::Unknown,
                    ..
                })
            ),
            "{answers:?}"
        );
        assert!(
            matches!(
                answers[2],
                Err(GuestError::Context {
                    problem: ContextProblem::WrongStreamKind {
                        recorded: StreamKind::Tcp,
                        attempted: StreamKind::Http
                    },
                    ..
                })
            ),
            "{answers:?}"
        );
        assert!(answers[3].is_ok(), "the same family twice is accepted");
        assert_eq!(guest.context_stream_kind(stream), Some(StreamKind::Tcp));
    }

    #[test]
    fn the_family_ends_with_the_context() {
        // Arrange
        let (mut guest, root, stream) = with_stream(TCP);
        guest.expect_stream_kind(stream, StreamKind::Tcp).unwrap();
        let mut scope = guest.enter(RecordingStream::new());
        scope.on_done(stream).unwrap();
        scope.on_delete(stream).unwrap();
        drop(scope.finish());

        // Act
        let next = guest.enter_root().on_context_create(Some(root)).unwrap();

        // Assert
        assert_eq!(
            guest.context_stream_kind(stream),
            None,
            "the family is gone"
        );
        assert_ne!(next, stream, "the table reuses no identifier");
        assert_eq!(guest.context_stream_kind(next), None);
    }

    #[test]
    fn the_family_of_a_root_and_of_an_unknown_context_is_none() {
        // Arrange
        let (guest, root, _) = with_stream(TCP);
        let unknown = ContextId::try_from(99).unwrap();

        // Act
        let answers = [
            guest.context_stream_kind(root),
            guest.context_stream_kind(unknown),
        ];

        // Assert
        assert_eq!(answers, [None, None]);
    }

    #[test]
    fn the_wrong_family_prints_both_families() {
        // Arrange
        let problem = ContextProblem::WrongStreamKind {
            recorded: StreamKind::Tcp,
            attempted: StreamKind::Http,
        };

        // Act
        let text = problem.to_string();

        // Assert
        assert_eq!(
            text,
            "is a TCP stream and took a callback of an HTTP stream"
        );
        assert_eq!(StreamKind::Http.to_string(), "an HTTP stream");
    }

    #[test]
    fn a_connection_close_is_refused_on_a_root_an_unknown_context_and_a_poisoned_guest() {
        // Arrange
        let (mut guest, root, stream) = with_stream(TCP);
        let unknown = ContextId::try_from(99).unwrap();
        let mut scope = guest.enter(RecordingStream::new());
        let refusals = [
            scope.on_downstream_connection_close(root, PeerType::Local),
            scope.on_downstream_connection_close(unknown, PeerType::Local),
        ];
        drop(scope.finish());
        guest.instance_mut().state_mut().poison();
        let mut scope = guest.enter(RecordingStream::new());

        // Act
        let poisoned = scope.on_upstream_connection_close(stream, PeerType::Local);

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
            matches!(poisoned, Err(GuestError::Runtime(crate::Error::Poisoned))),
            "{poisoned:?}"
        );
    }

    #[test]
    fn a_connection_close_is_refused_under_a_refused_root_and_for_the_other_family() {
        // Arrange
        let (mut guest, root, stream) = with_stream(TCP);
        let mut scope = guest.enter(RecordingStream::new());
        scope.on_request_headers(stream, 0, false).unwrap();
        let wrong_family = scope.on_downstream_connection_close(stream, PeerType::Local);
        drop(scope.finish());
        let (mut other, other_root, other_stream) = with_stream(TCP);
        other
            .instance_mut()
            .state_mut()
            .abi_mut()
            .contexts_mut()
            .reject(other_root);
        let mut scope = other.enter(RecordingStream::new());

        // Act
        let refused_root = scope.on_upstream_connection_close(other_stream, PeerType::Remote);

        // Assert
        assert!(
            matches!(
                wrong_family,
                Err(GuestError::Context {
                    problem: ContextProblem::WrongStreamKind {
                        recorded: StreamKind::Http,
                        attempted: StreamKind::Tcp
                    },
                    ..
                })
            ),
            "{wrong_family:?}"
        );
        assert!(
            matches!(refused_root, Err(GuestError::GuestRejected { .. })),
            "{refused_root:?}"
        );
        assert_eq!(root, other_root, "both guests number their roots the same");
    }

    #[test]
    fn a_connection_close_names_one_stream_context_for_each_scope() {
        // Arrange
        let (mut guest, root, first) = with_stream(TCP);
        let second = guest.enter_root().on_context_create(Some(root)).unwrap();
        let mut scope = guest.enter(RecordingStream::new());
        scope.on_new_connection(first).unwrap();

        // Act
        let answer = scope.on_downstream_connection_close(second, PeerType::Local);

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
    }

    #[test]
    fn an_unknown_peer_reaches_the_guest_and_a_size_above_the_maximum_is_refused() {
        // Arrange
        let (mut guest, _, stream) = with_stream(TCP);
        let too_large = i32::MAX.cast_unsigned() + 1;
        let mut scope = guest.enter(RecordingStream::new());
        scope
            .on_downstream_connection_close(stream, PeerType::Unknown)
            .unwrap();

        // Act
        let refused = scope.on_downstream_data(stream, too_large, false);

        // Assert
        assert!(
            matches!(
                refused,
                Err(GuestError::Runtime(crate::Error::ValueTooLarge { .. }))
            ),
            "{refused:?}"
        );
        drop(scope.finish());
        assert_eq!(word(&mut guest, 404), 0, "the unknown peer is zero");
        assert_eq!(word(&mut guest, 212), 0, "the data callback did not run");
    }

    #[test]
    fn an_answer_of_a_tcp_callback_that_is_not_an_action_is_an_unexpected_return() {
        // Arrange
        let (mut guest, _, stream) = with_stream(TCP);
        answer(&mut guest, 9);
        let mut scope = guest.enter(RecordingStream::new());

        // Act
        let answer = scope.on_upstream_data(stream, 0, false);

        // Assert
        assert!(
            matches!(
                answer,
                Err(GuestError::UnexpectedReturn {
                    callback: Callback::UpstreamData,
                    value: 9
                })
            ),
            "{answer:?}"
        );
    }

    #[test]
    fn a_declaration_is_refused_on_a_poisoned_guest() {
        // Arrange
        let (mut guest, _, stream) = with_stream(TCP);
        guest.instance_mut().state_mut().poison();

        // Act
        let answer = guest.expect_stream_kind(stream, StreamKind::Tcp);

        // Assert
        assert!(
            matches!(answer, Err(GuestError::Runtime(crate::Error::Poisoned))),
            "{answer:?}"
        );
        assert_eq!(
            guest.context_stream_kind(stream),
            None,
            "nothing was written"
        );
    }
}
