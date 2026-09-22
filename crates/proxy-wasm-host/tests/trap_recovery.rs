//! A guest that panics or refuses its start, and a new guest from the same
//! `GuestSpec` that serves the next request.
//!
//! The helpers below are test code, and the allowance clippy makes for a test
//! does not reach a function of an integration test that carries no test
//! attribute.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use proxy_wasm_host::Error;
use proxy_wasm_host::abi::v0_2_1::types::{Action, LogLevel};
use proxy_wasm_host::abi::v0_2_1::{
    Callback, CalloutId, CalloutKind, ContextId, Guest, GuestError, Started, StreamKind,
};

mod common;

use common::harness::{Exercise, http_plugin, stream_of};
use common::recorder::{Event, RecordingStream};

/// A started guest whose request headers panicked.
fn panicked(exercise: &Exercise) -> (Guest, Result<Action, GuestError>) {
    let (mut guest, root) = exercise.started(http_plugin("exercise"));
    let state =
        RecordingStream::new(&exercise.recorder).with_request_headers(&[("x-exercise", "panic")]);
    let (stream, state) = stream_of(&mut guest, root, StreamKind::Http, state);
    let (answer, _) = guest.with(state, |scope| scope.on_request_headers(stream, 1, false));
    (guest, answer)
}

/// Ends a root that its configuration refused, and answers the callouts that
/// the deletion ended.
fn tear_down(guest: &mut Guest, root: ContextId) -> Result<Vec<CalloutId>, GuestError> {
    let mut scope = guest.enter_root();
    scope.on_done(root)?;
    scope.on_delete(root)
}

#[test]
fn a_panic_in_a_request_poisons_the_guest() {
    // Arrange
    let exercise = Exercise::new();
    let (mut guest, answer) = panicked(&exercise);
    let stream = ContextId::try_from(2).unwrap();
    let state = RecordingStream::default();

    // Act
    let later = guest.enter(state).on_log(stream);

    // Assert
    assert!(matches!(
        answer,
        Err(GuestError::Runtime(Error::Trap { message, .. })) if message.contains("unreachable")
    ));
    assert!(guest.is_poisoned());
    assert!(!guest.is_serving());
    assert!(matches!(later, Err(GuestError::Runtime(Error::Poisoned))));
}

#[test]
fn the_panic_text_reaches_the_log_sink() {
    // Arrange
    let exercise = Exercise::new();

    // Act
    let (_, answer) = panicked(&exercise);

    // Assert
    assert!(answer.is_err());
    let critical = exercise.recorder.logs_at(LogLevel::Critical);
    assert_eq!(critical.len(), 1);
    assert!(
        critical[0].contains("exercise panic in request headers"),
        "{critical:?}"
    );
}

#[test]
fn a_trap_in_the_configuration_leaves_the_open_callouts_readable() {
    // Arrange
    let exercise = Exercise::new();
    let mut guest = exercise.spec.build().unwrap();
    let plugin = http_plugin("panic");

    // Act
    let started = guest.start(plugin);

    // Assert
    assert!(matches!(
        started,
        Err(GuestError::Runtime(Error::Trap { .. }))
    ));
    assert!(guest.is_poisoned());
    let open = guest.open_callouts();
    let kinds: Vec<_> = open.iter().map(|callout| callout.kind).collect();
    assert_eq!(kinds, [CalloutKind::GrpcCall, CalloutKind::GrpcStream]);
    let opened: Vec<_> = exercise
        .recorder
        .calls()
        .iter()
        .filter_map(|event| match event {
            Event::GrpcCall { callout, .. } | Event::GrpcStream { callout, .. } => Some(*callout),
            _ => None,
        })
        .collect();
    let listed: Vec<_> = open.iter().map(|callout| callout.callout).collect();
    assert_eq!(listed, opened);
    assert!(
        exercise.recorder.logs_at(LogLevel::Critical)[0].contains("exercise panic in configure")
    );
}

#[test]
fn a_refused_vm_start_takes_the_guest_out_of_service() {
    // Arrange
    let exercise = Exercise::with_vm_configuration("refuse");
    let mut guest = exercise.spec.build().unwrap();
    let plugin = http_plugin("exercise");

    // Act
    let started = guest.start(plugin);

    // Assert
    assert_eq!(
        started.unwrap(),
        Started::Refused {
            root: ContextId::try_from(1).unwrap(),
            callback: Callback::VmStart
        }
    );
    assert!(!guest.is_poisoned());
    assert!(!guest.is_serving());
    assert!(
        exercise
            .recorder
            .logs()
            .iter()
            .all(|line| !line.starts_with("configure"))
    );
}

#[test]
fn a_refused_configuration_keeps_its_callouts_until_its_root_is_deleted() {
    // Arrange
    let exercise = Exercise::new();
    let mut guest = exercise.spec.build().unwrap();
    let refused = guest.start(http_plugin("refuse")).unwrap();
    let open: Vec<_> = guest
        .open_callouts()
        .iter()
        .map(|open| open.callout)
        .collect();
    let root = refused.root();

    // Act
    let ended = tear_down(&mut guest, root);

    // Assert
    assert_eq!(
        refused,
        Started::Refused {
            root,
            callback: Callback::Configure
        }
    );
    assert_eq!(open.len(), 2);
    assert_eq!(ended.unwrap(), open);
    assert!(guest.open_callouts().is_empty());
    assert_eq!(guest.context_type(root), None);
    assert!(guest.is_serving());
}

#[test]
fn a_root_id_is_refused_while_its_refused_root_remains() {
    // Arrange
    let exercise = Exercise::new();
    let mut guest = exercise.spec.build().unwrap();
    let refused = guest.start(http_plugin("refuse")).unwrap().root();
    let plugin = http_plugin("exercise");

    // Act
    let second = guest.start(plugin);

    // Assert
    assert!(matches!(
        second,
        Err(GuestError::DuplicateRootId { root_id, root }) if root_id == b"http" && root == refused
    ));
}

#[test]
fn a_new_guest_from_the_same_guest_spec_serves_the_next_request() {
    // Arrange
    let exercise = Exercise::new();
    let (old, _) = panicked(&exercise);
    let mut guest = exercise.spec.build().unwrap();
    let started = guest.start(http_plugin("exercise")).unwrap();
    let root = started.root();
    let state = RecordingStream::new(&exercise.recorder);
    let (stream, state) = stream_of(&mut guest, root, StreamKind::Http, state);

    // Act
    let (answer, _) = guest.with(state, |scope| scope.on_request_headers(stream, 0, false));

    // Assert
    assert_eq!(started, Started::Serving(ContextId::try_from(1).unwrap()));
    assert_eq!(answer.unwrap(), Action::Pause);
    assert!(old.is_poisoned());
    assert!(guest.is_serving());
    assert_ne!(guest.id(), old.id());
}
