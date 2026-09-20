//! The services and the VM scoped inputs an embedder gives to a guest.

use std::sync::{Arc, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::abi::v0_2_1::callout_service::NoCallouts;
use crate::abi::v0_2_1::types::LogLevel;
use crate::abi::v0_2_1::{Callouts, InMemoryStore, SharedServices};

const DEFAULT_MAX_OPEN_CALLOUTS: usize = 1024;

/// Where guest log output goes.
///
/// The WASI `fd_write` function and the `proxy_log` host function both call
/// this.
/// One sink usually serves a whole process, so [`VmServices`] holds it in an
/// `Arc` and calls it through a shared reference.
pub trait LogSink: Send + Sync {
    /// Records one message at one level.
    fn log(&self, level: LogLevel, message: &[u8]);
}

/// The time source for the WASI `clock_time_get` function and for
/// `proxy_get_current_time_nanoseconds`.
///
/// A host may return approximate or frozen time.
/// A test can therefore install a clock with fixed values.
pub trait Clock: Send + Sync {
    /// Nanoseconds since the Unix epoch.
    fn realtime_nanos(&self) -> u64;
    /// Nanoseconds since an origin that never moves while the process runs.
    fn monotonic_nanos(&self) -> u64;
}

/// The clock that reads the operating system.
///
/// The monotonic origin is one `Instant` for the whole process.
/// Every instance therefore reports comparable monotonic values.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

fn monotonic_origin() -> Instant {
    static ORIGIN: OnceLock<Instant> = OnceLock::new();
    *ORIGIN.get_or_init(Instant::now)
}

fn nanos(duration: std::time::Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

impl Clock for SystemClock {
    fn realtime_nanos(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, nanos)
    }

    fn monotonic_nanos(&self) -> u64 {
        nanos(monotonic_origin().elapsed())
    }
}

/// What the host performs for a guest, and the VM scoped inputs the guest
/// reads.
///
/// The services are the log sink and the clock.
/// The inputs are the environment variables, the VM id, the VM configuration,
/// and the log level the guest can ask for.
/// Build one per instance and pass it to [`Guest::new`](crate::abi::v0_2_1::Guest::new).
/// You can change it between calls through
/// [`Guest::services_mut`](crate::abi::v0_2_1::Guest::services_mut).
///
/// Clone it to build a second instance against the same log sink, clock, and
/// shared state.
///
/// The inputs of one plugin live elsewhere.
/// The plugin name, the plugin root id, and the plugin configuration reach the
/// crate through [`PluginConfig`](crate::abi::v0_2_1::PluginConfig), because the ABI reads
/// them for one root context.
#[derive(Clone)]
pub struct VmServices {
    log: Arc<dyn LogSink>,
    clock: Arc<dyn Clock>,
    environment: Vec<(Vec<u8>, Vec<u8>)>,
    log_level: LogLevel,
    vm_id: Vec<u8>,
    vm_configuration: Vec<u8>,
    shared: Arc<dyn SharedServices>,
    callouts: Arc<dyn Callouts>,
    max_open_callouts: usize,
}

impl std::fmt::Debug for VmServices {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VmServices")
            .field("log_level", &self.log_level)
            .field("environment_variables", &self.environment.len())
            .field("vm_id", &String::from_utf8_lossy(&self.vm_id))
            .field("vm_configuration_bytes", &self.vm_configuration.len())
            .finish_non_exhaustive()
    }
}

impl VmServices {
    /// Services that log to `log`, read [`SystemClock`], report
    /// [`LogLevel::Info`], hold a [`InMemoryStore`], and have no
    /// environment, no VM id, and no VM configuration.
    ///
    /// The store this installs is private to the value you get back, so two
    /// instances built with two of these share no queue, no key, and no
    /// metric.
    /// Clone one value, or pass one store to both through
    /// [`VmServices::with_shared`], when you want them shared.
    ///
    /// The VM id is empty, and the VM id is what separates one plugin's
    /// shared data and metrics from another's, so set one per plugin.
    pub fn new(log: Arc<dyn LogSink>) -> Self {
        Self {
            log,
            clock: Arc::new(SystemClock),
            environment: Vec::new(),
            log_level: LogLevel::Info,
            vm_id: Vec::new(),
            vm_configuration: Vec::new(),
            shared: Arc::new(InMemoryStore::new()),
            callouts: Arc::new(NoCallouts),
            max_open_callouts: DEFAULT_MAX_OPEN_CALLOUTS,
        }
    }

    /// Replaces the clock.
    #[must_use]
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    /// Sets the environment variables that the guest sees through WASI.
    ///
    /// The ABI document says these must be configured per guest.
    /// They are never read from the process environment.
    #[must_use]
    pub fn with_environment(mut self, variables: Vec<(Vec<u8>, Vec<u8>)>) -> Self {
        self.environment = variables;
        self
    }

    /// Sets the level that `proxy_get_log_level` reports.
    ///
    /// A guest reads the level to skip log lines it would build and you would
    /// discard.
    /// The crate does not filter on it, so your sink still receives every
    /// message, and you filter there if you want to.
    /// The ABI has no way to tell a running guest that the level changed, so
    /// a guest that read one level keeps it until it asks again.
    #[must_use]
    pub fn with_log_level(mut self, level: LogLevel) -> Self {
        self.log_level = level;
        self
    }

    /// Sets the VM id.
    ///
    /// A guest names it when it resolves a shared queue that another VM
    /// registered, and it also separates the shared data and the metrics of
    /// this plugin from those of every other plugin that holds the same
    /// store.
    /// Two plugins you give one VM id share their keys and their metrics.
    /// An empty VM id, which is the default, puts every plugin of that store
    /// in one namespace.
    #[must_use]
    pub fn with_vm_id(mut self, vm_id: impl Into<Vec<u8>>) -> Self {
        self.vm_id = vm_id.into();
        self
    }

    /// Sets the bytes the guest reads from the `VM_CONFIGURATION` buffer.
    ///
    /// `CallScope::on_vm_start` reports the length of these bytes to the
    /// guest.
    /// If you replace them after the guest started, the guest reads bytes
    /// whose length it was never told, so that is yours to manage.
    #[must_use]
    pub fn with_vm_configuration(mut self, configuration: impl Into<Vec<u8>>) -> Self {
        self.vm_configuration = configuration.into();
        self
    }

    /// Replaces the shared data, the shared queues, and the metrics.
    ///
    /// An instance reads this value, so replacing it between calls through
    /// [`Guest::services_mut`](crate::abi::v0_2_1::Guest::services_mut) changes the store the guest
    /// reaches.
    /// The queue and metric identifiers the guest obtained belong to the
    /// store that issued them, so the crate drops them when the store
    /// changes, and the guest registers or resolves again.
    ///
    /// Several instances that hold the same value share that state, which is
    /// what lets one VM resolve a queue another registered.
    /// The default is an [`Arc`] of [`InMemoryStore`], which serves one
    /// process.
    #[must_use]
    pub fn with_shared(mut self, shared: Arc<dyn SharedServices>) -> Self {
        self.shared = shared;
        self
    }

    /// Sets the service that receives the callouts of the guest.
    ///
    /// The default refuses every callout, so a guest that calls
    /// `proxy_http_call` or a gRPC function gets `INTERNAL_FAILURE`.
    /// One service can serve several guests, and the callout identifiers of
    /// each guest start at one, so key your own record by
    /// [`Invocation::guest`](crate::abi::v0_2_1::Invocation) and the
    /// callout.
    #[must_use]
    pub fn with_callouts(mut self, callouts: Arc<dyn Callouts>) -> Self {
        self.callouts = callouts;
        self
    }

    /// Sets how many callouts the guest may have open at one time.
    ///
    /// The default is 1024.
    /// A guest at the maximum gets `INTERNAL_FAILURE` for a new callout.
    /// The crate reports that refusal through `tracing` at the warn level.
    /// The crate reads the value at each new callout.
    /// A value below the number of open callouts keeps them and refuses a
    /// new one.
    #[must_use]
    pub fn with_max_open_callouts(mut self, maximum: usize) -> Self {
        self.max_open_callouts = maximum;
        self
    }

    /// The service that receives the callouts of the guest.
    ///
    /// The `Arc` is returned rather than the value behind it, because you
    /// clone it to build a second instance with the same service.
    /// The trait is not downcastable, so keep your own `Arc` if you want
    /// your concrete type back.
    /// A call you make on the service yourself opens no callout, because
    /// only a callout function of the guest enters one in the record of the
    /// guest.
    pub fn callouts(&self) -> &Arc<dyn Callouts> {
        &self.callouts
    }

    /// How many callouts the guest may have open at one time.
    pub fn max_open_callouts(&self) -> usize {
        self.max_open_callouts
    }

    /// The shared data, the shared queues, and the metrics.
    ///
    /// The `Arc` is returned rather than the value behind it, because you
    /// clone it to build a second instance against the same state.
    /// The trait is not downcastable, so keep your own `Arc` if you want
    /// your concrete type back.
    pub fn shared(&self) -> &Arc<dyn SharedServices> {
        &self.shared
    }

    /// The log sink.
    pub fn log(&self) -> &dyn LogSink {
        self.log.as_ref()
    }

    /// The clock.
    pub fn clock(&self) -> &dyn Clock {
        self.clock.as_ref()
    }

    /// The environment variables, in the order given.
    pub fn environment(&self) -> &[(Vec<u8>, Vec<u8>)] {
        &self.environment
    }

    /// The level that `proxy_get_log_level` reports.
    pub fn log_level(&self) -> LogLevel {
        self.log_level
    }

    /// Changes the level that `proxy_get_log_level` reports.
    ///
    /// The level is advice for the guest, and your sink still receives every
    /// message whatever the level says.
    /// The ABI has no way to tell a running guest that the level changed, so
    /// a guest that read the old level keeps it until it asks again.
    pub fn set_log_level(&mut self, level: LogLevel) {
        self.log_level = level;
    }

    /// The VM id.
    pub fn vm_id(&self) -> &[u8] {
        &self.vm_id
    }

    /// The bytes of the `VM_CONFIGURATION` buffer.
    pub fn vm_configuration(&self) -> &[u8] {
        &self.vm_configuration
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::v0_2_1::test_support::RecordingSink;

    fn services() -> VmServices {
        VmServices::new(Arc::new(RecordingSink::default()))
    }

    #[test]
    fn system_clock_is_monotonic_and_shares_its_origin() {
        // Arrange
        let first = SystemClock;
        let second = SystemClock;

        // Act
        let readings = [
            first.monotonic_nanos(),
            second.monotonic_nanos(),
            first.monotonic_nanos(),
        ];

        // Assert
        assert!(readings[0] <= readings[1]);
        assert!(readings[1] <= readings[2]);
        assert!(readings[2] - readings[0] < 1_000_000_000);
    }

    #[test]
    fn system_clock_real_time_is_after_2020() {
        // Arrange
        let clock = SystemClock;
        let year_2020_nanos = 1_577_836_800_u64 * 1_000_000_000;

        // Act
        let now = clock.realtime_nanos();

        // Assert
        assert!(now > year_2020_nanos);
    }

    #[test]
    fn with_environment_keeps_the_order() {
        // Arrange
        let variables = vec![
            (b"B".to_vec(), b"2".to_vec()),
            (b"A".to_vec(), b"1".to_vec()),
        ];

        // Act
        let services = services().with_environment(variables.clone());

        // Assert
        assert_eq!(services.environment(), variables.as_slice());
    }

    #[test]
    fn new_services_report_the_defaults() {
        // Arrange
        let log = Arc::new(RecordingSink::default());

        // Act
        let services = VmServices::new(log);

        // Assert
        assert_eq!(services.log_level(), LogLevel::Info);
        assert!(services.vm_id().is_empty());
        assert!(services.vm_configuration().is_empty());
        assert!(services.environment().is_empty());
    }

    #[test]
    fn with_log_level_and_set_log_level_both_change_the_level() {
        // Arrange
        let mut services = services().with_log_level(LogLevel::Warn);
        let built = services.log_level();

        // Act
        services.set_log_level(LogLevel::Trace);

        // Assert
        assert_eq!(built, LogLevel::Warn);
        assert_eq!(services.log_level(), LogLevel::Trace);
    }

    #[test]
    fn the_vm_id_leaves_the_configuration_empty() {
        // Arrange
        let base = services();

        // Act
        let services = base.with_vm_id(*b"vm-1");

        // Assert
        assert_eq!(services.vm_id(), b"vm-1");
        assert!(services.vm_configuration().is_empty());
    }

    #[test]
    fn the_vm_configuration_leaves_the_vm_id_empty() {
        // Arrange
        let base = services();

        // Act
        let services = base.with_vm_configuration(*b"{}");

        // Assert
        assert_eq!(services.vm_configuration(), b"{}");
        assert!(services.vm_id().is_empty());
    }

    #[test]
    fn the_shared_services_default_to_the_in_memory_one() {
        // Arrange
        let call = crate::abi::v0_2_1::Invocation::new(
            crate::abi::v0_2_1::GuestId::next(),
            crate::abi::v0_2_1::ContextId::try_from(1).unwrap(),
        );

        // Act
        let services = services();

        // Assert
        assert_eq!(
            services
                .shared()
                .set_shared_data(call, b"vm", b"k", b"v", None),
            Ok(())
        );
        assert_eq!(
            services
                .shared()
                .get_shared_data(call, b"vm", b"k")
                .unwrap()
                .bytes,
            b"v"
        );
    }

    #[test]
    fn with_shared_replaces_the_default() {
        // Arrange
        let mine: Arc<dyn SharedServices> = Arc::new(InMemoryStore::new());
        let call = crate::abi::v0_2_1::Invocation::new(
            crate::abi::v0_2_1::GuestId::next(),
            crate::abi::v0_2_1::ContextId::try_from(1).unwrap(),
        );
        mine.set_shared_data(call, b"vm", b"k", b"mine", None)
            .unwrap();

        // Act
        let services = services().with_shared(Arc::clone(&mine));

        // Assert
        assert_eq!(
            services
                .shared()
                .get_shared_data(call, b"vm", b"k")
                .unwrap()
                .bytes,
            b"mine"
        );
    }

    #[test]
    fn the_debug_form_names_the_level_and_the_sizes() {
        // Arrange
        let services = services()
            .with_vm_id(*b"vm-1")
            .with_vm_configuration(*b"{}");

        // Act
        let shown = format!("{services:?}");

        // Assert
        assert!(shown.contains("log_level: Info"));
        assert!(shown.contains("vm_id: \"vm-1\""));
        assert!(shown.contains("vm_configuration_bytes: 2"));
    }
}
