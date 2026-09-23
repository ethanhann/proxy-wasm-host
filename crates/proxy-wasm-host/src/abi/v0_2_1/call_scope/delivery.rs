//! The callbacks that give a result or an event to the guest.

use crate::abi::v0_2_1::AbiAccess;
use crate::abi::v0_2_1::call_scope::{CallScope, prologue};
use crate::abi::v0_2_1::payload::Delivery;
use wasmtime::{TypedFunc, WasmParams};

use crate::abi::v0_2_1::{
    Callback, CalloutId, CalloutKind, CalloutProblem, ContextId, GrpcStatus, Guest, GuestError,
    HttpCallResponse, QueueId, QueueProblem, StreamState,
};

impl<H: StreamState> CallScope<'_, H> {
    /// Gives the result of an HTTP call to the guest, through
    /// `proxy_on_http_call_response`.
    ///
    /// `context` is the context that made the call, which your
    /// [`Callouts`](crate::abi::v0_2_1::Callouts) service received in the
    /// `Invocation`.
    /// The guest makes that context effective and may continue or answer its
    /// stream, so enter this scope with the stream state of that request, or
    /// with [`Guest::enter_root`] for a call that a root context made.
    /// The crate checks `context` against its record of the callout.
    /// It cannot check that the stream state you lent belongs to that
    /// context.
    ///
    /// The delivery ends the callout, whether the guest exports the callback
    /// or not.
    /// While the callback runs, the guest reads the headers, the trailers, and
    /// the body from `response`, and your stream state is not asked for them.
    /// `proxy_get_status` answers code zero and an empty message, because the
    /// ABI gives the status of an HTTP call no meaning.
    /// The crate keeps `response` for the time of the callback.
    /// Bytes that `response` owns are moved, and borrowed bytes are copied.
    ///
    /// # Errors
    ///
    /// Returns [`GuestError::Callout`] when the callout is not open, when
    /// `context` did not make it, when it is a gRPC callout, or when
    /// `response` arrived and has no header, [`GuestError::Context`] when
    /// the root of the caller is unknown, and [`GuestError::GuestRejected`]
    /// when that root was refused.
    /// The guest does not run in those cases, and the callout stays as it
    /// was.
    /// Returns the
    /// [common runtime failures](CallScope#the-common-runtime-failures).
    pub fn on_http_call_response(
        &mut self,
        context: ContextId,
        callout: CalloutId,
        response: HttpCallResponse<'_>,
    ) -> Result<(), GuestError> {
        self.guest.require_live()?;
        let abi = self.guest.instance().state().abi();
        let refused = |problem| GuestError::Callout {
            id: callout,
            problem,
        };
        let entry = abi
            .callouts()
            .get(callout)
            .ok_or_else(|| refused(CalloutProblem::NotOpen))?;
        if entry.caller != context {
            return Err(refused(CalloutProblem::NotMadeBy(context)));
        }
        if entry.kind != CalloutKind::HttpCall {
            return Err(refused(CalloutProblem::WrongKind(entry.kind)));
        }
        if !response.is_failed() && response.headers().is_empty() {
            return Err(refused(CalloutProblem::NoResponseHeader));
        }
        prologue::require_root(self.guest, entry.root)?;
        prologue::accepted(self.guest, entry.root)?;
        deliver_http_response(self.guest, entry.root, callout, response)
    }

    /// Calls `proxy_on_tick` on a root context.
    ///
    /// The crate runs no timer.
    /// [`Guest::tick_period`] and [`Guest::take_changes`] tell you the period
    /// a root asked for, and you call this from a timer of your own.
    ///
    /// # Errors
    ///
    /// Returns [`GuestError::Context`] when `root` is unknown or a stream
    /// context, [`GuestError::GuestRejected`], and the
    /// [common runtime failures](CallScope#the-common-runtime-failures).
    pub fn on_tick(&mut self, root: ContextId) -> Result<(), GuestError> {
        self.guest.require_live()?;
        prologue::require_root(self.guest, root)?;
        prologue::accepted(self.guest, root)?;
        let func = self.guest.callbacks().tick.clone();
        prologue::run(self.guest, root, Callback::Tick, func, root.wire(), ())?;
        Ok(())
    }

    /// Calls `proxy_on_queue_ready` on a root context that registered
    /// `queue`.
    ///
    /// The ABI does not say which root hears of an item.
    /// [`Guest::queue_registrants`] gives you the roots of this guest that
    /// registered the queue, and you call this one time for each item and
    /// each root you choose.
    /// A root that finds the queue empty reads that as no item.
    ///
    /// # Errors
    ///
    /// Returns [`GuestError::Context`] when `root` is unknown or a stream
    /// context, [`GuestError::GuestRejected`], [`GuestError::Queue`] when no
    /// context of `root` registered `queue`, and the
    /// [common runtime failures](CallScope#the-common-runtime-failures).
    pub fn on_queue_ready(&mut self, root: ContextId, queue: QueueId) -> Result<(), GuestError> {
        self.guest.require_live()?;
        prologue::require_root(self.guest, root)?;
        prologue::accepted(self.guest, root)?;
        let registrants = self.guest.instance().state().abi().registrants(queue);
        if !registrants.contains(&root) {
            return Err(GuestError::Queue {
                id: queue,
                problem: QueueProblem::NotRegisteredBy(root),
            });
        }
        let func = self.guest.callbacks().queue_ready.clone();
        let params = (root.wire(), queue.get().cast_signed());
        prologue::run(self.guest, root, Callback::QueueReady, func, params, ())?;
        Ok(())
    }
}

/// One delivery, as [`deliver`] runs it.
///
/// The fields travel in one value, because the argument list would otherwise
/// pass the limit that clippy sets.
pub(super) struct Delivered<P: WasmParams> {
    /// The context the guest gets as its first argument.
    ///
    /// A callout delivery passes the root of the caller, which a guest SDK
    /// requires.
    /// A foreign function call passes the context the embedder chose, which
    /// the ABI calls the plugin context and a guest SDK ignores.
    pub(super) context: ContextId,
    /// The callout the delivery answers, which a foreign function call has
    /// none of.
    pub(super) callout: Option<CalloutId>,
    /// What the guest reads while the callback runs.
    pub(super) delivery: Delivery,
    /// The callback to run.
    pub(super) callback: Callback,
    /// The exported function, or `None` when the guest exports none.
    pub(super) func: Option<TypedFunc<P, ()>>,
    /// The arguments of the callback.
    pub(super) params: P,
    /// Whether this delivery ends the callout.
    pub(super) ends: bool,
}

/// Runs one callback of a callout on `root`.
///
/// An entry that the delivery ends goes before the guest runs, because the
/// delivery is the one answer that callout gets.
/// It comes back when the delivery poisoned the guest, so that the embedder
/// can find the request that waits for it.
/// A delivery that does not end the callout leaves the entry in the table,
/// so the guest can send on the stream, cancel it, or close it from inside
/// the callback.
/// The delivered value is cleared when the guest returns or fails.
/// A panic of the stream state leaves it on a guest that the scope poisons,
/// where nothing reads it.
pub(super) fn deliver<P: WasmParams>(
    guest: &mut Guest,
    call: Delivered<P>,
) -> Result<(), GuestError> {
    let abi = guest.instance_mut().state_mut().abi_mut();
    let entry = match call.callout.filter(|_| call.ends) {
        Some(callout) => abi
            .callouts_mut()
            .remove(callout)
            .map(|entry| (callout, entry)),
        None => None,
    };
    abi.set_delivery(Some(call.delivery));
    let result = prologue::run(
        guest,
        call.context,
        call.callback,
        call.func,
        call.params,
        (),
    );
    let poisoned = guest.is_poisoned();
    let abi = guest.instance_mut().state_mut().abi_mut();
    abi.set_delivery(None);
    if let Some((callout, entry)) = entry.filter(|_| poisoned) {
        abi.callouts_mut().enter(callout, entry);
    }
    Ok(result?)
}

/// Ends `callout` and runs `proxy_on_http_call_response` on `root`.
///
/// A failed response holds nothing, so its three counts are zero, which is
/// how the ABI tells a guest that the call failed.
pub(super) fn deliver_http_response(
    guest: &mut Guest,
    root: ContextId,
    callout: CalloutId,
    response: HttpCallResponse<'_>,
) -> Result<(), GuestError> {
    let counts = (
        prologue::wire_size(response.headers().len())?,
        prologue::wire_size(response.body().len())?,
        prologue::wire_size(response.trailers().len())?,
    );
    let func = guest.callbacks().http_call_response.clone();
    deliver(
        guest,
        Delivered {
            context: root,
            callout: Some(callout),
            delivery: Delivery::http_call_response(callout, response),
            callback: Callback::HttpCallResponse,
            func,
            params: (
                root.wire(),
                callout.get().cast_signed(),
                counts.0,
                counts.1,
                counts.2,
            ),
            ends: true,
        },
    )
}

/// Ends `callout` and runs `proxy_on_grpc_close` on `root` with `status`.
///
/// The message is measured before anything changes, so a message the ABI
/// cannot hold leaves the callout open.
/// The code passes as its raw bits, so a code above `i32::MAX` reaches the
/// guest whole.
pub(super) fn deliver_grpc_close(
    guest: &mut Guest,
    root: ContextId,
    callout: CalloutId,
    status: GrpcStatus,
) -> Result<(), GuestError> {
    prologue::wire_size(status.message.len())?;
    let code = status.code.cast_signed();
    let func = guest.callbacks().grpc_close.clone();
    deliver(
        guest,
        Delivered {
            context: root,
            callout: Some(callout),
            delivery: Delivery::grpc_close(callout, status),
            callback: Callback::GrpcClose,
            func,
            params: (root.wire(), callout.get().cast_signed(), code),
            ends: true,
        },
    )
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;
    use std::sync::Arc;
    use std::time::Duration;

    use super::*;
    use crate::Error;
    use crate::abi::v0_2_1::callout::Callout;
    use crate::abi::v0_2_1::test_support::callouts::{GrpcAsk, RecordingCallouts, services_with};
    use crate::abi::v0_2_1::test_support::{RecordingSink, RecordingStream, engine, wat_bytes};
    use crate::abi::v0_2_1::types::Status;
    use crate::abi::v0_2_1::{
        Changes, ContextProblem, HeaderPairs, Host, InMemoryStore, OpenCallout, SharedServices,
        VmServices,
    };
    use crate::codec::pairs::encode_pairs;
    use crate::runtime::{GuestPtr, Limits, Module};

    /// A guest that records every callback it gets.
    ///
    /// A response callback writes a record of fifteen words at
    /// `1000 + 64 * n`.
    /// Words 0 to 4 are the five parameters.
    /// Words 5 and 6 are the status and the size of `proxy_get_header_map_size`
    /// on map 6.
    /// Words 7 to 9 are the status, the size, and the unused word of
    /// `proxy_get_buffer_status` on buffer 4.
    /// Words 10 to 13 are the status, the code, the address, and the size of
    /// `proxy_get_status`.
    /// Word 14 is the status of making the context at address 8 effective.
    ///
    /// Address 0 counts the response callbacks, and address 4 holds that count
    /// as `proxy_on_delete` saw it.
    /// `proxy_on_tick` writes its context at 12, and `proxy_on_queue_ready`
    /// writes its context at 16 and the queue at 20.
    /// `probe` writes the statuses of a read of map 6, of buffer 4, and of
    /// `proxy_get_status` at 60, 64, and 68.
    /// `register` and `resolve` write the queue identifier at 904.
    const RECORDER: &str = r#"(module
        (import "env" "proxy_get_header_map_size" (func $map_size (param i32 i32) (result i32)))
        (import "env" "proxy_get_buffer_status" (func $buffer (param i32 i32 i32) (result i32)))
        (import "env" "proxy_get_status" (func $status (param i32 i32 i32) (result i32)))
        (import "env" "proxy_set_effective_context" (func $effective (param i32) (result i32)))
        (import "env" "proxy_register_shared_queue" (func $register (param i32 i32 i32) (result i32)))
        (import "env" "proxy_resolve_shared_queue" (func $resolve (param i32 i32 i32 i32 i32) (result i32)))
        (import "env" "proxy_set_tick_period_milliseconds" (func $period (param i32) (result i32)))
        (memory (export "memory") 1)
        (data (i32.const 900) "q")
        (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 8192)
        (func (export "proxy_abi_version_0_2_1"))
        (func $base (result i32)
            (i32.add (i32.const 1000) (i32.mul (i32.const 64) (i32.load (i32.const 0)))))
        (func (export "proxy_on_http_call_response") (param i32 i32 i32 i32 i32)
            (local $b i32)
            (local.set $b (call $base))
            (i32.store (local.get $b) (local.get 0))
            (i32.store (i32.add (local.get $b) (i32.const 4)) (local.get 1))
            (i32.store (i32.add (local.get $b) (i32.const 8)) (local.get 2))
            (i32.store (i32.add (local.get $b) (i32.const 12)) (local.get 3))
            (i32.store (i32.add (local.get $b) (i32.const 16)) (local.get 4))
            (i32.store (i32.add (local.get $b) (i32.const 20))
                (call $map_size (i32.const 6) (i32.add (local.get $b) (i32.const 24))))
            (i32.store (i32.add (local.get $b) (i32.const 28))
                (call $buffer (i32.const 4) (i32.add (local.get $b) (i32.const 32))
                    (i32.add (local.get $b) (i32.const 36))))
            (i32.store (i32.add (local.get $b) (i32.const 40))
                (call $status (i32.add (local.get $b) (i32.const 44))
                    (i32.add (local.get $b) (i32.const 48)) (i32.add (local.get $b) (i32.const 52))))
            (i32.store (i32.add (local.get $b) (i32.const 56))
                (call $effective (i32.load (i32.const 8))))
            (i32.store (i32.const 0) (i32.add (i32.load (i32.const 0)) (i32.const 1))))
        (func (export "proxy_on_delete") (param i32)
            (i32.store (i32.const 4) (i32.load (i32.const 0))))
        (func (export "proxy_on_tick") (param i32)
            (i32.store (i32.const 12) (local.get 0)))
        (func (export "proxy_on_queue_ready") (param i32 i32)
            (i32.store (i32.const 16) (local.get 0))
            (i32.store (i32.const 20) (local.get 1)))
        (func (export "probe") (result i32)
            (i32.store (i32.const 60) (call $map_size (i32.const 6) (i32.const 72)))
            (i32.store (i32.const 64) (call $buffer (i32.const 4) (i32.const 72) (i32.const 76)))
            (i32.store (i32.const 68) (call $status (i32.const 72) (i32.const 76) (i32.const 80)))
            i32.const 0)
        (func (export "register") (result i32)
            (call $register (i32.const 900) (i32.const 1) (i32.const 904)))
        (func (export "resolve") (result i32)
            (call $resolve (i32.const 0) (i32.const 0) (i32.const 900) (i32.const 1) (i32.const 904)))
        (func (export "set_period") (param i32) (result i32)
            (call $period (local.get 0)))
        (func (export "crash") unreachable))"#;

    fn guest_with(services: VmServices) -> Guest {
        let engine = engine();
        let host = Host::new(&engine).unwrap();
        let module = Module::new(&engine, &wat_bytes(RECORDER)).unwrap();
        Guest::new(&host, &module, services, &Limits::default()).unwrap()
    }

    fn guest() -> Guest {
        guest_with(VmServices::new(Arc::new(RecordingSink::default())))
    }

    /// A guest with a root and a stream context.
    fn with_stream() -> (Guest, ContextId, ContextId) {
        let mut guest = guest();
        let root = guest.enter_root().on_context_create(None).unwrap();
        let stream = guest.enter_root().on_context_create(Some(root)).unwrap();
        (guest, root, stream)
    }

    fn open(guest: &mut Guest, caller: ContextId, root: ContextId) -> CalloutId {
        let table = guest.instance_mut().state_mut().abi_mut().callouts_mut();
        let id = table.reserve();
        let callout = Callout::new(CalloutKind::HttpCall, caller, root);
        table.enter(id, callout);
        id
    }

    fn http(callout: CalloutId, caller: ContextId, root: ContextId) -> OpenCallout {
        Callout::new(CalloutKind::HttpCall, caller, root).report(callout)
    }

    fn word(guest: &mut Guest, at: u32) -> u32 {
        guest
            .instance_mut()
            .memory()
            .unwrap()
            .read_u32(GuestPtr::from_address(at))
            .unwrap()
    }

    fn put(guest: &mut Guest, at: u32, value: u32) {
        guest
            .instance_mut()
            .memory()
            .unwrap()
            .write_u32(GuestPtr::from_address(at), value)
            .unwrap();
    }

    /// The fifteen words the guest wrote for response callback `n`.
    fn record(guest: &mut Guest, n: u32) -> Vec<u32> {
        (0..15)
            .map(|field| word(guest, 1000 + 64 * n + 4 * field))
            .collect()
    }

    fn pairs(list: &[(&'static [u8], &'static [u8])]) -> HeaderPairs<'static> {
        list.iter()
            .map(|(key, value)| (Cow::Borrowed(*key), Cow::Borrowed(*value)))
            .collect()
    }

    fn received() -> HttpCallResponse<'static> {
        HttpCallResponse::received(pairs(&[(b":status", b"200"), (b"x", b"y")]))
            .with_body(Cow::Borrowed(b"hello"))
            .with_trailers(pairs(&[(b"t", b"v")]))
    }

    fn status(value: u32) -> Status {
        Status::try_from(value.cast_signed()).unwrap()
    }

    #[test]
    fn a_delivery_gives_the_guest_the_root_the_counts_and_the_response() {
        // Arrange
        let (mut guest, root, stream) = with_stream();
        let callout = open(&mut guest, stream, root);
        put(&mut guest, 8, stream.get());

        // Act
        let result =
            guest
                .enter(RecordingStream::new())
                .on_http_call_response(stream, callout, received());

        // Assert
        assert!(result.is_ok(), "{result:?}");
        let seen = record(&mut guest, 0);
        assert_eq!(seen[..5], [root.get(), callout.get(), 2, 5, 1]);
        assert_eq!((status(seen[5]), seen[6]), (Status::Ok, 36), "map 6");
        assert_eq!((status(seen[7]), seen[8]), (Status::Ok, 5), "buffer 4");
        assert_eq!((status(seen[10]), seen[11]), (Status::Ok, 0), "the status");
        assert_eq!(status(seen[14]), Status::Ok, "the caller became effective");
        assert!(guest.open_callouts().is_empty());
    }

    #[test]
    fn a_failed_response_gives_three_zeros_and_no_status() {
        // Arrange
        let (mut guest, root, stream) = with_stream();
        let callout = open(&mut guest, stream, root);
        let mut scope = guest.enter_root();

        // Act
        let result = scope.on_http_call_response(stream, callout, HttpCallResponse::failed());

        // Assert
        assert!(result.is_ok(), "{result:?}");
        drop(scope);
        let seen = record(&mut guest, 0);
        assert_eq!(seen[2..5], [0, 0, 0]);
        assert_eq!((status(seen[10]), seen[11]), (Status::Ok, 0));
    }

    #[test]
    fn a_received_response_with_no_header_is_refused_and_the_callout_stays() {
        // Arrange
        let (mut guest, root, stream) = with_stream();
        let callout = open(&mut guest, stream, root);
        let no_header = HttpCallResponse::received(Vec::new()).with_body(Cow::Borrowed(b"x"));
        let mut scope = guest.enter_root();

        // Act
        let result = scope.on_http_call_response(stream, callout, no_header);

        // Assert
        assert!(
            matches!(
                result,
                Err(GuestError::Callout { id, problem: CalloutProblem::NoResponseHeader })
                    if id == callout
            ),
            "{result:?}"
        );
        drop(scope);
        assert_eq!(word(&mut guest, 0), 0, "no response callback ran");
        assert_eq!(guest.open_callout_count(), 1);
    }

    #[test]
    fn the_response_is_not_readable_before_a_delivery() {
        // Arrange
        let (mut guest, _, _) = with_stream();

        // Act
        let probed = guest.call_export::<(), i32>("probe", ());

        // Assert
        assert!(probed.is_ok());
        assert_eq!(
            [60, 64, 68].map(|at| status(word(&mut guest, at))),
            [Status::BadArgument, Status::NotFound, Status::NotFound]
        );
    }

    #[test]
    fn the_response_is_not_readable_after_a_delivery() {
        // Arrange
        let (mut guest, root, stream) = with_stream();
        let callout = open(&mut guest, stream, root);
        guest
            .enter_root()
            .on_http_call_response(stream, callout, received())
            .unwrap();

        // Act
        let probed = guest.call_export::<(), i32>("probe", ());

        // Assert
        assert!(probed.is_ok());
        assert_eq!(
            [60, 64, 68].map(|at| status(word(&mut guest, at))),
            [Status::BadArgument, Status::NotFound, Status::NotFound]
        );
    }

    #[test]
    fn the_stream_state_is_never_asked_for_the_response() {
        // Arrange
        let (mut guest, root, stream) = with_stream();
        let callout = open(&mut guest, stream, root);
        let mut scope = guest.enter(RecordingStream::new());

        // Act
        let result = scope.on_http_call_response(stream, callout, received());

        // Assert
        assert!(result.is_ok(), "{result:?}");
        let recording = scope.finish();
        assert!(recording.calls().is_empty());
        assert!(recording.buffer_calls().is_empty());
    }

    #[test]
    fn a_delivery_the_table_does_not_allow_is_refused_and_the_guest_does_not_run() {
        // Arrange
        let (mut guest, root, stream) = with_stream();
        let callout = open(&mut guest, stream, root);
        let unknown = CalloutId::try_from(99_u32).unwrap();
        let mut scope = guest.enter_root();

        // Act
        let refusals = [
            scope.on_http_call_response(stream, unknown, received()),
            scope.on_http_call_response(root, callout, received()),
        ];

        // Assert
        assert!(matches!(
            refusals[0],
            Err(GuestError::Callout { id, problem: CalloutProblem::NotOpen }) if id == unknown
        ));
        assert!(matches!(
            refusals[1],
            Err(GuestError::Callout { id, problem: CalloutProblem::NotMadeBy(named) })
                if id == callout && named == root
        ));
        drop(scope);
        assert_eq!(word(&mut guest, 0), 0, "the guest got no callback");
        assert_eq!(guest.open_callouts(), [http(callout, stream, root)]);
        assert_eq!(
            refusals[1].as_ref().unwrap_err().to_string(),
            format!("callout {callout} was not made by context {root}")
        );
    }

    #[test]
    fn a_delivery_for_a_callout_whose_root_is_gone_is_refused_and_the_guest_does_not_run() {
        // Arrange
        let mut guest = guest();
        let gone = ContextId::try_from(9).unwrap();
        let callout = open(&mut guest, gone, gone);
        let mut scope = guest.enter_root();

        // Act
        let result = scope.on_http_call_response(gone, callout, received());

        // Assert
        assert!(matches!(
            result,
            Err(GuestError::Context {
                id,
                problem: ContextProblem::Unknown
            }) if id == gone
        ));
        drop(scope);
        assert_eq!(word(&mut guest, 0), 0, "no response callback ran");
    }

    #[test]
    fn a_delivery_on_a_refused_root_is_refused_and_the_callout_stays() {
        // Arrange
        let (mut guest, root, stream) = with_stream();
        let callout = open(&mut guest, stream, root);
        let abi = guest.instance_mut().state_mut().abi_mut();
        abi.contexts_mut().reject(root);
        let mut scope = guest.enter_root();

        // Act
        let result = scope.on_http_call_response(stream, callout, received());

        // Assert
        assert!(
            matches!(result, Err(GuestError::GuestRejected { root: refused, .. }) if refused == root),
            "{result:?}"
        );
        drop(scope);
        assert_eq!(word(&mut guest, 0), 0, "no response callback ran");
        assert_eq!(guest.open_callouts().len(), 1);
    }

    #[test]
    fn one_open_callout_is_reported_with_its_caller_and_its_root() {
        // Arrange
        let (mut guest, root, stream) = with_stream();
        let callout = open(&mut guest, stream, root);
        let absent = CalloutId::try_from(77_u32).unwrap();

        // Act
        let answers = [guest.open_callout(callout), guest.open_callout(absent)];

        // Assert
        assert_eq!(answers, [Some(http(callout, stream, root)), None]);
        assert_eq!(guest.open_callout_count(), 1);
    }

    #[test]
    fn a_second_delivery_for_one_callout_is_refused() {
        // Arrange
        let (mut guest, root, stream) = with_stream();
        let callout = open(&mut guest, stream, root);
        guest
            .enter_root()
            .on_http_call_response(stream, callout, received())
            .unwrap();

        // Act
        let second = guest
            .enter_root()
            .on_http_call_response(stream, callout, received());

        // Assert
        assert!(matches!(
            second,
            Err(GuestError::Callout {
                problem: CalloutProblem::NotOpen,
                ..
            })
        ));
        assert_eq!(word(&mut guest, 0), 1);
    }

    #[test]
    fn a_trap_in_the_callback_clears_the_response_and_poisons_the_guest() {
        // Arrange
        let engine = engine();
        let host = Host::new(&engine).unwrap();
        let wat = r#"(module
            (memory (export "memory") 1)
            (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 8192)
            (func (export "proxy_abi_version_0_2_1"))
            (func (export "proxy_on_http_call_response") (param i32 i32 i32 i32 i32) unreachable))"#;
        let module = Module::new(&engine, &wat_bytes(wat)).unwrap();
        let services = VmServices::new(Arc::new(RecordingSink::default()));
        let mut guest = Guest::new(&host, &module, services, &Limits::default()).unwrap();
        let root = guest.enter_root().on_context_create(None).unwrap();
        let callout = open(&mut guest, root, root);

        // Act
        let result = guest
            .enter_root()
            .on_http_call_response(root, callout, received());

        // Assert
        assert!(matches!(
            result,
            Err(GuestError::Runtime(Error::Trap { .. }))
        ));
        assert!(guest.is_poisoned());
        assert!(guest.instance().state().abi().delivery().is_none());
        assert_eq!(guest.open_callouts(), [http(callout, root, root)]);
        assert_eq!(guest.open_callout_count(), 1);
    }

    #[test]
    fn a_deleted_stream_gets_its_failures_after_the_delete_callback() {
        // Arrange
        let (mut guest, root, stream) = with_stream();
        let first = open(&mut guest, stream, root);
        let second = open(&mut guest, stream, root);
        let mut scope = guest.enter(RecordingStream::new());
        scope.on_done(stream).unwrap();

        // Act
        let result = scope.on_delete(stream);

        // Assert
        assert_eq!(result.ok(), Some(vec![first, second]));
        drop(scope);
        assert_eq!(word(&mut guest, 4), 0, "the delete callback ran first");
        assert_eq!(word(&mut guest, 0), 2);
        assert_eq!(
            record(&mut guest, 0)[..5],
            [root.get(), first.get(), 0, 0, 0]
        );
        assert_eq!(
            record(&mut guest, 1)[..5],
            [root.get(), second.get(), 0, 0, 0]
        );
        assert!(guest.open_callouts().is_empty());
        let late = guest
            .enter_root()
            .on_http_call_response(stream, first, received());
        assert!(matches!(
            late,
            Err(GuestError::Callout {
                problem: CalloutProblem::NotOpen,
                ..
            })
        ));
    }

    #[test]
    fn a_deleted_root_gets_its_failures_before_the_delete_callback() {
        // Arrange
        let mut guest = guest();
        let root = guest.enter_root().on_context_create(None).unwrap();
        let first = open(&mut guest, root, root);
        let second = open(&mut guest, root, root);
        let mut scope = guest.enter_root();
        scope.on_done(root).unwrap();

        // Act
        let result = scope.on_delete(root);

        // Assert
        assert_eq!(result.ok(), Some(vec![first, second]));
        drop(scope);
        assert_eq!(
            word(&mut guest, 4),
            2,
            "both failures came before the delete"
        );
        assert!(guest.open_callouts().is_empty());
    }

    #[test]
    fn a_context_with_no_callout_gets_no_response_callback_at_its_deletion() {
        // Arrange
        let (mut guest, root, stream) = with_stream();
        let other = guest.enter_root().on_context_create(Some(root)).unwrap();
        let kept = open(&mut guest, other, root);
        let mut scope = guest.enter_root();
        scope.on_done(stream).unwrap();

        // Act
        let result = scope.on_delete(stream);

        // Assert
        assert_eq!(result.ok(), Some(Vec::new()));
        drop(scope);
        assert_eq!(word(&mut guest, 0), 0);
        assert_eq!(guest.open_callouts(), [http(kept, other, root)]);
    }

    #[test]
    fn a_tick_reaches_a_root_and_is_refused_for_every_other_context() {
        // Arrange
        let (mut guest, root, stream) = with_stream();
        let unknown = ContextId::try_from(99).unwrap();
        let mut scope = guest.enter_root();

        // Act
        let answers = [
            scope.on_tick(root),
            scope.on_tick(stream),
            scope.on_tick(unknown),
        ];

        // Assert
        assert!(answers[0].is_ok(), "{:?}", answers[0]);
        assert!(matches!(answers[1], Err(GuestError::Context { id, .. }) if id == stream));
        assert!(matches!(answers[2], Err(GuestError::Context { id, .. }) if id == unknown));
        drop(scope);
        assert_eq!(word(&mut guest, 12), root.get());
    }

    #[test]
    fn a_queue_callback_reaches_only_a_root_that_registered_the_queue() {
        // Arrange
        let mut guest = guest();
        let registrant = guest.enter_root().on_context_create(None).unwrap();
        guest.call_export::<(), i32>("register", ()).unwrap();
        let queue = QueueId::try_from(word(&mut guest, 904)).unwrap();
        let resolver = guest.enter_root().on_context_create(None).unwrap();
        guest.call_export::<(), i32>("resolve", ()).unwrap();
        let stream = guest
            .enter_root()
            .on_context_create(Some(registrant))
            .unwrap();
        let unknown = ContextId::try_from(99).unwrap();
        let mut scope = guest.enter_root();

        // Act
        let answers = [
            scope.on_queue_ready(registrant, queue),
            scope.on_queue_ready(resolver, queue),
            scope.on_queue_ready(stream, queue),
            scope.on_queue_ready(unknown, queue),
        ];

        // Assert
        assert!(answers[0].is_ok(), "{:?}", answers[0]);
        assert!(matches!(
            answers[1],
            Err(GuestError::Queue { id, problem: QueueProblem::NotRegisteredBy(root) })
                if id == queue && root == resolver
        ));
        assert!(matches!(answers[2], Err(GuestError::Context { id, .. }) if id == stream));
        assert!(matches!(answers[3], Err(GuestError::Context { id, .. }) if id == unknown));
        drop(scope);
        assert_eq!(
            (word(&mut guest, 16), word(&mut guest, 20)),
            (registrant.get(), queue.get())
        );
        assert_eq!(guest.queue_registrants(queue), [registrant]);
    }

    #[test]
    fn a_refused_root_gets_no_tick_and_no_queue_callback() {
        // Arrange
        let mut guest = guest();
        let root = guest.enter_root().on_context_create(None).unwrap();
        guest.call_export::<(), i32>("register", ()).unwrap();
        let queue = QueueId::try_from(word(&mut guest, 904)).unwrap();
        let abi = guest.instance_mut().state_mut().abi_mut();
        abi.contexts_mut().reject(root);
        let mut scope = guest.enter_root();

        // Act
        let answers = [scope.on_tick(root), scope.on_queue_ready(root, queue)];

        // Assert
        for answer in answers {
            assert!(
                matches!(answer, Err(GuestError::GuestRejected { root: refused, .. }) if refused == root),
                "{answer:?}"
            );
        }
        drop(scope);
        assert_eq!((word(&mut guest, 12), word(&mut guest, 16)), (0, 0));
    }

    #[test]
    fn two_roots_register_one_queue_and_a_deleted_root_leaves_the_set() {
        // Arrange
        let mut guest = guest();
        let first = guest.enter_root().on_context_create(None).unwrap();
        guest.call_export::<(), i32>("register", ()).unwrap();
        let second = guest.enter_root().on_context_create(None).unwrap();
        guest.call_export::<(), i32>("register", ()).unwrap();
        let queue = QueueId::try_from(word(&mut guest, 904)).unwrap();
        let both = guest.queue_registrants(queue);
        let mut scope = guest.enter_root();
        scope.on_done(first).unwrap();

        // Act
        let deleted = scope.on_delete(first);

        // Assert
        assert!(deleted.is_ok(), "{deleted:?}");
        drop(scope);
        assert_eq!(both, [first, second]);
        assert_eq!(guest.queue_registrants(queue), [second]);
    }

    #[test]
    fn a_replaced_store_empties_the_registrants() {
        // Arrange
        let mut guest = guest();
        let root = guest.enter_root().on_context_create(None).unwrap();
        guest.call_export::<(), i32>("register", ()).unwrap();
        let queue = QueueId::try_from(word(&mut guest, 904)).unwrap();
        let replacement: Arc<dyn SharedServices> = Arc::new(InMemoryStore::new());
        let services = guest.services().clone().with_shared(replacement);
        *guest.services_mut() = services;

        // Act
        let answer = guest.enter_root().on_queue_ready(root, queue);

        // Assert
        assert!(matches!(
            answer,
            Err(GuestError::Queue {
                problem: QueueProblem::NotRegisteredBy(_),
                ..
            })
        ));
        assert!(guest.queue_registrants(queue).is_empty());
    }

    #[test]
    fn a_replaced_store_empties_the_registrants_and_the_queue_changes_at_once() {
        // Arrange
        let mut guest = guest();
        guest.enter_root().on_context_create(None).unwrap();
        guest.call_export::<(), i32>("register", ()).unwrap();
        let queue = QueueId::try_from(word(&mut guest, 904)).unwrap();
        let replacement: Arc<dyn SharedServices> = Arc::new(InMemoryStore::new());
        let services = guest.services().clone().with_shared(replacement);

        // Act
        *guest.services_mut() = services;

        // Assert
        assert!(guest.queue_registrants(queue).is_empty());
        assert!(guest.take_changes().queues.is_empty());
    }

    fn periods(changes: &Changes) -> Vec<(ContextId, Option<Duration>)> {
        changes.tick_periods.clone().into_iter().collect()
    }

    fn registrations(changes: &Changes) -> Vec<(QueueId, ContextId, Vec<u8>)> {
        let each = changes.queues.iter();
        each.map(|entry| (entry.queue, entry.root, entry.name.clone()))
            .collect()
    }

    #[test]
    fn a_stream_context_that_registers_and_sets_a_period_is_recorded_as_its_root() {
        // Arrange
        let (mut guest, root, stream) = with_stream();
        let abi = guest.instance_mut().state_mut().abi_mut();
        abi.contexts_mut().set_effective(stream);
        guest.call_export::<(), i32>("register", ()).unwrap();
        let queue = QueueId::try_from(word(&mut guest, 904)).unwrap();

        // Act
        let result = guest.call_export::<i32, i32>("set_period", 100);

        // Assert
        assert!(result.is_ok(), "{result:?}");
        assert_eq!(guest.queue_registrants(queue), [root]);
        let changes = guest.take_changes();
        assert_eq!(registrations(&changes), [(queue, root, b"q".to_vec())]);
        assert_eq!(
            periods(&changes),
            [(root, Some(Duration::from_millis(100)))]
        );
    }

    #[test]
    fn a_period_a_guest_sets_many_times_is_one_change() {
        // Arrange
        let mut guest = guest();
        let root = guest.enter_root().on_context_create(None).unwrap();
        for period in [100, 100, 250] {
            guest.call_export::<i32, i32>("set_period", period).unwrap();
        }

        // Act
        let changes = guest.take_changes();

        // Assert
        assert_eq!(changes.tick_periods.len(), 1);
        assert_eq!(
            periods(&changes),
            [(root, Some(Duration::from_millis(250)))]
        );
    }

    #[test]
    fn the_changes_report_a_tick_period_and_a_registration_one_time() {
        // Arrange
        let mut guest = guest();
        let root = guest.enter_root().on_context_create(None).unwrap();
        guest.call_export::<i32, i32>("set_period", 250).unwrap();
        guest.call_export::<i32, i32>("set_period", 0).unwrap();
        guest.call_export::<(), i32>("register", ()).unwrap();
        guest.call_export::<(), i32>("register", ()).unwrap();
        let queue = QueueId::try_from(word(&mut guest, 904)).unwrap();

        // Act
        let changes = guest.take_changes();

        // Assert
        assert_eq!(
            periods(&changes),
            [(root, None)],
            "the last period is the one"
        );
        assert_eq!(registrations(&changes), [(queue, root, b"q".to_vec())]);
        assert!(!changes.is_empty());
        assert_eq!(guest.take_changes(), Changes::default());
    }

    /// A guest that exports none of the three callbacks of this file.
    const SILENT: &str = r#"(module
        (memory (export "memory") 1)
        (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 8192)
        (func (export "proxy_abi_version_0_2_1"))
        (func (export "crash") unreachable))"#;

    fn silent() -> (Guest, ContextId) {
        let engine = engine();
        let host = Host::new(&engine).unwrap();
        let module = Module::new(&engine, &wat_bytes(SILENT)).unwrap();
        let services = VmServices::new(Arc::new(RecordingSink::default()));
        let mut guest = Guest::new(&host, &module, services, &Limits::default()).unwrap();
        let root = guest.enter_root().on_context_create(None).unwrap();
        (guest, root)
    }

    #[test]
    fn a_guest_that_exports_none_of_the_three_gets_no_call_and_the_callout_still_ends() {
        // Arrange
        let (mut guest, root) = silent();
        let callout = open(&mut guest, root, root);
        let queue = QueueId::try_from(1_u32).unwrap();
        let state = guest.instance_mut().state_mut();
        state.abi_mut().settle();
        state.abi_mut().register_queue(queue, root, b"q");
        let three = [
            Callback::HttpCallResponse,
            Callback::Tick,
            Callback::QueueReady,
        ];
        let exported = three.map(|callback| guest.exports_callback(callback));
        let mut scope = guest.enter_root();

        // Act
        let answers = [
            scope.on_http_call_response(root, callout, received()),
            scope.on_tick(root),
            scope.on_queue_ready(root, queue),
        ];

        // Assert
        assert_eq!(exported, [false; 3]);
        for answer in answers {
            assert!(answer.is_ok(), "{answer:?}");
        }
        drop(scope);
        assert!(guest.open_callouts().is_empty());
        assert!(!guest.is_poisoned());
    }

    #[test]
    fn the_three_callbacks_are_refused_on_a_poisoned_guest() {
        // Arrange
        let (mut guest, root) = silent();
        let callout = open(&mut guest, root, root);
        let queue = QueueId::try_from(1_u32).unwrap();
        let _ = guest.call_export::<(), ()>("crash", ());
        let mut scope = guest.enter_root();

        // Act
        let answers = [
            scope.on_http_call_response(root, callout, received()),
            scope.on_tick(root),
            scope.on_queue_ready(root, queue),
        ];

        // Assert
        for answer in &answers {
            assert!(
                matches!(answer, Err(GuestError::Runtime(Error::Poisoned))),
                "{answer:?}"
            );
        }
        drop(scope);
        assert_eq!(
            guest.open_callouts(),
            [http(callout, root, root)],
            "a poisoned guest still reports what was open"
        );
    }

    /// The text form of `bytes` for a WAT data segment.
    fn escaped(bytes: &[u8]) -> String {
        use std::fmt::Write as _;
        bytes.iter().fold(String::new(), |mut text, byte| {
            let _ = write!(text, "\\{byte:02x}");
            text
        })
    }

    /// A guest whose root makes an HTTP call from its tick, and also from a
    /// response callback when `again` is set.
    ///
    /// Address 0 holds the status of the last call and address 4 its
    /// identifier.
    /// A response callback writes the identifier at 8 and the header count at
    /// 12.
    fn calling_guest(again: bool) -> (Guest, Arc<RecordingCallouts>, ContextId) {
        let headers = encode_pairs(&[
            (b":authority".as_slice(), b"config".as_slice()),
            (b":method", b"GET"),
            (b":path", b"/v1"),
        ])
        .unwrap();
        let wat = format!(
            r#"(module
            (import "env" "proxy_http_call"
                (func $call (param i32 i32 i32 i32 i32 i32 i32 i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 100) "config")
            (data (i32.const 200) "{headers}")
            (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 8192)
            (func (export "proxy_abi_version_0_2_1"))
            (func $make (export "make")
                (i32.store (i32.const 0)
                    (call $call (i32.const 100) (i32.const 6) (i32.const 200) (i32.const {len})
                        (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
                        (i32.const 5000) (i32.const 4))))
            (func (export "proxy_on_tick") (param i32) (call $make))
            (func (export "proxy_on_http_call_response") (param i32 i32 i32 i32 i32)
                (i32.store (i32.const 8) (local.get 1))
                (i32.store (i32.const 12) (local.get 2))
                (if (i32.const {again}) (then (call $make)))))"#,
            headers = escaped(&headers),
            len = headers.len(),
            again = i32::from(again),
        );
        let engine = engine();
        let host = Host::new(&engine).unwrap();
        let module = Module::new(&engine, &wat_bytes(&wat)).unwrap();
        let service = Arc::new(RecordingCallouts::new());
        let services = services_with(Arc::clone(&service));
        let mut guest = Guest::new(&host, &module, services, &Limits::default()).unwrap();
        let root = guest.enter_root().on_context_create(None).unwrap();
        (guest, service, root)
    }

    #[test]
    fn a_root_that_makes_a_call_from_its_tick_reaches_the_service_and_gets_the_response() {
        // Arrange
        let (mut guest, service, root) = calling_guest(false);
        guest.enter_root().on_tick(root).unwrap();
        let (call, callout, request) = service.http_calls().remove(0);

        // Act
        let delivered = guest
            .enter_root()
            .on_http_call_response(call.context, callout, received());

        // Assert
        assert!(delivered.is_ok(), "{delivered:?}");
        assert_eq!(status(word(&mut guest, 0)), Status::Ok);
        assert_eq!(
            word(&mut guest, 4),
            callout.get(),
            "the guest got the identifier"
        );
        assert_eq!((call.context, call.callback), (root, Some(Callback::Tick)));
        assert_eq!(request.upstream.as_ref(), b"config");
        assert_eq!(request.timeout, Duration::from_secs(5));
        assert_eq!(
            (word(&mut guest, 8), word(&mut guest, 12)),
            (callout.get(), 2)
        );
    }

    #[test]
    fn a_call_through_an_export_reaches_the_service_with_no_callback() {
        // Arrange
        let (mut guest, service, root) = calling_guest(false);

        // Act
        let result = guest.call_export::<(), ()>("make", ());

        // Assert
        assert!(result.is_ok(), "{result:?}");
        assert_eq!(status(word(&mut guest, 0)), Status::Ok);
        let calls = service.http_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!((calls[0].0.context, calls[0].0.callback), (root, None));
        assert_eq!(guest.open_callouts().len(), 1);
    }

    #[test]
    fn a_root_that_is_deleted_opens_no_call_in_its_failure_delivery() {
        // Arrange
        let (mut guest, service, root) = calling_guest(true);
        guest.enter_root().on_tick(root).unwrap();
        let mut scope = guest.enter_root();
        scope.on_done(root).unwrap();

        // Act
        let result = scope.on_delete(root);

        // Assert
        assert_eq!(
            result.ok().map(|ended| ended.len()),
            Some(1),
            "the call of the tick alone"
        );
        drop(scope);
        assert_eq!(
            service.http_calls().len(),
            1,
            "the service was not asked in the failure delivery"
        );
        assert_eq!(
            status(word(&mut guest, 0)),
            Status::InternalFailure,
            "the guest was refused inside the deletion"
        );
        assert_eq!(guest.context_type(root), None);
        assert!(guest.open_callouts().is_empty());
    }

    /// A guest that counts the failure deliveries of a deletion.
    ///
    /// Address 0 counts the HTTP responses and address 4 the gRPC closes.
    /// Address 8 holds the sum of both as `proxy_on_delete` saw it.
    /// Address 16 and address 20 hold the last identifier of each kind.
    /// `set_cancel` sets a callout that the close callback cancels.
    const ENDING: &str = r#"(module
        (import "env" "proxy_grpc_cancel" (func $cancel (param i32) (result i32)))
        (memory (export "memory") 1)
        (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 8192)
        (func (export "proxy_abi_version_0_2_1"))
        (func (export "set_cancel") (param i32) (i32.store (i32.const 24) (local.get 0)))
        (func (export "proxy_on_http_call_response") (param i32 i32 i32 i32 i32)
            (i32.store (i32.const 0) (i32.add (i32.load (i32.const 0)) (i32.const 1)))
            (i32.store (i32.const 16) (local.get 1)))
        (func (export "proxy_on_grpc_close") (param i32 i32 i32)
            (i32.store (i32.const 4) (i32.add (i32.load (i32.const 4)) (i32.const 1)))
            (i32.store (i32.const 20) (local.get 1))
            (if (i32.load (i32.const 24))
                (then (i32.store (i32.const 28) (call $cancel (i32.load (i32.const 24)))))))
        (func (export "proxy_on_delete") (param i32)
            (i32.store (i32.const 8)
                (i32.add (i32.load (i32.const 0)) (i32.load (i32.const 4))))))"#;

    /// A guest of `ENDING` with a service and a root context.
    fn ending() -> (Guest, Arc<RecordingCallouts>, ContextId) {
        let engine = engine();
        let host = Host::new(&engine).unwrap();
        let module = Module::new(&engine, &wat_bytes(ENDING)).unwrap();
        let service = Arc::new(RecordingCallouts::new());
        let mut guest = Guest::new(
            &host,
            &module,
            services_with(service.clone()),
            &Limits::default(),
        )
        .unwrap();
        let root = guest.enter_root().on_context_create(None).unwrap();
        (guest, service, root)
    }

    fn open_kind(
        guest: &mut Guest,
        kind: CalloutKind,
        caller: ContextId,
        root: ContextId,
    ) -> CalloutId {
        let table = guest.instance_mut().state_mut().abi_mut().callouts_mut();
        let id = table.reserve();
        table.enter(id, Callout::new(kind, caller, root));
        id
    }

    #[test]
    fn a_deleted_stream_context_ends_a_callout_of_each_kind_after_the_delete_callback() {
        // Arrange
        let (mut guest, _, root) = ending();
        let stream = guest.enter_root().on_context_create(Some(root)).unwrap();
        let http = open_kind(&mut guest, CalloutKind::HttpCall, stream, root);
        let unary = open_kind(&mut guest, CalloutKind::GrpcCall, stream, root);
        let opened = open_kind(&mut guest, CalloutKind::GrpcStream, stream, root);
        let mut scope = guest.enter_root();
        scope.on_done(stream).unwrap();

        // Act
        let ended = scope.on_delete(stream);

        // Assert
        assert_eq!(ended.ok(), Some(vec![http, unary, opened]));
        drop(scope);
        assert_eq!(word(&mut guest, 0), 1, "one HTTP failure");
        assert_eq!(word(&mut guest, 4), 2, "one close for each gRPC kind");
        assert_eq!(word(&mut guest, 8), 0, "the failures come after the delete");
        assert!(guest.open_callouts().is_empty());
    }

    #[test]
    fn a_deleted_root_gets_its_closes_before_the_delete_callback() {
        // Arrange
        let (mut guest, _, root) = ending();
        let unary = open_kind(&mut guest, CalloutKind::GrpcCall, root, root);
        let mut scope = guest.enter_root();
        scope.on_done(root).unwrap();

        // Act
        let ended = scope.on_delete(root);

        // Assert
        assert_eq!(ended.ok(), Some(vec![unary]));
        drop(scope);
        assert_eq!(word(&mut guest, 4), 1);
        assert_eq!(word(&mut guest, 20), unary.get());
        assert_eq!(word(&mut guest, 8), 1, "the delete saw the close");
    }

    #[test]
    fn a_callout_the_guest_cancels_inside_a_close_gets_no_delivery() {
        // Arrange
        let (mut guest, service, root) = ending();
        let first = open_kind(&mut guest, CalloutKind::GrpcStream, root, root);
        let second = open_kind(&mut guest, CalloutKind::GrpcCall, root, root);
        guest
            .instance_mut()
            .call::<i32, ()>("set_cancel", second.get().cast_signed())
            .unwrap();
        let mut scope = guest.enter_root();
        scope.on_done(root).unwrap();

        // Act
        let ended = scope.on_delete(root);

        // Assert
        assert_eq!(
            ended.ok(),
            Some(vec![first]),
            "the answer names the callouts the deletion ended"
        );
        drop(scope);
        assert_eq!(word(&mut guest, 4), 1, "the second got no callback");
        assert_eq!(status(word(&mut guest, 28)), Status::Ok);
        assert_eq!(
            service
                .grpc_calls()
                .into_iter()
                .map(|(_, id, ask)| (id, ask))
                .collect::<Vec<_>>(),
            vec![(second, GrpcAsk::Cancel)]
        );
        assert!(guest.open_callouts().is_empty());
    }
}
