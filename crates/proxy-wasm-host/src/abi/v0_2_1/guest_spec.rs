//! What every guest of one VM is built from.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

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
///    [`Guest::open_callouts`] lists each one, and it answers on a poisoned
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
    counters: Arc<BuildCounters>,
}

/// What one spec counts, shared by every clone of it.
#[derive(Debug, Default)]
pub(crate) struct BuildCounters {
    builds: AtomicU64,
    poisoned_guests: AtomicU64,
}

fn raise(counter: &AtomicU64) {
    let _ = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
        Some(count.saturating_add(1))
    });
}

impl BuildCounters {
    fn record_build(&self) {
        raise(&self.builds);
    }

    /// Records that a poisoned guest of this spec was dropped.
    pub(crate) fn record_poison(&self) {
        raise(&self.poisoned_guests);
    }
}

impl fmt::Debug for GuestSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GuestSpec")
            .field("services", &self.services)
            .field("limits", &self.limits)
            .field("builds", &self.builds())
            .field("poisoned_guests", &self.poisoned_guests())
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
            counters: Arc::new(BuildCounters::default()),
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
        let mut guest = Guest::new(
            &self.host,
            &self.module,
            self.services.clone(),
            &self.limits,
        )?;
        self.counters.record_build();
        guest.count_on(Arc::clone(&self.counters));
        Ok(guest)
    }

    /// How many guests this spec built.
    ///
    /// A clone of the spec shares the count, and a guest that
    /// [`Guest::new`] built alone is not in it.
    /// The number stops at [`u64::MAX`].
    pub fn builds(&self) -> u64 {
        self.counters.builds.load(Ordering::Relaxed)
    }

    /// How many guests this spec built were poisoned and then dropped.
    ///
    /// The number rises when you drop a poisoned guest, whether you build
    /// its replacement before or after, and it counts each guest of a pool.
    /// A poisoned guest that you keep is not in it until you drop it.
    /// A plugin that traps on every request drives it up at the rate of the
    /// traffic, and a plugin that works leaves it where it was.
    ///
    /// The ABI document asks a host to limit the rate of these failures.
    /// Keep the value you read, compare it after each rebuild, and stop
    /// serving with the plugin when it rises faster than you accept.
    /// The crate applies no window of its own, because the rate you accept
    /// belongs to your deployment.
    /// The number stops at [`u64::MAX`].
    pub fn poisoned_guests(&self) -> u64 {
        self.counters.poisoned_guests.load(Ordering::Relaxed)
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
    use crate::abi::v0_2_1::PluginConfig;
    use crate::abi::v0_2_1::test_support::{MINIMAL_GUEST, engine, services, wat_bytes};
    use crate::abi::v0_2_1::types::LogLevel;

    const TRAPS: &str = r#"(module
        (memory (export "memory") 1)
        (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 1024)
        (func (export "proxy_abi_version_0_2_1"))
        (func (export "proxy_on_context_create") (param i32 i32) unreachable))"#;

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

    /// A guest of `TRAPS` that has trapped in its start.
    fn poisoned(spec: &GuestSpec) -> Guest {
        let mut guest = spec.build().unwrap();
        assert!(guest.start(PluginConfig::new()).is_err());
        assert!(guest.is_poisoned());
        guest
    }

    #[test]
    fn a_spec_counts_the_guests_it_built() {
        // Arrange
        let spec = spec(MARKED, services());
        let first = spec.build().unwrap();

        // Act
        let second = spec.build().unwrap();

        // Assert
        assert_eq!(spec.builds(), 2);
        assert_eq!(spec.poisoned_guests(), 0);
        assert_ne!(first.id(), second.id());
    }

    #[test]
    fn a_clone_of_a_spec_counts_into_the_same_numbers() {
        // Arrange
        let spec = spec(TRAPS, services());
        let clone = spec.clone();
        let _first = spec.build().unwrap();
        let guest = poisoned(&clone);

        // Act
        drop(guest);

        // Assert
        assert_eq!(spec.builds(), 2);
        assert_eq!(clone.builds(), 2);
        assert_eq!(spec.poisoned_guests(), 1);
    }

    #[test]
    fn a_poisoned_guest_counts_when_it_is_dropped() {
        // Arrange
        let spec = spec(TRAPS, services());
        let guest = poisoned(&spec);
        assert_eq!(spec.poisoned_guests(), 0, "a live guest is not counted");

        // Act
        drop(guest);

        // Assert
        assert_eq!(spec.poisoned_guests(), 1);
    }

    #[test]
    fn a_poisoned_guest_counts_when_its_replacement_was_built_first() {
        // Arrange
        let spec = spec(TRAPS, services());
        let mut guest = poisoned(&spec);
        assert_eq!(spec.poisoned_guests(), 0, "{:?} is still alive", guest.id());

        // Act
        guest = spec.build().unwrap();

        // Assert
        assert_eq!(spec.poisoned_guests(), 1);
        assert!(!guest.is_poisoned());
    }

    #[test]
    fn every_poisoned_guest_of_a_pool_counts() {
        // Arrange
        let spec = spec(TRAPS, services());
        let pool = [poisoned(&spec), poisoned(&spec), poisoned(&spec)];

        // Act
        drop(pool);

        // Assert
        assert_eq!(spec.poisoned_guests(), 3);
    }

    #[test]
    fn a_guest_that_was_never_poisoned_is_not_counted() {
        // Arrange
        let spec = spec(MARKED, services());
        let guest = spec.build().unwrap();

        // Act
        drop(guest);

        // Assert
        assert_eq!(spec.builds(), 1);
        assert_eq!(spec.poisoned_guests(), 0);
    }
}
