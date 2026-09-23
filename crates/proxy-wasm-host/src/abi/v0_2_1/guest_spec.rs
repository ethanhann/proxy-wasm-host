//! What every guest of one VM is built from.

use std::fmt;

use crate::Error;
use crate::abi::AbiVersion;
use crate::abi::v0_2_1::{Guest, GuestError, Host, VmServices};
use crate::runtime::{Limits, Module};

/// The host, the module, the services, and the limits of one VM.
///
/// Sometimes a guest traps, and you need a new guest that serves the same
/// plugin.
/// Keep a `GuestSpec` beside each guest, and call [`GuestSpec::build`] and
/// [`Guest::start`] to replace it.
/// The module keeps its compiled code, so a new guest does not compile it
/// again.
///
/// When a guest traps, or [`Guest::is_serving`] answers false, recover in four
/// steps:
///
/// 1. End the requests that wait for a callout of the old guest.
///    [`Guest::open_callouts`] names each one, and it answers on a poisoned
///    guest.
///    Cancel each gRPC stream at your side, because the crate ends none of
///    them.
/// 2. Take your request back with [`Guest::take_stream`] if a scope left it
///    on the old guest.
/// 3. Build a new guest and start each of its roots.
/// 4. Declare the family of each new stream context again with
///    [`Guest::expect_stream_kind`], because the new guest holds no record of
///    the old one.
///
/// The new guest has its own [`GuestId`](crate::abi::v0_2_1::GuestId), and its
/// contexts start at one.
/// Drop every identifier of the old guest.
/// The services are cloned, so the new guest shares the log sink, the
/// callouts service, and the shared services of the old one.
/// You keep the plugin of each root beside the `GuestSpec`, because step 3
/// starts each root with its plugin again.
/// [`Guest::is_serving`] tells a pool when a guest needs this recovery.
///
/// For example, a guest is built and started again after a failure:
///
/// ```
/// use std::sync::Arc;
/// use proxy_wasm_host::abi::v0_2_1::types::LogLevel;
/// use proxy_wasm_host::abi::v0_2_1::{
///     GuestError, GuestSpec, Host, LogContext, LogSink, PluginConfig, Started, VmServices,
/// };
/// use proxy_wasm_host::{Engine, Limits, Module};
///
/// struct Discard;
/// impl LogSink for Discard {
///     fn log(&self, _: LogContext<'_>, _: LogLevel, _: &[u8]) {}
/// }
///
/// # fn main() -> Result<(), GuestError> {
/// let wat = r#"(module
///     (memory (export "memory") 1)
///     (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 1024)
///     (func (export "proxy_abi_version_0_2_1")))"#;
/// let engine = Engine::new()?;
/// let module = Module::new(&engine, &wat::parse_str(wat).unwrap())?;
/// let services = VmServices::new(Arc::new(Discard));
/// let spec = GuestSpec::new(&Host::new(&engine)?, &module, services, &Limits::default())?;
/// let plugin = PluginConfig::new().with_root_id(*b"http");
///
/// let mut guest = spec.build()?;
/// assert!(matches!(guest.start(plugin.clone())?, Started::Serving(_)));
///
/// // A callback traps here, and the guest is poisoned.
/// if !guest.is_serving() {
///     for _callout in guest.open_callouts() {
///         // End the request that waits for this callout.
///     }
///     guest = spec.build()?;
///     match guest.start(plugin)? {
///         Started::Serving(_root) => {}
///         Started::Refused { callback, .. } => eprintln!("{callback} refused the plugin"),
///     }
/// }
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct GuestSpec {
    host: Host,
    module: Module,
    services: VmServices,
    limits: Limits,
}

impl fmt::Debug for GuestSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GuestSpec")
            .field("services", &self.services)
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

impl GuestSpec {
    /// Checks the module once, and holds clones of the four values that
    /// [`Guest::new`] takes.
    ///
    /// # Errors
    ///
    /// Returns [`GuestError::UnsupportedAbi`] when the module exports no
    /// accepted `proxy_abi_version_*` marker.
    /// Returns [`GuestError::Runtime`] with
    /// [`Error::Config`](crate::Error::Config) when the module was compiled on
    /// another engine than the one of `host`, because no guest of that pair
    /// can be built.
    pub fn new(
        host: &Host,
        module: &Module,
        services: VmServices,
        limits: &Limits,
    ) -> Result<Self, GuestError> {
        AbiVersion::detect(module.abi_exports())?;
        if !wasmtime::Engine::same(host.engine().wasmtime(), module.wasmtime().engine()) {
            return Err(GuestError::Runtime(Error::Config {
                message: "the module was compiled on another engine than the host".to_owned(),
            }));
        }
        Ok(Self {
            host: host.clone(),
            module: module.clone(),
            services,
            limits: limits.clone(),
        })
    }

    /// Builds a new guest with a clone of the services.
    ///
    /// No callback runs here.
    /// The guest runs its WASI start function, which the Rust SDK uses to
    /// install its root factory, and then [`Guest::start`] runs the
    /// callbacks of a root.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`Guest::new`].
    pub fn build(&self) -> Result<Guest, GuestError> {
        Guest::new(
            &self.host,
            &self.module,
            self.services.clone(),
            &self.limits,
        )
    }

    /// The services that each guest [`GuestSpec::build`] gives receives a
    /// clone of.
    pub fn services(&self) -> &VmServices {
        &self.services
    }

    /// The services, for a change that each later guest receives.
    ///
    /// A change through [`Guest::services_mut`] reaches one guest.
    /// If you change the log level of a running guest and want the guest you
    /// build after a trap to keep it, make the same change here.
    pub fn services_mut(&mut self) -> &mut VmServices {
        &mut self.services
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::v0_2_1::test_support::{MINIMAL_GUEST, engine, services, wat_bytes};
    use crate::abi::v0_2_1::types::LogLevel;

    const MARKED: &str = r#"(module
        (memory (export "memory") 1)
        (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 1024)
        (func (export "proxy_abi_version_0_2_1")))"#;

    fn spec(wat: &str, services: VmServices) -> GuestSpec {
        limited(wat, services, &Limits::default())
    }

    fn limited(wat: &str, services: VmServices, limits: &Limits) -> GuestSpec {
        let engine = engine();
        let module = Module::new(&engine, &wat_bytes(wat)).unwrap();
        GuestSpec::new(&Host::new(&engine).unwrap(), &module, services, limits).unwrap()
    }

    fn assert_send_sync<T: Send + Sync>() {}

    const _: () = {
        let _ = assert_send_sync::<GuestSpec>;
    };

    #[test]
    fn build_gives_a_guest_of_the_module_and_the_services() {
        // Arrange
        let spec = spec(MARKED, services().with_vm_id(*b"edge"));

        // Act
        let guest = spec.build();

        // Assert
        let guest = guest.unwrap();
        assert_eq!(guest.abi(), AbiVersion::V0_2_1);
        assert_eq!(guest.services().vm_id(), b"edge");
        assert_eq!(spec.services().vm_id(), b"edge");
        assert!(!guest.is_poisoned());
    }

    #[test]
    fn two_guests_of_one_spec_have_different_identities() {
        // Arrange
        let spec = spec(MARKED, services());
        let first = spec.build().unwrap();

        // Act
        let second = spec.build();

        // Assert
        assert_ne!(first.id(), second.unwrap().id());
    }

    #[test]
    fn a_module_with_no_abi_marker_is_refused_by_new() {
        // Arrange
        let engine = engine();
        let module = Module::new(&engine, &wat_bytes(MINIMAL_GUEST)).unwrap();
        let host = Host::new(&engine).unwrap();
        let limits = Limits::default();

        // Act
        let spec = GuestSpec::new(&host, &module, services(), &limits);

        // Assert
        assert!(matches!(spec, Err(GuestError::UnsupportedAbi(_))));
    }

    #[test]
    fn a_module_of_another_engine_is_refused_by_new() {
        // Arrange
        let module = Module::new(&engine(), &wat_bytes(MARKED)).unwrap();
        let host = Host::new(&engine()).unwrap();
        let limits = Limits::default();

        // Act
        let spec = GuestSpec::new(&host, &module, services(), &limits);

        // Assert
        assert!(matches!(
            spec,
            Err(GuestError::Runtime(Error::Config { message }))
                if message == "the module was compiled on another engine than the host"
        ));
    }

    #[test]
    fn a_change_through_services_mut_reaches_the_next_guest() {
        // Arrange
        let mut spec = spec(MARKED, services());
        spec.services_mut().set_log_level(LogLevel::Critical);

        // Act
        let guest = spec.build();

        // Assert
        assert_eq!(guest.unwrap().services().log_level(), LogLevel::Critical);
    }

    #[test]
    fn build_gives_the_guest_the_limits_it_holds() {
        // Arrange
        let limits = Limits::default().with_memory_bytes(1024);
        let spec = limited(MARKED, services(), &limits);

        // Act
        let guest = spec.build();

        // Assert
        assert!(matches!(
            guest,
            Err(GuestError::Runtime(Error::Instantiate { source }))
                if source.to_string().contains("exceeds memory limits")
        ));
    }

    #[test]
    fn the_debug_form_names_the_services_and_the_limits() {
        // Arrange
        let spec = spec(MARKED, services());

        // Act
        let text = format!("{spec:?}");

        // Assert
        assert!(
            text.starts_with("GuestSpec { services: VmServices"),
            "{text}"
        );
        assert!(text.contains("limits: Limits"), "{text}");
        assert!(text.ends_with(", .. }"), "{text}");
    }
}
