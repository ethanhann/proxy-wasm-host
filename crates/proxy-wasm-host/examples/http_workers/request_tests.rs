//! The tests of the request, the answer, the headers, and the sink of the
//! `http_workers` example.

use std::io::{Read, Write};
use std::sync::{Arc, Mutex, PoisonError};

use proxy_wasm_host::HeaderMap;
use proxy_wasm_host::abi::v0_2_1::GuestId;
use proxy_wasm_host::abi::v0_2_1::types::{Action, LogLevel};
use tiny_http::{Header, TestRequest};

use super::*;

/// The authority of a server on the default port.
const AUTHORITY: &str = "127.0.0.1:2045";

fn test_request(headers: &[(&str, &str)]) -> Request {
    let mut test = TestRequest::new().with_path("/example");
    for (name, value) in headers {
        test = test.with_header(format!("{name}: {value}").parse::<Header>().unwrap());
    }
    Request::from(test)
}

fn request(headers: &[(&str, &str)]) -> HttpRequest {
    request_state(&test_request(headers), AUTHORITY)
}

#[test]
fn the_request_state_holds_the_three_pseudo_headers() {
    // Arrange
    let source = test_request(&[]);

    // Act
    let state = request_state(&source, AUTHORITY);

    // Assert
    let names: Vec<String> = pairs(&state.headers)
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    assert_eq!(names[..3], [":method", ":path", ":authority"]);
    assert_eq!(pairs(&state.headers)[1].1, "/example");
}

#[test]
fn the_authority_is_the_one_the_server_listens_on() {
    // Arrange
    let source = test_request(&[]);

    // Act
    let state = request_state(&source, "127.0.0.1:8080");

    // Assert
    assert_eq!(
        pairs(&state.headers)[2],
        (":authority".to_owned(), "127.0.0.1:8080".to_owned())
    );
}

#[test]
fn the_answer_lists_a_header_the_guest_added_in_lower_case() {
    // Arrange
    let mut state = request(&[]);
    state.headers.add(b"Wasm-Context", b"3").unwrap();

    // Act
    let answer = answer_of(&state, Action::Continue);

    // Assert
    let mut body = String::new();
    answer.into_reader().read_to_string(&mut body).unwrap();
    assert_eq!(
        body,
        ":method: GET\n:path: /example\n:authority: 127.0.0.1:2045\ncontent-length: 0\nwasm-context: 3\n"
    );
}

#[test]
fn a_replaced_header_is_one_lower_case_pair_at_the_end() {
    // Arrange
    let mut state = request(&[("Wasm-Context", "99"), ("x-trace", "7")]);

    // Act
    state.headers.set(b"Wasm-Context", b"3").unwrap();

    // Assert
    let names: Vec<(String, String)> = pairs(&state.headers).into_iter().skip(3).collect();
    assert_eq!(
        names,
        vec![
            ("x-trace".to_owned(), "7".to_owned()),
            ("content-length".to_owned(), "0".to_owned()),
            ("wasm-context".to_owned(), "3".to_owned()),
        ]
    );
}

#[test]
fn a_header_is_found_whatever_the_case_of_its_name() {
    // Arrange
    let state = request(&[("x-trace", "7")]);

    // Act
    let found = state.headers.get(b"X-Trace");

    // Assert
    assert_eq!(found.as_deref(), Some(&b"7"[..]));
}

#[test]
fn the_sink_maps_each_level_to_its_tracing_level() {
    // Arrange
    let lines = Lines::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(lines.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .finish();
    let context = LogContext::new(b"vm", GuestId::next());
    let levels = [
        LogLevel::Trace,
        LogLevel::Debug,
        LogLevel::Info,
        LogLevel::Warn,
        LogLevel::Error,
        LogLevel::Critical,
    ];

    // Act
    tracing::subscriber::with_default(subscriber, || {
        for level in levels {
            TracingSink.log(context.clone(), level, b"line");
        }
    });

    // Assert
    let text = lines.text();
    let seen: Vec<&str> = text
        .lines()
        .filter_map(|line| line.split_whitespace().nth(1))
        .collect();
    assert_eq!(seen, ["TRACE", "DEBUG", "INFO", "WARN", "ERROR", "ERROR"]);
    assert_eq!(text.matches("guest: line").count(), 6, "{text}");
}

/// A writer that keeps every line a subscriber writes.
#[derive(Clone, Default)]
struct Lines(Arc<Mutex<Vec<u8>>>);

impl Lines {
    fn text(&self) -> String {
        let bytes = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        String::from_utf8(bytes).unwrap()
    }
}

impl Write for Lines {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Lines {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}
