//! The `exercise-all` guest with the recorder as every service.

use std::borrow::Cow;
use std::sync::{Arc, OnceLock};

use proxy_wasm_host::abi::v0_2_1::types::LogLevel;
use proxy_wasm_host::abi::v0_2_1::{
    Clock, ContextId, Guest, GuestError, GuestSpec, Host, PluginConfig, Started, StreamKind,
    VmServices,
};
use proxy_wasm_host::{Engine, Limits, Module};

use super::recorder::{Recorder, StreamDouble};

pub const EXERCISE_ALL: &[u8] = include_bytes!("../fixtures/exercise-all.wasm");

/// One second after the epoch on the wall clock.
pub const NOW: u64 = 1_000_000_000;

/// A monotonic reading that differs from the wall clock, so a test sees which
/// clock the guest read.
pub const MONOTONIC: u64 = 7;

/// A clock that answers the same reading every time, so a test that reads a
/// time can assert what the guest wrote.
pub struct FixedClock;

impl Clock for FixedClock {
    fn realtime_nanos(&self) -> u64 {
        NOW
    }

    fn monotonic_nanos(&self) -> u64 {
        MONOTONIC
    }
}

/// The host and the module, compiled once for every test of one file.
fn compiled() -> &'static (Host, Module) {
    static COMPILED: OnceLock<(Host, Module)> = OnceLock::new();
    COMPILED.get_or_init(|| {
        let engine = Engine::new().unwrap();
        let module = Module::new(&engine, EXERCISE_ALL).unwrap();
        (Host::new(&engine).unwrap(), module)
    })
}

/// The `GuestSpec` of the guest and the recorder that serves it.
pub struct Exercise {
    pub spec: GuestSpec,
    pub recorder: Recorder,
}

impl Exercise {
    /// A `GuestSpec` whose VM configuration is `vm_configuration`.
    pub fn with_vm_configuration(vm_configuration: &str) -> Self {
        let recorder = Recorder::default();
        let services = VmServices::new(Arc::new(recorder.clone()))
            .with_clock(Arc::new(FixedClock))
            .with_log_level(LogLevel::Debug)
            .with_vm_id(*b"exercise_vm")
            .with_vm_configuration(vm_configuration.as_bytes().to_vec())
            .with_environment(vec![(b"EXERCISE".to_vec(), b"on".to_vec())])
            .with_shared(Arc::new(recorder.shared()))
            .with_callouts(Arc::new(recorder.clone()));
        let (host, module) = compiled();
        let spec = GuestSpec::new(host, module, services, &Limits::default()).unwrap();
        Self { spec, recorder }
    }

    /// A `GuestSpec` with the VM configuration `exercise`.
    pub fn new() -> Self {
        Self::with_vm_configuration("exercise")
    }

    /// A guest with one root of `plugin` started, and the root.
    pub fn started(&self, plugin: PluginConfig) -> (Guest, ContextId) {
        let mut guest = self.spec.build().unwrap();
        match guest.start(plugin).unwrap() {
            Started::Serving(root) => (guest, root),
            refused @ Started::Refused { .. } => panic!("the root was refused: {refused:?}"),
        }
    }
}

/// The plugin of the HTTP root, with `configuration`.
pub fn http_plugin(configuration: &str) -> PluginConfig {
    PluginConfig::new()
        .with_name(*b"exercise")
        .with_root_id(*b"http")
        .with_configuration(configuration.as_bytes().to_vec())
}

/// The plugin of the TCP root.
pub fn tcp_plugin() -> PluginConfig {
    PluginConfig::new()
        .with_name(*b"exercise")
        .with_root_id(*b"tcp")
        .with_configuration(b"tcp".to_vec())
}

/// One header pair.
pub fn pair(key: &'static str, value: &'static str) -> (Cow<'static, [u8]>, Cow<'static, [u8]>) {
    (
        Cow::Borrowed(key.as_bytes()),
        Cow::Borrowed(value.as_bytes()),
    )
}

/// A stream context of `kind` under `root`, and the state lent to create it.
pub fn stream_of(
    guest: &mut Guest,
    root: ContextId,
    kind: StreamKind,
    state: StreamDouble,
) -> (ContextId, StreamDouble) {
    let (created, state) = guest.with(state, |scope| {
        let stream = scope.on_context_create(Some(root))?;
        scope.expect_stream_kind(stream, kind)?;
        Ok::<_, GuestError>(stream)
    });
    (created.unwrap(), state)
}
