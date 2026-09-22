//! The root context of the `exercise-all` guest, from its start to its ticks,
//! its queue, and its gRPC callouts.
//!
//! The helpers below are test code, and the allowance clippy makes for a test
//! does not reach a function of an integration test that carries no test
//! attribute.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::borrow::Cow;
use std::time::Duration;

use proxy_wasm_host::abi::v0_2_1::types::{LogLevel, MetricType};
use proxy_wasm_host::abi::v0_2_1::{
    Callback, CalloutId, CalloutKind, ContextId, GrpcStatus, Guest, GuestError, HeaderPairs,
    Started,
};

mod common;

use common::harness::{Exercise, NOW, http_plugin, pair, tcp_plugin};
use common::recorder::Event;

fn callout(id: u32) -> CalloutId {
    CalloutId::try_from(id).unwrap()
}

fn info(line: &str) -> Event {
    Event::Log(LogLevel::Info, line.to_owned())
}

/// What the test delivers to a gRPC stream, from its metadata to its close.
struct Replies {
    initial: HeaderPairs<'static>,
    message: Cow<'static, [u8]>,
    trailing: HeaderPairs<'static>,
    status: GrpcStatus,
}

/// Delivers the whole life of a gRPC stream to the root that opened it.
fn run_stream(
    guest: &mut Guest,
    root: ContextId,
    stream: CalloutId,
    replies: Replies,
) -> Result<(), GuestError> {
    let mut scope = guest.enter_root();
    scope.on_grpc_receive_initial_metadata(root, stream, replies.initial)?;
    scope.on_grpc_receive(root, stream, replies.message)?;
    scope.on_grpc_receive_trailing_metadata(root, stream, replies.trailing)?;
    scope.on_grpc_close(root, stream, replies.status)
}

#[test]
fn the_vm_start_reads_the_vm_the_environment_and_the_clock() {
    // Arrange
    let exercise = Exercise::new();
    let mut guest = exercise.spec.build().unwrap();
    let plugin = tcp_plugin();

    // Act
    let started = guest.start(plugin);

    // Assert
    assert_eq!(
        started.unwrap(),
        Started::Serving(ContextId::try_from(1).unwrap())
    );
    let logs = exercise.recorder.logs();
    assert_eq!(
        logs[..4],
        [
            "vm_start configuration=exercise".to_owned(),
            "environment EXERCISE=on".to_owned(),
            "arguments count=0".to_owned(),
            format!("clock host={NOW} wasi={NOW}"),
        ]
    );
    assert!(logs[4].starts_with("random hash="), "{}", logs[4]);
    assert_eq!(
        logs[5..],
        [
            "log_level level=1",
            "stdout ok",
            "configure root_id=tcp configuration=tcp",
        ]
    );
    assert!(exercise.recorder.calls().is_empty());
    assert!(guest.take_changes().is_empty());
}

#[test]
fn two_guests_draw_different_random_keys() {
    // Arrange
    let exercise = Exercise::new();
    let _first = exercise.started(tcp_plugin());
    let mut second = exercise.spec.build().unwrap();
    let plugin = tcp_plugin();

    // Act
    let started = second.start(plugin);

    // Assert
    assert!(matches!(started, Ok(Started::Serving(_))));
    let hashes: Vec<String> = exercise
        .recorder
        .logs()
        .into_iter()
        .filter(|line| line.starts_with("random hash="))
        .collect();
    assert_eq!(hashes.len(), 2);
    assert_ne!(hashes[0], hashes[1]);
}

#[test]
fn the_http_root_registers_a_queue_defines_metrics_and_opens_two_grpc_callouts() {
    // Arrange
    let exercise = Exercise::new();
    let mut guest = exercise.spec.build().unwrap();
    let plugin = http_plugin("exercise");

    // Act
    let started = guest.start(plugin);

    // Assert
    let root = started.unwrap().root();
    let queue = guest.take_changes().queues.first().unwrap().queue;
    assert_eq!(
        exercise.recorder.calls(),
        [
            Event::RegisterQueue("exercise".into(), queue),
            Event::ResolveQueue("exercise".into(), queue),
            Event::DefineMetric(MetricType::Counter, "exercise_requests".into()),
            Event::DefineMetric(MetricType::Gauge, "exercise_gauge".into()),
            Event::GrpcCall {
                callout: callout(1),
                service: "exercise.Echo".into(),
                method: "Say".into(),
                message: "hello".into()
            },
            Event::GrpcStream {
                callout: callout(2),
                service: "exercise.Echo".into(),
                method: "Chat".into()
            },
            Event::GrpcSend {
                callout: callout(2),
                message: "first".into(),
                end_of_stream: false
            },
        ]
    );
    let invocations: Vec<_> = exercise
        .recorder
        .recorded()
        .into_iter()
        .filter_map(|(at, _)| at)
        .map(|at| (at.guest, at.context, at.callback))
        .collect();
    assert_eq!(
        invocations,
        [(guest.id(), root, Some(Callback::Configure)); 7]
    );
    assert_eq!(
        exercise.recorder.logs()[7..],
        [
            "configure root_id=http configuration=exercise",
            "queue registered=1 resolved=1",
            "metrics counter=1 gauge=2",
            "grpc_call id=1",
            "grpc_stream id=2",
            "grpc_send id=2",
        ]
    );
    let kinds: Vec<_> = guest.open_callouts().iter().map(|open| open.kind).collect();
    assert_eq!(kinds, [CalloutKind::GrpcCall, CalloutKind::GrpcStream]);
    assert_eq!(guest.tick_period(root), Some(Duration::from_millis(100)));
    assert_eq!(guest.queue_registrants(queue), [root]);
}

#[test]
fn a_tick_updates_the_metrics_and_cancels_a_new_grpc_call() {
    // Arrange
    let exercise = Exercise::new();
    let (mut guest, root) = exercise.started(http_plugin("exercise"));
    exercise.recorder.clear();

    // Act
    let ticked = guest.enter_root().on_tick(root);

    // Assert
    ticked.unwrap();
    assert_eq!(
        exercise.recorder.logs(),
        ["tick counter=1 gauge=7", "grpc_cancel id=3"]
    );
    assert!(matches!(
        exercise.recorder.calls()[..],
        [
            Event::IncrementMetric(_, 1),
            Event::RecordMetric(_, 7),
            Event::GrpcCall { callout: opened, .. },
            Event::GrpcCancel(cancelled),
        ] if opened == callout(3) && cancelled == callout(3)
    ));
    let (at, _) = exercise.recorder.recorded().into_iter().nth(3).unwrap();
    assert_eq!(at.unwrap().callback, Some(Callback::Tick));
    let open: Vec<_> = guest
        .open_callouts()
        .iter()
        .map(|open| open.callout)
        .collect();
    assert_eq!(open, [callout(1), callout(2)]);
}

#[test]
fn a_grpc_call_response_reaches_the_root_with_its_status() {
    // Arrange
    let exercise = Exercise::new();
    let (mut guest, root) = exercise.started(http_plugin("exercise"));
    let call = callout(1);
    let message = Cow::Borrowed(b"world".as_slice());
    exercise.recorder.clear();

    // Act
    let delivered = guest.enter_root().on_grpc_receive(root, call, message);

    // Assert
    delivered.unwrap();
    assert_eq!(
        exercise.recorder.logs(),
        ["grpc_response id=1 status=0 message= body=world"]
    );
    assert!(exercise.recorder.calls().is_empty());
    assert_eq!(guest.open_callout(call), None);
    assert_eq!(guest.open_callout_count(), 1);
}

#[test]
fn a_failed_grpc_call_reaches_the_root_with_its_status_and_message() {
    // Arrange
    let exercise = Exercise::new();
    let (mut guest, root) = exercise.started(http_plugin("exercise"));
    let call = callout(1);
    let status = GrpcStatus::new(14, "unavailable");
    exercise.recorder.clear();

    // Act
    let delivered = guest.enter_root().on_grpc_close(root, call, status);

    // Assert
    delivered.unwrap();
    assert_eq!(
        exercise.recorder.logs(),
        ["grpc_response id=1 status=14 message=unavailable body="]
    );
    assert_eq!(guest.open_callout(call), None);
}

#[test]
fn a_grpc_stream_runs_from_its_metadata_to_its_close() {
    // Arrange
    let exercise = Exercise::new();
    let (mut guest, root) = exercise.started(http_plugin("exercise"));
    let stream = callout(2);
    let replies = Replies {
        initial: vec![pair("k", "v")],
        message: Cow::Borrowed(b"reply"),
        trailing: vec![pair("t", "done")],
        status: GrpcStatus::new(14, "unavailable"),
    };
    exercise.recorder.clear();

    // Act
    let delivered = run_stream(&mut guest, root, stream, replies);

    // Assert
    delivered.unwrap();
    assert_eq!(
        exercise.recorder.events(),
        [
            info("grpc_initial id=2 pairs=k:v"),
            Event::GrpcClose(stream),
            info("grpc_close id=2"),
            info("grpc_message id=2 body=reply"),
            info("grpc_trailing id=2 pairs=t:done"),
            info("grpc_stream_close id=2 code=14 status=14 message=unavailable"),
        ]
    );
    let open: Vec<_> = guest
        .open_callouts()
        .iter()
        .map(|open| open.callout)
        .collect();
    assert_eq!(open, [callout(1)]);
}
