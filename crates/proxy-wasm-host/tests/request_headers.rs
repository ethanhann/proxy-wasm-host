//! The request header lifecycle against the two committed guests.
//!
//! The helpers below are test code, and the allowance clippy makes for a test
//! does not reach a function of an integration test that carries no test
//! attribute.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::borrow::Cow;
use std::ops::ControlFlow;
use std::sync::{Arc, Mutex, PoisonError};

use proxy_wasm_host::abi::AbiVersion;
use proxy_wasm_host::abi::v0_2_1::types::{Action, LogLevel, MapType, Status};
use proxy_wasm_host::abi::v0_2_1::{
    Access, ContextId, Guest, Invocation, PluginConfig, StreamState,
};
use proxy_wasm_host::codec::pairs::PairVisitor;
use proxy_wasm_host::runtime::{Engine, Limits, LogSink, Module, VmServices};
use proxy_wasm_host::{Error, HeaderMap, NotAllowed, VecHeaderMap};

const RUST_SDK: &[u8] = include_bytes!("fixtures/add-request-header.wasm");
const TINYGO: &[u8] = include_bytes!("fixtures/add-request-header-tinygo.wasm");

#[derive(Default)]
struct Sink(Mutex<Vec<(LogLevel, String)>>);

impl LogSink for Sink {
    fn log(&self, level: LogLevel, message: &[u8]) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((level, String::from_utf8_lossy(message).into_owned()));
    }
}

/// A request whose headers the guest may change.
#[derive(Default)]
struct Request {
    headers: VecHeaderMap,
}

impl StreamState for Request {
    fn header_map(
        &mut self,
        _: Invocation,
        _: Access,
        map: MapType,
    ) -> Result<&mut dyn HeaderMap, Status> {
        match map {
            MapType::HttpRequestHeaders => Ok(&mut self.headers),
            _ => Err(Status::NotFound),
        }
    }
}

/// A header map that refuses every write.
#[derive(Default)]
struct Sealed(VecHeaderMap);

impl HeaderMap for Sealed {
    fn get(&self, key: &[u8]) -> Option<Cow<'_, [u8]>> {
        self.0.get(key)
    }

    fn for_each_pair(&self, f: &mut PairVisitor<'_>) -> ControlFlow<()> {
        self.0.for_each_pair(f)
    }

    fn set(&mut self, _: &[u8], _: &[u8]) -> Result<(), NotAllowed> {
        Err(NotAllowed)
    }

    fn add(&mut self, _: &[u8], _: &[u8]) -> Result<(), NotAllowed> {
        Err(NotAllowed)
    }

    fn remove(&mut self, _: &[u8]) -> Result<(), NotAllowed> {
        Err(NotAllowed)
    }

    fn replace_all(&mut self, _: &[(&[u8], &[u8])]) -> Result<(), NotAllowed> {
        Err(NotAllowed)
    }
}

/// A request whose headers the guest may not change.
#[derive(Default)]
struct SealedRequest {
    headers: Sealed,
}

impl StreamState for SealedRequest {
    fn header_map(
        &mut self,
        _: Invocation,
        _: Access,
        map: MapType,
    ) -> Result<&mut dyn HeaderMap, Status> {
        match map {
            MapType::HttpRequestHeaders => Ok(&mut self.headers),
            _ => Err(Status::NotFound),
        }
    }
}

/// Drives one guest through the lifecycle one named step at a time.
struct Lifecycle {
    guest: Guest,
    sink: Arc<Sink>,
    root: Option<ContextId>,
    stream: Option<ContextId>,
    plugin: PluginConfig,
}

impl Lifecycle {
    fn new(bytes: &[u8]) -> Self {
        Self::configured(bytes, Vec::new(), PluginConfig::new())
    }

    fn configured(bytes: &[u8], vm_configuration: Vec<u8>, plugin: PluginConfig) -> Self {
        let engine = Engine::new().unwrap();
        let module = Module::new(&engine, bytes).unwrap();
        let sink = Arc::new(Sink::default());
        let services = VmServices::new(sink.clone()).with_vm_configuration(vm_configuration);
        let guest = Guest::new(&engine, &module, services, &Limits::default()).unwrap();
        Self {
            guest,
            sink,
            root: None,
            stream: None,
            plugin,
        }
    }

    /// Creates the root context, then runs VM start and configure.
    fn start_root(&mut self) -> Result<(ContextId, bool, bool), Error> {
        let mut scope = self.guest.enter_root();
        let root = scope.on_context_create(None)?;
        let started = scope.on_vm_start(root)?;
        let configured = scope.on_configure(root, self.plugin.clone())?;
        self.root = Some(root);
        Ok((root, started, configured))
    }

    fn create_stream(&mut self) -> Result<ContextId, Error> {
        let stream = self.guest.enter_root().on_context_create(self.root)?;
        self.stream = Some(stream);
        Ok(stream)
    }

    fn through_stream(bytes: &[u8]) -> Self {
        Self::through_stream_of(Self::new(bytes))
    }

    fn through_stream_of(mut lifecycle: Self) -> Self {
        lifecycle.start_root().unwrap();
        lifecycle.create_stream().unwrap();
        lifecycle
    }

    fn request_headers<H: StreamState>(&mut self, request: H) -> (Result<Action, Error>, H) {
        let mut scope = self.guest.enter(request);
        let action = scope.on_request_headers(self.stream.unwrap(), 0, true);
        (action, scope.finish())
    }

    /// Runs done, log, and delete on the stream context with `request` lent
    /// to the guest, and gives the request back.
    fn finalize<H: StreamState>(&mut self, request: H) -> (Result<bool, Error>, H) {
        let stream = self.stream.unwrap();
        let mut scope = self.guest.enter(request);
        let done = scope
            .on_done(stream)
            .and_then(|done| scope.on_log(stream).map(|()| done))
            .and_then(|done| scope.on_delete(stream).map(|()| done));
        (done, scope.finish())
    }

    fn logs(&self) -> Vec<(LogLevel, String)> {
        self.sink
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

fn header(request: &Request, key: &str) -> Option<String> {
    request
        .headers
        .get(key.as_bytes())
        .map(|value| String::from_utf8_lossy(&value).into_owned())
}

#[test]
fn the_rust_sdk_guest_starts_its_root_context() {
    // Arrange
    let mut lifecycle = Lifecycle::new(RUST_SDK);

    // Act
    let (root, started, configured) = lifecycle.start_root().unwrap();

    // Assert
    assert_eq!(lifecycle.guest.abi(), AbiVersion::V0_2_1);
    assert_eq!(root.get(), 1);
    assert!(started);
    assert!(configured);
}

#[test]
fn the_rust_sdk_guest_adds_the_context_header_and_logs() {
    // Arrange
    let mut lifecycle = Lifecycle::through_stream(RUST_SDK);

    // Act
    let (action, request) = lifecycle.request_headers(Request::default());

    // Assert
    assert_eq!(action.unwrap(), Action::Continue);
    assert_eq!(lifecycle.stream.unwrap().get(), 2);
    assert_eq!(header(&request, "Wasm-Context").as_deref(), Some("2"));
    assert_eq!(
        lifecycle.logs(),
        vec![(LogLevel::Info, "adding header".to_owned())]
    );
}

#[test]
fn the_rust_sdk_guest_finalizes_its_stream_context() {
    // Arrange
    let mut lifecycle = Lifecycle::through_stream(RUST_SDK);
    let (_, request) = lifecycle.request_headers(Request::default());

    // Act
    let (done, request) = lifecycle.finalize(request);

    // Assert
    assert!(done.unwrap());
    assert_eq!(header(&request, "Wasm-Context").as_deref(), Some("2"));
    assert_eq!(
        lifecycle.guest.context_state(lifecycle.stream.unwrap()),
        None
    );
    assert!(!lifecycle.guest.instance().is_poisoned());
}

#[test]
fn the_tinygo_guest_runs_the_same_lifecycle() {
    // Arrange
    let mut lifecycle = Lifecycle::through_stream(TINYGO);

    // Act
    let (action, request) = lifecycle.request_headers(Request::default());

    // Assert
    assert_eq!(lifecycle.guest.abi(), AbiVersion::V0_2_0);
    assert_eq!(action.unwrap(), Action::Continue);
    assert_eq!(header(&request, "Wasm-Context").as_deref(), Some("2"));
}

#[test]
fn the_tinygo_guest_finalizes_its_stream_context() {
    // Arrange
    let mut lifecycle = Lifecycle::through_stream(TINYGO);
    let (_, request) = lifecycle.request_headers(Request::default());

    // Act
    let (done, _) = lifecycle.finalize(request);

    // Assert
    assert!(done.unwrap());
    assert_eq!(
        lifecycle.guest.context_state(lifecycle.stream.unwrap()),
        None
    );
}

#[test]
fn a_refused_write_ends_the_stream_of_the_rust_sdk_guest() {
    // Arrange
    let mut lifecycle = Lifecycle::through_stream(RUST_SDK);
    let sealed = SealedRequest {
        headers: Sealed(VecHeaderMap::from(vec![(b"seed".to_vec(), b"1".to_vec())])),
    };

    // Act
    let (action, request) = lifecycle.request_headers(sealed);

    // Assert
    assert!(matches!(action, Err(Error::Trap { .. })));
    assert!(lifecycle.guest.instance().is_poisoned());
    assert_eq!(
        request.headers.0.pairs(),
        vec![(b"seed".to_vec(), b"1".to_vec())]
    );
}

#[test]
fn the_lifecycle_survives_a_vm_and_a_plugin_configuration() {
    // Arrange
    let plugin = PluginConfig::new()
        .with_name(b"add-header".to_vec())
        .with_configuration(br#"{"header":"Wasm-Context"}"#.to_vec());
    let configured = Lifecycle::configured(RUST_SDK, b"vm bytes".to_vec(), plugin);
    let mut lifecycle = Lifecycle::through_stream_of(configured);

    // Act
    let (action, request) = lifecycle.request_headers(Request::default());

    // Assert
    assert_eq!(action.unwrap(), Action::Continue);
    assert_eq!(header(&request, "Wasm-Context").as_deref(), Some("2"));
    assert_eq!(
        lifecycle
            .guest
            .plugin(lifecycle.root.unwrap())
            .unwrap()
            .name(),
        b"add-header"
    );
}
