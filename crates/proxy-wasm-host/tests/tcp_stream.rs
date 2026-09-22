//! One TCP connection through the `exercise-all` guest, from its start to
//! both closes.
//!
//! The helpers below are test code, and the allowance clippy makes for a test
//! does not reach a function of an integration test that carries no test
//! attribute.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use proxy_wasm_host::abi::v0_2_1::types::{Action, PeerType, StreamType};
use proxy_wasm_host::abi::v0_2_1::{CallScope, Callback, ContextId, GuestError, StreamKind};

mod common;

use common::harness::{Exercise, tcp_plugin};
use common::recorder::{Event, StreamDouble};

/// Writes `data` as the whole of `buffer` before the next callback.
fn fill(buffer: &mut Vec<u8>, data: &[u8]) {
    buffer.clear();
    buffer.extend_from_slice(data);
}

/// What the script saw: the answers of the data callbacks, the buffers after
/// each one, and the context.
#[derive(Debug, PartialEq)]
struct Seen {
    actions: Vec<Action>,
    downstream: Vec<Vec<u8>>,
    upstream: Vec<u8>,
    context: ContextId,
}

/// Runs one connection from the new connection to its deletion.
fn connection(
    scope: &mut CallScope<'_, StreamDouble>,
    root: ContextId,
) -> Result<Seen, GuestError> {
    let context = scope.on_context_create(Some(root))?;
    scope.expect_stream_kind(context, StreamKind::Tcp)?;
    let mut actions = vec![scope.on_new_connection(context)?];
    let mut downstream = Vec::new();
    fill(&mut scope.stream_mut().downstream, b"hello");
    actions.push(scope.on_downstream_data(context, 5, false)?);
    downstream.push(scope.stream().downstream.clone());
    fill(&mut scope.stream_mut().downstream, b"pause");
    actions.push(scope.on_downstream_data(context, 5, false)?);
    downstream.push(scope.stream().downstream.clone());
    fill(&mut scope.stream_mut().upstream, b"world");
    actions.push(scope.on_upstream_data(context, 5, true)?);
    let upstream = scope.stream().upstream.clone();
    scope.on_downstream_connection_close(context, PeerType::Remote)?;
    scope.on_upstream_connection_close(context, PeerType::Local)?;
    scope.on_done(context)?;
    scope.on_log(context)?;
    scope.on_delete(context)?;
    Ok(Seen {
        actions,
        downstream,
        upstream,
        context,
    })
}

#[test]
fn a_connection_runs_from_its_start_to_both_closes() {
    // Arrange
    let exercise = Exercise::new();
    let (mut guest, root) = exercise.started(tcp_plugin());
    let state = StreamDouble::new(&exercise.recorder);
    exercise.recorder.clear();

    // Act
    let (seen, _) = guest.with(state, |scope| connection(scope, root));

    // Assert
    let context = seen.unwrap().context;
    assert_eq!(
        exercise.recorder.logs(),
        [
            format!("new_connection context={context}"),
            "downstream_data size=5 end=false data=hello".to_owned(),
            "downstream_data size=5 end=false data=pause".to_owned(),
            "upstream_data size=5 end=true data=world".to_owned(),
            "downstream_close peer=Remote".to_owned(),
            "upstream_close peer=Local".to_owned(),
            format!("log context={context}"),
        ]
    );
    assert_eq!(guest.context_state(context), None);
}

#[test]
fn the_guest_changes_the_data_in_each_direction() {
    // Arrange
    let exercise = Exercise::new();
    let (mut guest, root) = exercise.started(tcp_plugin());
    let state = StreamDouble::new(&exercise.recorder);
    exercise.recorder.clear();

    // Act
    let (seen, _) = guest.with(state, |scope| connection(scope, root));

    // Assert
    let seen = seen.unwrap();
    assert_eq!(
        seen.actions,
        [
            Action::Continue,
            Action::Continue,
            Action::Pause,
            Action::Continue
        ]
    );
    assert_eq!(seen.downstream, [b"HELLO".to_vec(), b"pause".to_vec()]);
    assert_eq!(seen.upstream, b"WORLD");
    let calls: Vec<_> = exercise
        .recorder
        .recorded()
        .into_iter()
        .filter_map(|(at, event)| at.map(|at| (at.context, at.callback, event)))
        .collect();
    assert_eq!(
        calls,
        [
            (
                seen.context,
                Some(Callback::UpstreamData),
                Event::ContinueStream(StreamType::Downstream)
            ),
            (
                seen.context,
                Some(Callback::UpstreamData),
                Event::CloseStream(StreamType::Upstream)
            ),
        ]
    );
}
