//! The callbacks that give the answer of a gRPC callout to the guest.

use std::borrow::Cow;

use crate::abi::v0_2_1::AbiAccess;
use crate::abi::v0_2_1::call_scope::delivery::{Delivered, deliver, deliver_grpc_close};
use crate::abi::v0_2_1::call_scope::{CallScope, prologue};
use crate::abi::v0_2_1::payload::Delivery;
use crate::abi::v0_2_1::{
    Callback, CalloutId, CalloutKind, CalloutProblem, ContextId, GrpcStatus, Guest, GuestError,
    HeaderPairs, StreamState,
};

/// The callout a delivery runs for, once the crate accepted it.
struct Accepted {
    root: ContextId,
    kind: CalloutKind,
}

/// Checks the callout, the caller, the kind, and the root of the caller.
///
/// The kinds a delivery accepts come from the callback it runs.
/// A metadata callback and a message reach a gRPC callout of either kind,
/// and an HTTP call takes none of the four.
fn accepted<const N: usize>(
    guest: &Guest,
    context: ContextId,
    callout: CalloutId,
    kinds: [CalloutKind; N],
) -> Result<Accepted, GuestError> {
    let refused = |problem| GuestError::Callout {
        id: callout,
        problem,
    };
    let entry = guest
        .instance()
        .state()
        .abi()
        .callouts()
        .get(callout)
        .ok_or_else(|| refused(CalloutProblem::NotOpen))?;
    if entry.caller != context {
        return Err(refused(CalloutProblem::NotMadeBy(context)));
    }
    if !kinds.contains(&entry.kind) {
        return Err(refused(CalloutProblem::WrongKind(entry.kind)));
    }
    prologue::require_root(guest, entry.root)?;
    prologue::accepted(guest, entry.root)?;
    Ok(Accepted {
        root: entry.root,
        kind: entry.kind,
    })
}

const BOTH_KINDS: [CalloutKind; 2] = [CalloutKind::GrpcCall, CalloutKind::GrpcStream];

impl<H: StreamState> CallScope<'_, H> {
    /// Gives the initial metadata of a gRPC callout to the guest, through
    /// `proxy_on_grpc_receive_initial_metadata`.
    ///
    /// `context` is the context that made the callout, which your
    /// [`Callouts`](crate::abi::v0_2_1::Callouts) service received in the
    /// `Invocation`.
    /// Enter this scope with the stream state of that request, or with
    /// [`Guest::enter_root`](crate::abi::v0_2_1::Guest::enter_root) for a
    /// callout that a root context made.
    ///
    /// The callout stays open, and the guest reads the pairs from
    /// `metadata` while the callback runs.
    /// It reads them in this callback alone.
    /// A key that is not UTF-8 stops a guest of the Rust SDK with a panic,
    /// so give the keys the server sent.
    ///
    /// Deliver on the guest that owns the callout.
    /// The callout identifiers of every guest start at one, and the crate
    /// checks the identifier against the table of this guest alone, so an
    /// event you deliver on another guest reaches the wrong plugin.
    ///
    /// The guest may send on the callout, cancel it, or close it from inside
    /// the callback, and your service then runs on this thread while this
    /// method has not returned.
    /// Hold no lock of your own on that callout while you deliver.
    ///
    /// # Errors
    ///
    /// Returns [`GuestError::Callout`] when the callout is not open, when
    /// `context` did not make it, or when it is an HTTP call,
    /// [`GuestError::Context`] when the root of the caller is unknown, and
    /// [`GuestError::GuestRejected`] when that root was refused.
    /// The guest does not run in those cases, and the callout stays as it
    /// was.
    /// Returns the
    /// [common runtime failures](CallScope#the-common-runtime-failures).
    pub fn on_grpc_receive_initial_metadata(
        &mut self,
        context: ContextId,
        callout: CalloutId,
        metadata: HeaderPairs<'_>,
    ) -> Result<(), GuestError> {
        self.guest.require_live()?;
        let open = accepted(self.guest, context, callout, BOTH_KINDS)?;
        let count = prologue::wire_size(metadata.len())?;
        let func = self.guest.callbacks().grpc_receive_initial_metadata.clone();
        deliver(
            self.guest,
            Delivered {
                root: open.root,
                callout,
                delivery: Delivery::grpc_initial_metadata(callout, metadata),
                callback: Callback::GrpcReceiveInitialMetadata,
                func,
                params: (open.root.wire(), callout.get().cast_signed(), count),
                ends: false,
            },
        )
    }

    /// Gives one message of a gRPC callout to the guest, through
    /// `proxy_on_grpc_receive`.
    ///
    /// A gRPC call gets one answer, so this delivery ends it.
    /// A gRPC stream gets many messages, so the callout stays open until you
    /// close it.
    /// The kind you opened therefore says whether your own record of the
    /// callout ends here.
    /// The rules of
    /// [`on_grpc_receive_initial_metadata`](CallScope::on_grpc_receive_initial_metadata)
    /// on the guest and on your locks hold for this method as well.
    /// The guest reads the bytes from the `GRPC_CALL_MESSAGE` buffer while
    /// the callback runs, and in this callback alone.
    ///
    /// # Errors
    ///
    /// Returns [`GuestError::Callout`] when the callout is not open, when
    /// `context` did not make it, or when it is an HTTP call,
    /// [`GuestError::Context`] when the root of the caller is unknown, and
    /// [`GuestError::GuestRejected`] when that root was refused.
    /// Returns [`GuestError::Runtime`] with
    /// [`Error::ValueTooLarge`](crate::Error::ValueTooLarge) for a message
    /// above `i32::MAX` bytes, which leaves the callout open, and the
    /// [common runtime failures](CallScope#the-common-runtime-failures).
    pub fn on_grpc_receive(
        &mut self,
        context: ContextId,
        callout: CalloutId,
        message: Cow<'_, [u8]>,
    ) -> Result<(), GuestError> {
        self.guest.require_live()?;
        let open = accepted(self.guest, context, callout, BOTH_KINDS)?;
        let size = prologue::wire_size(message.len())?;
        let func = self.guest.callbacks().grpc_receive.clone();
        deliver(
            self.guest,
            Delivered {
                root: open.root,
                callout,
                delivery: Delivery::grpc_message(callout, message),
                callback: Callback::GrpcReceive,
                func,
                params: (open.root.wire(), callout.get().cast_signed(), size),
                ends: open.kind == CalloutKind::GrpcCall,
            },
        )
    }

    /// Gives the trailing metadata of a gRPC callout to the guest, through
    /// `proxy_on_grpc_receive_trailing_metadata`.
    ///
    /// The callout stays open, and the guest reads the pairs in this
    /// callback alone.
    /// You end the callout with [`CallScope::on_grpc_close`].
    /// The rules of
    /// [`on_grpc_receive_initial_metadata`](CallScope::on_grpc_receive_initial_metadata)
    /// on the guest and on your locks hold for this method as well.
    ///
    /// # Errors
    ///
    /// Returns [`GuestError::Callout`] when the callout is not open, when
    /// `context` did not make it, or when it is an HTTP call,
    /// [`GuestError::Context`] when the root of the caller is unknown, and
    /// [`GuestError::GuestRejected`] when that root was refused.
    /// Returns the
    /// [common runtime failures](CallScope#the-common-runtime-failures).
    pub fn on_grpc_receive_trailing_metadata(
        &mut self,
        context: ContextId,
        callout: CalloutId,
        metadata: HeaderPairs<'_>,
    ) -> Result<(), GuestError> {
        self.guest.require_live()?;
        let open = accepted(self.guest, context, callout, BOTH_KINDS)?;
        let count = prologue::wire_size(metadata.len())?;
        let func = self
            .guest
            .callbacks()
            .grpc_receive_trailing_metadata
            .clone();
        deliver(
            self.guest,
            Delivered {
                root: open.root,
                callout,
                delivery: Delivery::grpc_trailing_metadata(callout, metadata),
                callback: Callback::GrpcReceiveTrailingMetadata,
                func,
                params: (open.root.wire(), callout.get().cast_signed(), count),
                ends: false,
            },
        )
    }

    /// Ends a gRPC callout with its status, through `proxy_on_grpc_close`.
    ///
    /// The delivery ends the callout, whether the guest exports the callback
    /// or not.
    /// The guest reads the code and the message with `proxy_get_status`
    /// while the callback runs, and it reads them in this callback alone.
    /// A guest that asks outside a delivery gets `NOT_FOUND`, which stops a
    /// plugin of the Rust SDK, so a plugin asks for the status where the ABI
    /// says it can.
    /// A code above `i32::MAX` reaches the guest whole, because the crate
    /// passes the raw bits.
    /// Deliver on the guest that owns the callout, as
    /// [`on_grpc_receive_initial_metadata`](CallScope::on_grpc_receive_initial_metadata)
    /// says.
    ///
    /// # Errors
    ///
    /// Returns [`GuestError::Callout`] when the callout is not open, when
    /// `context` did not make it, or when it is an HTTP call,
    /// [`GuestError::Context`] when the root of the caller is unknown, and
    /// [`GuestError::GuestRejected`] when that root was refused.
    /// Returns [`GuestError::Runtime`] with
    /// [`Error::ValueTooLarge`](crate::Error::ValueTooLarge) for a message
    /// above `i32::MAX` bytes, which leaves the callout open, and the
    /// [common runtime failures](CallScope#the-common-runtime-failures).
    pub fn on_grpc_close(
        &mut self,
        context: ContextId,
        callout: CalloutId,
        status: GrpcStatus,
    ) -> Result<(), GuestError> {
        self.guest.require_live()?;
        let open = accepted(self.guest, context, callout, BOTH_KINDS)?;
        deliver_grpc_close(self.guest, open.root, callout, status)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::Error;
    use crate::abi::v0_2_1::callout::Callout;
    use crate::abi::v0_2_1::test_support::callouts::{GrpcAsk, RecordingCallouts, services_with};
    use crate::abi::v0_2_1::test_support::{RecordingSink, engine, status, wat_bytes};
    use crate::abi::v0_2_1::types::Status;
    use crate::abi::v0_2_1::{ContextProblem, Host, HttpCallResponse, OpenCallout, VmServices};
    use crate::runtime::{GuestPtr, Limits, Module};

    /// A guest that records every gRPC callback it gets.
    ///
    /// Each callback writes its three parameters at its own base and counts
    /// itself in the fourth word there.
    /// The bases are 100 for the initial metadata, 200 for a message, 300
    /// for the trailing metadata, and 400 for a close.
    /// The probes write the status and the value of each read that a
    /// callback makes, in the words from 500.
    /// `set_action` makes the message callback cancel its callout, send on
    /// it, or trap.
    const RECORDER: &str = r#"(module
        (import "env" "proxy_get_header_map_size" (func $map_size (param i32 i32) (result i32)))
        (import "env" "proxy_get_buffer_status" (func $buffer (param i32 i32 i32) (result i32)))
        (import "env" "proxy_get_status" (func $status (param i32 i32 i32) (result i32)))
        (import "env" "proxy_grpc_cancel" (func $cancel (param i32) (result i32)))
        (import "env" "proxy_grpc_send" (func $send (param i32 i32 i32 i32) (result i32)))
        (memory (export "memory") 1)
        (global $action (mut i32) (i32.const 0))
        (data (i32.const 900) "m")
        (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 8192)
        (func (export "proxy_abi_version_0_2_1"))
        (func (export "set_action") (param i32) (global.set $action (local.get 0)))
        (func $record (param $base i32) (param $a i32) (param $b i32) (param $c i32)
            (i32.store (local.get $base) (local.get $a))
            (i32.store (i32.add (local.get $base) (i32.const 4)) (local.get $b))
            (i32.store (i32.add (local.get $base) (i32.const 8)) (local.get $c))
            (i32.store (i32.add (local.get $base) (i32.const 12))
                (i32.add (i32.load (i32.add (local.get $base) (i32.const 12))) (i32.const 1))))
        (func (export "proxy_on_grpc_receive_initial_metadata") (param i32 i32 i32)
            (call $record (i32.const 100) (local.get 0) (local.get 1) (local.get 2))
            (i32.store (i32.const 500) (call $map_size (i32.const 4) (i32.const 504))))
        (func (export "proxy_on_grpc_receive") (param i32 i32 i32)
            (call $record (i32.const 200) (local.get 0) (local.get 1) (local.get 2))
            (i32.store (i32.const 508) (call $buffer (i32.const 5) (i32.const 512) (i32.const 516)))
            (i32.store (i32.const 520) (call $map_size (i32.const 4) (i32.const 524)))
            (i32.store (i32.const 528) (call $status (i32.const 532) (i32.const 536) (i32.const 540)))
            (if (i32.eq (global.get $action) (i32.const 1))
                (then (i32.store (i32.const 600) (call $cancel (local.get 1)))))
            (if (i32.eq (global.get $action) (i32.const 2))
                (then (i32.store (i32.const 604)
                    (call $send (local.get 1) (i32.const 900) (i32.const 1) (i32.const 0)))))
            (if (i32.eq (global.get $action) (i32.const 3)) (then unreachable)))
        (func (export "proxy_on_grpc_receive_trailing_metadata") (param i32 i32 i32)
            (call $record (i32.const 300) (local.get 0) (local.get 1) (local.get 2))
            (i32.store (i32.const 544) (call $map_size (i32.const 5) (i32.const 548))))
        (func (export "proxy_on_grpc_close") (param i32 i32 i32)
            (call $record (i32.const 400) (local.get 0) (local.get 1) (local.get 2))
            (i32.store (i32.const 552) (call $status (i32.const 556) (i32.const 560) (i32.const 564)))
            (i32.store (i32.const 568) (call $buffer (i32.const 5) (i32.const 572) (i32.const 576)))
            (if (i32.eq (global.get $action) (i32.const 3)) (then unreachable))))"#;

    /// A guest that opens a gRPC stream from its tick and from its done
    /// callback.
    ///
    /// Address 0 and address 4 hold the status of each call.
    const OPENER: &str = r#"(module
        (import "env" "proxy_grpc_stream" (func $open (param i32 i32 i32 i32 i32 i32 i32 i32 i32) (result i32)))
        (memory (export "memory") 1)
        (data (i32.const 900) "authz")
        (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 8192)
        (func (export "proxy_abi_version_0_2_1"))
        (func $make (result i32)
            (call $open (i32.const 900) (i32.const 5) (i32.const 900) (i32.const 5)
                (i32.const 900) (i32.const 5) (i32.const 0) (i32.const 0) (i32.const 8)))
        (func (export "proxy_on_tick") (param i32)
            (i32.store (i32.const 0) (call $make)))
        (func (export "proxy_on_done") (param i32) (result i32)
            (i32.store (i32.const 4) (call $make))
            i32.const 1))"#;

    /// A guest that exports none of the four gRPC callbacks.
    const SILENT: &str = r#"(module
        (memory (export "memory") 1)
        (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 8192)
        (func (export "proxy_abi_version_0_2_1")))"#;

    fn guest_of(wat: &str, services: VmServices) -> Guest {
        let engine = engine();
        let host = Host::new(&engine).unwrap();
        let module = Module::new(&engine, &wat_bytes(wat)).unwrap();
        Guest::new(&host, &module, services, &Limits::default()).unwrap()
    }

    /// A recording guest, its service, and a root context.
    fn recording() -> (Guest, Arc<RecordingCallouts>, ContextId) {
        let service = Arc::new(RecordingCallouts::new());
        let mut guest = guest_of(RECORDER, services_with(service.clone()));
        let root = guest.enter_root().on_context_create(None).unwrap();
        (guest, service, root)
    }

    fn open(guest: &mut Guest, kind: CalloutKind, caller: ContextId, root: ContextId) -> CalloutId {
        let table = guest.instance_mut().state_mut().abi_mut().callouts_mut();
        let id = table.reserve();
        table.enter(id, Callout::new(kind, caller, root));
        id
    }

    fn word(guest: &mut Guest, at: u32) -> u32 {
        guest
            .instance_mut()
            .memory()
            .unwrap()
            .read_u32(GuestPtr::from_address(at))
            .unwrap()
    }

    fn action(guest: &mut Guest, value: i32) {
        guest
            .instance_mut()
            .call::<i32, ()>("set_action", value)
            .unwrap();
    }

    fn metadata() -> HeaderPairs<'static> {
        vec![(
            Cow::Borrowed(b"k".as_slice()),
            Cow::Borrowed(b"v".as_slice()),
        )]
    }

    fn message() -> Cow<'static, [u8]> {
        Cow::Borrowed(b"hello")
    }

    fn bytes(value: &'static [u8]) -> Cow<'static, [u8]> {
        Cow::Borrowed(value)
    }

    /// The status of a server that ended the callout with an error.
    fn unavailable() -> GrpcStatus {
        GrpcStatus::new(14, "unavailable")
    }

    /// The status of a callout that ended well.
    fn finished() -> GrpcStatus {
        GrpcStatus::new(0, "")
    }

    fn wrong_kind(result: &Result<(), GuestError>, kind: CalloutKind) -> bool {
        matches!(
            result,
            Err(GuestError::Callout {
                problem: CalloutProblem::WrongKind(found),
                ..
            }) if *found == kind
        )
    }

    #[test]
    fn a_call_ends_at_its_message_and_a_stream_keeps_its_entry() {
        // Arrange
        let (mut guest, _, root) = recording();
        let unary = open(&mut guest, CalloutKind::GrpcCall, root, root);
        let stream = open(&mut guest, CalloutKind::GrpcStream, root, root);
        let mut scope = guest.enter_root();

        // Act
        let answers = [
            scope.on_grpc_receive(root, unary, message()),
            scope.on_grpc_receive(root, stream, message()),
        ];

        // Assert
        assert!(answers.iter().all(Result::is_ok), "{answers:?}");
        drop(scope);
        assert_eq!(word(&mut guest, 212), 2, "both messages reached the guest");
        assert_eq!(
            guest.open_callouts(),
            [OpenCallout {
                callout: stream,
                caller: root,
                root,
                kind: CalloutKind::GrpcStream,
            }]
        );
    }

    #[test]
    fn a_close_ends_a_call_and_a_stream() {
        // Arrange
        let (mut guest, _, root) = recording();
        let unary = open(&mut guest, CalloutKind::GrpcCall, root, root);
        let stream = open(&mut guest, CalloutKind::GrpcStream, root, root);
        let mut scope = guest.enter_root();

        // Act
        let answers = [
            scope.on_grpc_close(root, unary, unavailable()),
            scope.on_grpc_close(root, stream, finished()),
        ];

        // Assert
        assert!(answers.iter().all(Result::is_ok), "{answers:?}");
        drop(scope);
        assert_eq!(word(&mut guest, 412), 2);
        assert!(guest.open_callouts().is_empty());
    }

    #[test]
    fn the_two_metadata_deliveries_keep_the_entry_of_both_grpc_kinds() {
        // Arrange
        let (mut guest, _, root) = recording();
        let unary = open(&mut guest, CalloutKind::GrpcCall, root, root);
        let stream = open(&mut guest, CalloutKind::GrpcStream, root, root);
        let mut scope = guest.enter_root();

        // Act
        let answers = [
            scope.on_grpc_receive_initial_metadata(root, unary, metadata()),
            scope.on_grpc_receive_initial_metadata(root, stream, metadata()),
            scope.on_grpc_receive_trailing_metadata(root, unary, metadata()),
            scope.on_grpc_receive_trailing_metadata(root, stream, metadata()),
        ];

        // Assert
        assert!(answers.iter().all(Result::is_ok), "{answers:?}");
        drop(scope);
        assert_eq!((word(&mut guest, 112), word(&mut guest, 312)), (2, 2));
        assert_eq!(guest.open_callout_count(), 2, "both callouts stay open");
    }

    #[test]
    fn an_http_callout_takes_none_of_the_four_deliveries() {
        // Arrange
        let (mut guest, _, root) = recording();
        let http = open(&mut guest, CalloutKind::HttpCall, root, root);
        let mut scope = guest.enter_root();

        // Act
        let answers = [
            scope.on_grpc_receive_initial_metadata(root, http, metadata()),
            scope.on_grpc_receive(root, http, message()),
            scope.on_grpc_receive_trailing_metadata(root, http, metadata()),
            scope.on_grpc_close(root, http, finished()),
        ];

        // Assert
        assert!(
            answers
                .iter()
                .all(|answer| wrong_kind(answer, CalloutKind::HttpCall)),
            "{answers:?}"
        );
        drop(scope);
        assert_eq!(word(&mut guest, 112), 0, "the guest did not run");
        assert_eq!(guest.open_callout_count(), 1);
    }

    #[test]
    fn a_grpc_callout_takes_no_http_response() {
        // Arrange
        let (mut guest, _, root) = recording();
        let stream = open(&mut guest, CalloutKind::GrpcStream, root, root);
        let mut scope = guest.enter_root();

        // Act
        let answer = scope.on_http_call_response(root, stream, HttpCallResponse::failed());

        // Assert
        assert!(wrong_kind(&answer, CalloutKind::GrpcStream), "{answer:?}");
        drop(scope);
        assert_eq!(guest.open_callout_count(), 1);
    }

    #[test]
    fn a_delivery_gives_the_guest_the_root_the_callout_and_the_count() {
        // Arrange
        let (mut guest, _, root) = recording();
        let stream = open(&mut guest, CalloutKind::GrpcStream, root, root);
        let mut scope = guest.enter_root();

        // Act
        let answer = scope.on_grpc_receive_initial_metadata(root, stream, metadata());

        // Assert
        assert!(answer.is_ok(), "{answer:?}");
        drop(scope);
        let recorded = [
            word(&mut guest, 100),
            word(&mut guest, 104),
            word(&mut guest, 108),
        ];
        assert_eq!(recorded, [root.wire().cast_unsigned(), stream.get(), 1]);
        assert_eq!(status(word(&mut guest, 500).cast_signed()), Status::Ok);
        assert_eq!(
            word(&mut guest, 504),
            16,
            "the guest read the serialized size of the one pair"
        );
    }

    #[test]
    fn a_message_is_readable_in_its_own_callback_alone() {
        // Arrange
        let (mut guest, _, root) = recording();
        let stream = open(&mut guest, CalloutKind::GrpcStream, root, root);
        let mut scope = guest.enter_root();
        let second = bytes(b"second one");
        scope
            .on_grpc_receive(root, stream, bytes(b"first"))
            .unwrap();

        // Act
        let answer = scope.on_grpc_receive(root, stream, second);

        // Assert
        assert!(answer.is_ok(), "{answer:?}");
        drop(scope);
        assert_eq!(word(&mut guest, 208), 10, "the size of the second message");
        assert_eq!(status(word(&mut guest, 508).cast_signed()), Status::Ok);
        assert_eq!(word(&mut guest, 512), 10, "buffer five holds it");
        assert_eq!(
            status(word(&mut guest, 520).cast_signed()),
            Status::BadArgument,
            "map four is not in a message delivery"
        );
    }

    #[test]
    fn a_close_answers_its_status_and_no_message_buffer() {
        // Arrange
        let (mut guest, _, root) = recording();
        let unary = open(&mut guest, CalloutKind::GrpcCall, root, root);
        let mut scope = guest.enter_root();

        // Act
        let answer = scope.on_grpc_close(root, unary, unavailable());

        // Assert
        assert!(answer.is_ok(), "{answer:?}");
        drop(scope);
        assert_eq!(word(&mut guest, 408), 14, "the code is a parameter");
        assert_eq!(status(word(&mut guest, 552).cast_signed()), Status::Ok);
        assert_eq!(word(&mut guest, 556), 14);
        assert_eq!(word(&mut guest, 564), 11, "the message size");
        assert_eq!(
            status(word(&mut guest, 568).cast_signed()),
            Status::NotFound,
            "a close holds no message"
        );
    }

    #[test]
    fn a_close_code_above_the_signed_maximum_reaches_the_guest_whole() {
        // Arrange
        let (mut guest, _, root) = recording();
        let unary = open(&mut guest, CalloutKind::GrpcCall, root, root);
        let highest_code = GrpcStatus::new(u32::MAX, "");
        let mut scope = guest.enter_root();

        // Act
        let answer = scope.on_grpc_close(root, unary, highest_code);

        // Assert
        assert!(answer.is_ok(), "{answer:?}");
        drop(scope);
        assert_eq!(word(&mut guest, 408), u32::MAX);
        assert_eq!(word(&mut guest, 556), u32::MAX);
    }

    #[test]
    fn a_grpc_delivery_the_table_does_not_allow_is_refused_and_the_guest_does_not_run() {
        // Arrange
        let (mut guest, _, root) = recording();
        let stream = guest.enter_root().on_context_create(Some(root)).unwrap();
        let mine = open(&mut guest, CalloutKind::GrpcStream, root, root);
        let unknown = CalloutId::try_from(99_u32).unwrap();
        let mut scope = guest.enter_root();

        // Act
        let answers = [
            scope.on_grpc_receive(root, unknown, message()),
            scope.on_grpc_receive(stream, mine, message()),
        ];

        // Assert
        assert!(
            matches!(
                answers[0],
                Err(GuestError::Callout {
                    problem: CalloutProblem::NotOpen,
                    ..
                })
            ),
            "{answers:?}"
        );
        assert!(
            matches!(
                answers[1],
                Err(GuestError::Callout {
                    problem: CalloutProblem::NotMadeBy(_),
                    ..
                })
            ),
            "{answers:?}"
        );
        drop(scope);
        assert_eq!(word(&mut guest, 212), 0);
    }

    #[test]
    fn a_delivery_whose_root_is_gone_is_refused() {
        // Arrange
        let (mut guest, _, root) = recording();
        let stream = guest.enter_root().on_context_create(Some(root)).unwrap();
        let callout = open(&mut guest, CalloutKind::GrpcStream, stream, root);
        guest
            .instance_mut()
            .state_mut()
            .abi_mut()
            .contexts_mut()
            .remove(root);
        let mut scope = guest.enter_root();

        // Act
        let answer = scope.on_grpc_receive(stream, callout, message());

        // Assert
        assert!(
            matches!(
                answer,
                Err(GuestError::Context {
                    problem: ContextProblem::Unknown,
                    ..
                })
            ),
            "{answer:?}"
        );
    }

    #[test]
    fn the_four_callbacks_are_refused_on_a_poisoned_guest() {
        // Arrange
        let (mut guest, _, root) = recording();
        let stream = open(&mut guest, CalloutKind::GrpcStream, root, root);
        guest.instance_mut().state_mut().poison();
        let mut scope = guest.enter_root();

        // Act
        let answers = [
            scope.on_grpc_receive_initial_metadata(root, stream, metadata()),
            scope.on_grpc_receive(root, stream, message()),
            scope.on_grpc_receive_trailing_metadata(root, stream, metadata()),
            scope.on_grpc_close(root, stream, finished()),
        ];

        // Assert
        for answer in &answers {
            assert!(
                matches!(answer, Err(GuestError::Runtime(Error::Poisoned))),
                "{answer:?}"
            );
        }
        drop(scope);
        assert_eq!(guest.open_callout_count(), 1);
    }

    #[test]
    fn a_guest_that_exports_none_of_the_four_gets_no_call_and_the_callout_still_ends() {
        // Arrange
        let service = Arc::new(RecordingCallouts::new());
        let mut guest = guest_of(SILENT, services_with(service));
        let root = guest.enter_root().on_context_create(None).unwrap();
        let unary = open(&mut guest, CalloutKind::GrpcCall, root, root);
        let mut scope = guest.enter_root();

        // Act
        let answer = scope.on_grpc_close(root, unary, finished());

        // Assert
        assert!(answer.is_ok(), "{answer:?}");
        drop(scope);
        assert!(guest.open_callouts().is_empty());
    }

    #[test]
    fn a_guest_that_cancels_its_stream_inside_a_message_ends_it() {
        // Arrange
        let (mut guest, service, root) = recording();
        let stream = open(&mut guest, CalloutKind::GrpcStream, root, root);
        action(&mut guest, 1);
        let mut scope = guest.enter_root();

        // Act
        let answer = scope.on_grpc_receive(root, stream, message());

        // Assert
        assert!(answer.is_ok(), "{answer:?}");
        drop(scope);
        assert_eq!(status(word(&mut guest, 600).cast_signed()), Status::Ok);
        assert!(guest.open_callouts().is_empty());
        assert_eq!(
            service
                .grpc_calls()
                .into_iter()
                .map(|(_, _, ask)| ask)
                .collect::<Vec<_>>(),
            vec![GrpcAsk::Cancel]
        );
        let mut scope = guest.enter_root();
        let late = scope.on_grpc_close(root, stream, finished());
        assert!(
            matches!(
                late,
                Err(GuestError::Callout {
                    problem: CalloutProblem::NotOpen,
                    ..
                })
            ),
            "{late:?}"
        );
    }

    #[test]
    fn a_guest_that_sends_inside_a_message_reaches_the_service_during_the_delivery() {
        // Arrange
        let (mut guest, service, root) = recording();
        let stream = open(&mut guest, CalloutKind::GrpcStream, root, root);
        action(&mut guest, 2);
        let mut scope = guest.enter_root();

        // Act
        let answer = scope.on_grpc_receive(root, stream, message());

        // Assert
        assert!(answer.is_ok(), "{answer:?}");
        drop(scope);
        assert_eq!(status(word(&mut guest, 604).cast_signed()), Status::Ok);
        let asked = service.grpc_calls();
        assert_eq!(asked.len(), 1);
        assert_eq!(
            asked[0].2,
            GrpcAsk::Send {
                message: b"m".to_vec(),
                end_of_stream: false,
            }
        );
        assert_eq!(asked[0].0.callout, Some(stream), "the delivery was running");
        assert_eq!(guest.open_callout_count(), 1);
    }

    #[test]
    fn a_delivery_that_ends_a_callout_and_traps_returns_the_entry() {
        // Arrange
        let (mut guest, _, root) = recording();
        let unary = open(&mut guest, CalloutKind::GrpcCall, root, root);
        action(&mut guest, 3);
        let mut scope = guest.enter_root();

        // Act
        let answer = scope.on_grpc_receive(root, unary, message());

        // Assert
        assert!(matches!(answer, Err(GuestError::Runtime(_))), "{answer:?}");
        drop(scope);
        assert!(guest.is_poisoned());
        assert_eq!(
            guest.open_callouts(),
            [OpenCallout {
                callout: unary,
                caller: root,
                root,
                kind: CalloutKind::GrpcCall,
            }],
            "the embedder finds the request that waits"
        );
    }

    #[test]
    fn a_delivery_that_keeps_a_callout_and_traps_leaves_one_entry() {
        // Arrange
        let (mut guest, _, root) = recording();
        let stream = open(&mut guest, CalloutKind::GrpcStream, root, root);
        action(&mut guest, 3);
        let mut scope = guest.enter_root();

        // Act
        let answer = scope.on_grpc_receive(root, stream, message());

        // Assert
        assert!(matches!(answer, Err(GuestError::Runtime(_))), "{answer:?}");
        drop(scope);
        assert_eq!(guest.open_callout_count(), 1);
    }

    #[test]
    fn a_grpc_call_through_an_export_reaches_the_service() {
        // Arrange
        let service = Arc::new(RecordingCallouts::new());
        let wat = r#"(module
            (import "env" "proxy_grpc_stream" (func $open (param i32 i32 i32 i32 i32 i32 i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 900) "authz")
            (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 8192)
            (func (export "proxy_abi_version_0_2_1"))
            (func (export "make")
                (i32.store (i32.const 0)
                    (call $open (i32.const 900) (i32.const 5) (i32.const 900) (i32.const 5)
                        (i32.const 900) (i32.const 5) (i32.const 0) (i32.const 0) (i32.const 8)))))"#;
        let mut guest = guest_of(wat, services_with(service.clone()));
        let root = guest.enter_root().on_context_create(None).unwrap();
        guest.enter_root().on_tick(root).unwrap();

        // Act
        let result = guest.call_export::<(), ()>("make", ());

        // Assert
        assert!(result.is_ok(), "{result:?}");
        assert_eq!(status(word(&mut guest, 0).cast_signed()), Status::Ok);
        let asked = service.grpc_calls();
        assert_eq!(asked.len(), 1);
        assert_eq!(asked[0].0.callback, None);
        assert_eq!(guest.open_callout_count(), 1);
    }

    #[test]
    fn a_service_of_two_guests_tells_their_callouts_apart_by_the_guest() {
        // Arrange
        let service = Arc::new(RecordingCallouts::new());
        let mut first = guest_of(OPENER, services_with(service.clone()));
        let mut second = guest_of(OPENER, services_with(service.clone()));
        let one = first.enter_root().on_context_create(None).unwrap();
        let two = second.enter_root().on_context_create(None).unwrap();
        first.enter_root().on_tick(one).unwrap();
        second.enter_root().on_tick(two).unwrap();

        // Act
        let asked = service.grpc_calls();

        // Assert
        assert_eq!(one, two, "both context tables start at one");
        assert_eq!(asked.len(), 2);
        assert_eq!(asked[0].1, asked[1].1, "both callout tables start at one");
        assert_eq!(asked[0].0.context, asked[1].0.context);
        assert_eq!(
            (asked[0].0.guest, asked[1].0.guest),
            (first.id(), second.id())
        );
        assert_ne!(
            asked[0].0.guest, asked[1].0.guest,
            "the guest tells them apart"
        );
    }

    #[test]
    fn the_identity_of_a_guest_is_the_same_in_every_callback() {
        // Arrange
        let service = Arc::new(RecordingCallouts::new());
        let mut guest = guest_of(OPENER, services_with(service.clone()));
        let root = guest.enter_root().on_context_create(None).unwrap();
        guest.enter_root().on_tick(root).unwrap();

        // Act
        guest.enter_root().on_done(root).unwrap();

        // Assert
        let asked = service.grpc_calls();
        assert_eq!(asked.len(), 2);
        assert_eq!(asked[0].0.guest, asked[1].0.guest);
        assert_eq!(asked[0].0.guest, guest.id());
        assert_ne!(asked[0].0.callback, asked[1].0.callback);
    }

    #[test]
    fn open_callout_reports_the_kind_of_a_grpc_callout() {
        // Arrange
        let (mut guest, _, root) = recording();
        let unary = open(&mut guest, CalloutKind::GrpcCall, root, root);
        let stream = open(&mut guest, CalloutKind::GrpcStream, root, root);

        // Act
        let answers = [guest.open_callout(unary), guest.open_callout(stream)];

        // Assert
        assert_eq!(
            answers[0].map(|entry| entry.kind),
            Some(CalloutKind::GrpcCall)
        );
        assert_eq!(
            answers[1].map(|entry| entry.kind),
            Some(CalloutKind::GrpcStream)
        );
        assert_eq!(answers[0].map(|entry| entry.caller), Some(root));
        assert_eq!(guest.open_callout_count(), 2);
    }

    #[test]
    fn a_guest_with_no_service_refuses_a_grpc_callout() {
        // Arrange
        let mut guest = guest_of(
            RECORDER,
            VmServices::new(Arc::new(RecordingSink::default())),
        );
        let root = guest.enter_root().on_context_create(None).unwrap();
        let callout = open(&mut guest, CalloutKind::GrpcCall, root, root);
        let mut scope = guest.enter_root();

        // Act
        let answer = scope.on_grpc_close(root, callout, finished());

        // Assert
        assert!(answer.is_ok(), "a delivery does not ask the service");
        drop(scope);
        assert!(guest.open_callouts().is_empty());
    }

    #[test]
    fn a_delivery_for_a_stream_context_names_the_root_as_the_plugin_context() {
        // Arrange
        let (mut guest, _, root) = recording();
        let stream = guest.enter_root().on_context_create(Some(root)).unwrap();
        let callout = open(&mut guest, CalloutKind::GrpcStream, stream, root);
        let mut scope = guest.enter_root();

        // Act
        let answer = scope.on_grpc_receive(stream, callout, message());

        // Assert
        assert!(answer.is_ok(), "{answer:?}");
        drop(scope);
        assert_eq!(
            word(&mut guest, 200),
            root.wire().cast_unsigned(),
            "a guest SDK looks the plugin context up by this word"
        );
        assert_ne!(word(&mut guest, 200), stream.wire().cast_unsigned());
    }
}
