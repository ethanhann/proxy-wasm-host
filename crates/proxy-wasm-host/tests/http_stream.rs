//! One HTTP request through the `exercise-all` guest, with its callout, its
//! body, its response, its foreign function call, and its queue item.
//!
//! The helpers below are test code, and the allowance clippy makes for a test
//! does not reach a function of an integration test that carries no test
//! attribute.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::borrow::Cow;

use proxy_wasm_host::abi::v0_2_1::types::{Action, StreamType};
use proxy_wasm_host::abi::v0_2_1::{
    CallScope, Callback, ContextId, ContextState, Guest, GuestError, HttpCallResponse, StreamKind,
};

mod common;

use common::harness::{Exercise, http_plugin, pair, stream_of};
use common::recorder::{Event, RecordingStream, pairs};

/// The sizes the callbacks after the request headers announce.
struct Script {
    request_body: u32,
    response_headers: u32,
    response_body: u32,
}

/// Runs the callbacks after the request headers, up to the response trailers.
fn rest_of_request(
    scope: &mut CallScope<'_, RecordingStream>,
    stream: ContextId,
    script: &Script,
) -> Result<Vec<Action>, GuestError> {
    Ok(vec![
        scope.on_request_body(stream, script.request_body, false)?,
        scope.on_request_trailers(stream, 0)?,
        scope.on_response_headers(stream, script.response_headers, false)?,
        scope.on_response_body(stream, script.response_body, true)?,
        scope.on_response_trailers(stream, 0)?,
    ])
}

/// A started guest with one HTTP stream context and the request lent to it.
struct Request {
    exercise: Exercise,
    guest: Guest,
    root: ContextId,
    stream: ContextId,
    state: RecordingStream,
    count: u32,
}

impl Request {
    /// A stream context whose request headers have not run.
    fn created(request_headers: &[(&str, &str)]) -> Self {
        let exercise = Exercise::new();
        let (mut guest, root) = exercise.started(http_plugin("exercise"));
        let state = RecordingStream::new(&exercise.recorder).with_request_headers(request_headers);
        let count = u32::try_from(pairs(&state.request_headers).len()).unwrap();
        let (stream, state) = stream_of(&mut guest, root, StreamKind::Http, state);
        exercise.recorder.clear();
        Self {
            exercise,
            guest,
            root,
            stream,
            state,
            count,
        }
    }

    /// A stream context past its request headers.
    fn with_headers(request_headers: &[(&str, &str)]) -> Self {
        let Self {
            exercise,
            mut guest,
            root,
            stream,
            state,
            count,
        } = Self::created(request_headers);
        let (answer, state) = guest.with(state, |scope| {
            scope.on_request_headers(stream, count, false)
        });
        answer.unwrap();
        exercise.recorder.clear();
        Self {
            exercise,
            guest,
            root,
            stream,
            state,
            count,
        }
    }
}

#[test]
fn a_request_reaches_every_family_of_the_request_headers() {
    // Arrange
    let Request {
        exercise,
        mut guest,
        stream,
        state,
        count,
        ..
    } = Request::created(&[("x-removed", "a"), ("x-replaced", "b")]);

    // Act
    let (answer, state) = guest.with(state, |scope| {
        scope.on_request_headers(stream, count, false)
    });

    // Assert
    assert_eq!(answer.unwrap(), Action::Pause);
    assert_eq!(
        pairs(&state.request_headers),
        [
            ("x-replaced".to_owned(), "2".to_owned()),
            ("x-added".to_owned(), "1".to_owned())
        ]
    );
    let queue = guest.take_changes().queues.first().unwrap().queue;
    let calls = exercise.recorder.calls();
    assert_eq!(
        calls[..4],
        [
            Event::SetProperty(vec!["exercise".into(), "seen".into()], "yes".into()),
            Event::SetSharedData("exercise".into(), "seen".into()),
            Event::Enqueue(queue, format!("from {stream}")),
            Event::Enqueued(queue, "exercise".into()),
        ]
    );
    assert!(matches!(
        &calls[4..],
        [Event::HttpCall { upstream, .. }] if upstream == "upstream"
    ));
    let invocations: Vec<_> = exercise
        .recorder
        .recorded()
        .into_iter()
        .filter_map(|(at, _)| at)
        .map(|at| (at.guest, at.context, at.callback))
        .collect();
    assert_eq!(
        invocations,
        [(guest.id(), stream, Some(Callback::RequestHeaders)); 4]
    );
    assert_eq!(
        exercise.recorder.logs(),
        [
            "request_headers count=2:2 path=/exercise",
            "shared_data value= cas=0",
            "enqueue queue=1",
            "http_call id=3",
        ]
    );
    assert_eq!(
        state.properties[&vec!["exercise".into(), "seen".into()]],
        "yes"
    );
}

#[test]
fn the_second_request_reads_the_shared_data_of_the_first() {
    // Arrange
    let Request {
        exercise,
        mut guest,
        root,
        ..
    } = Request::with_headers(&[]);
    let state = RecordingStream::new(&exercise.recorder);
    let (second, state) = stream_of(&mut guest, root, StreamKind::Http, state);
    exercise.recorder.clear();

    // Act
    let (answer, _) = guest.with(state, |scope| scope.on_request_headers(second, 0, false));

    // Assert
    answer.unwrap();
    assert_eq!(exercise.recorder.logs()[1], "shared_data value=seen cas=1");
}

#[test]
fn an_http_call_response_resumes_the_request() {
    // Arrange
    let Request {
        exercise,
        mut guest,
        stream,
        state,
        ..
    } = Request::with_headers(&[]);
    let callout = guest.open_callouts().last().unwrap().callout;
    let response = HttpCallResponse::received(vec![pair(":status", "200")])
        .with_body(Cow::Borrowed(b"ok"))
        .with_trailers(vec![pair("t", "1")]);

    // Act
    let (answer, _) = guest.with(state, |scope| {
        scope.on_http_call_response(stream, callout, response)
    });

    // Assert
    answer.unwrap();
    assert_eq!(
        exercise.recorder.logs(),
        ["http_response headers=1:1 body=ok trailers=1:1"]
    );
    let recorded = exercise.recorder.recorded();
    let (at, event) = recorded.last().unwrap();
    assert_eq!(event, &Event::ContinueStream(StreamType::HttpRequest));
    let at = at.unwrap();
    assert_eq!(
        (at.context, at.callback, at.callout),
        (stream, Some(Callback::HttpCallResponse), Some(callout))
    );
    assert_eq!(guest.open_callout(callout), None);
}

#[test]
fn the_body_trailers_and_response_reach_the_guest() {
    // Arrange
    let Request {
        exercise,
        mut guest,
        stream,
        mut state,
        ..
    } = Request::with_headers(&[]);
    state.request_body = b"hello".to_vec();
    state.response_headers = vec![
        (b"a".to_vec(), b"1".to_vec()),
        (b"b".to_vec(), b"2".to_vec()),
    ]
    .into();
    state.response_body = b"answer".to_vec();
    let script = Script {
        request_body: 5,
        response_headers: 2,
        response_body: 6,
    };

    // Act
    let (answer, state) = guest.with(state, |scope| rest_of_request(scope, stream, &script));

    // Assert
    assert_eq!(answer.unwrap(), [Action::Continue; 5]);
    assert_eq!(state.request_body, b"replaced");
    assert_eq!(
        pairs(&state.request_trailers),
        [("x-trailer".to_owned(), "set".to_owned())]
    );
    assert_eq!(
        exercise.recorder.logs(),
        [
            "request_body size=5 end=false body=hello",
            "request_trailers count=0",
            "response_headers count=2:2",
            "foreign_call answer=pong",
            "response_body size=6 end=true body=answer",
            "response_trailers count=0:0",
        ]
    );
    assert_eq!(
        exercise.recorder.calls(),
        [Event::ForeignCall {
            name: "exercise_echo".into(),
            arguments: "ping".into()
        }]
    );
}

#[test]
fn a_foreign_function_call_reaches_the_request_with_its_arguments() {
    // Arrange
    let Request {
        exercise,
        mut guest,
        stream,
        state,
        ..
    } = Request::with_headers(&[]);
    let arguments = Cow::Borrowed(b"args".as_slice());

    // Act
    let (answer, _) = guest.with(state, |scope| {
        scope.on_foreign_function(stream, 7, arguments)
    });

    // Assert
    answer.unwrap();
    assert_eq!(
        exercise.recorder.logs(),
        ["foreign_function id=7 arguments=args"]
    );
    assert!(exercise.recorder.calls().is_empty());
}

#[test]
fn an_enqueue_reaches_the_root_that_registered_the_queue() {
    // Arrange
    let Request {
        exercise,
        mut guest,
        root,
        stream,
        ..
    } = Request::with_headers(&[]);
    let queue = guest.take_changes().queues.first().unwrap().queue;
    let registrants = guest.queue_registrants(queue);

    // Act
    let delivered = guest.enter_root().on_queue_ready(registrants[0], queue);

    // Assert
    delivered.unwrap();
    assert_eq!(registrants, [root]);
    let item = format!("from {stream}");
    assert_eq!(
        exercise.recorder.logs(),
        [format!("queue_ready queue=1 item={item}")]
    );
    assert_eq!(exercise.recorder.calls(), [Event::Dequeue(queue, item)]);
}

#[test]
fn a_deferred_done_completes_at_the_next_tick() {
    // Arrange
    let Request {
        exercise,
        mut guest,
        root,
        stream,
        state,
        ..
    } = Request::with_headers(&[("x-exercise", "defer")]);
    let (done, _) = guest.with(state, |scope| scope.on_done(stream));
    let pending = guest.context_state(stream);
    exercise.recorder.clear();

    // Act
    let ticked = guest.enter_root().on_tick(root);

    // Assert
    ticked.unwrap();
    assert!(!done.unwrap());
    assert_eq!(pending, Some(ContextState::Pending));
    assert_eq!(guest.context_state(stream), Some(ContextState::Done));
    assert_eq!(
        exercise.recorder.logs().last().unwrap(),
        &format!("done context={stream}")
    );
}

#[test]
fn a_local_response_reaches_the_stream_state() {
    // Arrange
    let Request {
        exercise,
        mut guest,
        stream,
        state,
        count,
        ..
    } = Request::created(&[("x-exercise", "local")]);

    // Act
    let (answer, _) = guest.with(state, |scope| {
        scope.on_request_headers(stream, count, false)
    });

    // Assert
    assert_eq!(answer.unwrap(), Action::Pause);
    assert_eq!(
        exercise.recorder.calls(),
        [Event::LocalResponse {
            status: 403,
            headers: vec![("x-local".into(), "yes".into())],
            body: "denied".into()
        }]
    );
    assert_eq!(
        exercise.recorder.logs(),
        [
            "request_headers count=1:1 path=/exercise",
            "local_response status=403"
        ]
    );
}
