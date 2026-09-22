//! The answer tables of the test doubles.
//!
//! Every other test file relies on the doubles accepting each call, and two
//! tests of the third party guests rely on them refusing one. These tests
//! prove both halves, so a failure elsewhere is a failure of the crate and
//! not of the support code.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use proxy_wasm_host::abi::v0_2_1::types::Status;
use proxy_wasm_host::abi::v0_2_1::{
    ContextId, GrpcOpenRefusal, Guest, HttpCallRefusal, StreamKind,
};

mod common;

use common::answers::{CalloutCall, CalloutRefusal, StreamCall};
use common::harness::{Exercise, http_plugin, stream_of};
use common::recorder::{Event, StreamDouble};

/// Runs the request headers callback of the `exercise-all` guest and reports
/// whether it returned.
fn request_headers(
    guest: &mut Guest,
    root: ContextId,
    state: StreamDouble,
) -> (bool, StreamDouble) {
    let (stream, state) = stream_of(guest, root, StreamKind::Http, state);
    let (outcome, state) = guest.with(state, |scope| scope.on_request_headers(stream, 1, false));
    (outcome.is_ok(), state)
}

#[test]
fn a_method_with_no_entry_keeps_its_answer() {
    // Arrange
    let exercise = Exercise::new();
    let (mut guest, root) = exercise.started(http_plugin("doubles"));
    let state = StreamDouble::new(&exercise.recorder);

    // Act
    let (returned, _state) = request_headers(&mut guest, root, state);

    // Assert
    assert!(returned, "the callback must return");
    assert!(
        exercise
            .recorder
            .logs()
            .iter()
            .any(|line| line.contains("path=/exercise")),
        "the property must reach the guest when no refusal is asked for"
    );
}

#[test]
fn a_refusal_reaches_the_guest_as_its_status() {
    // Arrange
    let exercise = Exercise::new();
    let (mut guest, root) = exercise.started(http_plugin("doubles"));
    let mut state = StreamDouble::new(&exercise.recorder);
    state
        .refusals
        .insert(StreamCall::Property, Status::NotFound);

    // Act
    let (returned, _state) = request_headers(&mut guest, root, state);

    // Assert
    assert!(returned, "a refused property must not end the callback");
    assert!(
        exercise
            .recorder
            .logs()
            .iter()
            .any(|line| line.starts_with("request_headers ") && line.ends_with("path=")),
        "the guest must read an empty path when the double refuses the property"
    );
}

#[test]
fn a_callout_refusal_reaches_the_guest() {
    // Arrange
    let exercise = Exercise::new();
    let (mut guest, root) = exercise.started(http_plugin("doubles"));
    let state = StreamDouble::new(&exercise.recorder);
    exercise.recorder.refuse(
        CalloutCall::HttpCall,
        CalloutRefusal::Http(HttpCallRefusal::UnknownUpstream),
    );

    // Act
    let (returned, _state) = request_headers(&mut guest, root, state);

    // Assert
    assert!(
        !returned,
        "the guest unwraps the open of its callout, so a refusal traps it"
    );
    assert!(guest.is_poisoned());
    assert!(
        !exercise
            .recorder
            .calls()
            .iter()
            .any(|event| matches!(event, Event::HttpCall { .. })),
        "a refused callout must record no call"
    );
}

#[test]
fn clear_empties_the_events_and_the_callout_refusals() {
    // Arrange
    let exercise = Exercise::new();
    let (mut guest, root) = exercise.started(http_plugin("doubles"));
    exercise.recorder.refuse(
        CalloutCall::HttpCall,
        CalloutRefusal::Http(HttpCallRefusal::UnknownUpstream),
    );
    let state = StreamDouble::new(&exercise.recorder);
    let before = exercise.recorder.events().len();

    // Act
    exercise.recorder.clear();

    // Assert
    assert!(before > 0, "the start must have recorded something");
    assert!(
        exercise.recorder.events().is_empty(),
        "clear must empty the event list"
    );
    let (returned, _state) = request_headers(&mut guest, root, state);
    assert!(returned, "the callout must be accepted again");
    assert!(
        exercise
            .recorder
            .calls()
            .iter()
            .any(|event| matches!(event, Event::HttpCall { .. })),
        "the accepted callout must be recorded"
    );
}

#[test]
fn a_refusal_that_a_method_cannot_answer_panics() {
    // Arrange
    let exercise = Exercise::new();
    let (mut guest, root) = exercise.started(http_plugin("doubles"));
    let state = StreamDouble::new(&exercise.recorder);
    exercise.recorder.refuse(
        CalloutCall::HttpCall,
        CalloutRefusal::GrpcOpen(GrpcOpenRefusal::Failed),
    );

    // Act
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        request_headers(&mut guest, root, state)
    }));

    // Assert
    assert!(
        panicked.is_err(),
        "a refusal of the wrong kind must not pass as an acceptance"
    );
}
