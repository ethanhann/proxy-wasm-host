//! What a guest exports beside the ABI's own callbacks.

use crate::Error;
use crate::abi::v0_2_1::{AbiAccess, Guest, GuestError};

/// Every callback and every allocator the ABI defines starts with this.
const ABI_PREFIX: &str = "proxy_";

/// The allocator a guest may export in place of `proxy_on_memory_allocate`.
const LIBC_ALLOCATOR: &str = "malloc";

impl Guest {
    /// Whether an earlier failure unwound a guest call.
    ///
    /// A poisoned guest refuses every later call.
    /// To recover, build a new guest with
    /// [`GuestSpec`](crate::abi::v0_2_1::GuestSpec).
    /// You keep the compiled code, and you lose the context table and the
    /// queue and metric identifiers this guest obtained.
    pub fn is_poisoned(&self) -> bool {
        self.instance.is_poisoned()
    }

    /// Whether the guest can serve a request.
    ///
    /// Sometimes you keep a pool of guests and replace the ones that can no
    /// longer serve.
    /// A guest is out of service when it is poisoned and when its VM start
    /// refused, because every later callback of that guest answers
    /// [`GuestError::GuestRejected`].
    /// [`Guest::is_poisoned`] answers false for a refused VM start, so check
    /// this method in a pool.
    pub fn is_serving(&self) -> bool {
        !self.is_poisoned()
            && self
                .instance
                .state()
                .abi()
                .contexts()
                .vm_rejected()
                .is_none()
    }

    /// Refuses a poisoned guest, and poisons a guest whose last callback did
    /// not return, which is what a caught panic leaves behind.
    pub(crate) fn require_live(&mut self) -> Result<(), Error> {
        let state = self.instance.state_mut();
        if state.abi().current_callback().is_some() {
            state.poison();
        }
        if state.is_poisoned() {
            Err(Error::Poisoned)
        } else {
            Ok(())
        }
    }

    /// Whether the guest exports `name`.
    pub fn exports(&self, name: &str) -> bool {
        self.instance.has_export(name)
    }

    /// Calls an export the ABI does not name.
    ///
    /// The ABI's own callbacks run through [`Guest::enter`] and
    /// [`Guest::with`], which keep the context table and the running callback
    /// in step with the guest.
    /// This is for an export beside them, such as a reactor's initializer.
    /// The call runs with no stream state, so a host function the export
    /// calls reports the unavailable status of its family.
    ///
    /// # Errors
    ///
    /// Every failure is a [`GuestError::Runtime`].
    /// The error inside is [`Error::Config`] for a name that starts with
    /// `proxy_` and for `malloc`, because the crate calls those itself and a
    /// call from you would leave the guest and the context table disagreeing.
    /// It is [`Error::Poisoned`] after an earlier failure, which includes a
    /// callback that never returned.
    /// It is the error of the guest call otherwise.
    pub fn call_export<P: wasmtime::WasmParams, R: wasmtime::WasmResults>(
        &mut self,
        name: &str,
        params: P,
    ) -> Result<R, GuestError> {
        if name.starts_with(ABI_PREFIX) || name == LIBC_ALLOCATOR {
            return Err(GuestError::Runtime(Error::Config {
                message: format!("{name} belongs to the ABI, and the crate calls it itself"),
            }));
        }
        self.require_live()?;
        Ok(self.instance.call(name, params)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::v0_2_1::Callback;
    use crate::abi::v0_2_1::Host;
    use crate::abi::v0_2_1::test_support::{engine, services, wat_bytes};
    use crate::runtime::{Engine, Limits, Module};

    /// A guest with one export of its own, and a counter of the contexts the
    /// guest was told about.
    const ADDER: &str = r#"(module
        (memory (export "memory") 1)
        (global $created (mut i32) (i32.const 0))
        (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 1024)
        (func (export "malloc") (param i32) (result i32) i32.const 2048)
        (func (export "proxy_abi_version_0_2_1"))
        (func (export "proxy_on_context_create") (param i32 i32)
            (global.set $created (i32.add (global.get $created) (i32.const 1))))
        (func (export "created") (result i32) (global.get $created))
        (func (export "add") (param i32 i32) (result i32)
            (i32.add (local.get 0) (local.get 1))))"#;

    fn guest(engine: &Engine) -> Guest {
        let module = Module::new(engine, &wat_bytes(ADDER)).unwrap();
        Guest::new(
            &Host::new(engine).unwrap(),
            &module,
            services(),
            &Limits::default(),
        )
        .unwrap()
    }

    fn refused_name(result: Result<(), GuestError>) -> Option<String> {
        match result {
            Err(GuestError::Runtime(Error::Config { message })) => Some(message),
            _ => None,
        }
    }

    #[test]
    fn an_export_the_abi_does_not_name_is_callable() {
        // Arrange
        let engine = engine();
        let mut guest = guest(&engine);

        // Act
        let sum = guest.call_export::<(i32, i32), i32>("add", (2, 3));

        // Assert
        assert!(matches!(sum, Ok(5)));
        assert!(!guest.is_poisoned());
    }

    #[test]
    fn a_missing_export_is_reported_by_name() {
        // Arrange
        let engine = engine();
        let mut guest = guest(&engine);

        // Act
        let result = guest.call_export::<(), ()>("absent", ());

        // Assert
        assert!(
            matches!(result, Err(GuestError::Runtime(Error::MissingExport { name })) if name == "absent")
        );
        assert!(!guest.is_poisoned());
    }

    #[test]
    fn a_name_the_abi_owns_is_refused_before_the_guest_runs() {
        // Arrange
        let engine = engine();
        let mut guest = guest(&engine);

        // Act
        let refused = [
            refused_name(guest.call_export::<(i32, i32), ()>("proxy_on_context_create", (99, 0))),
            refused_name(
                guest
                    .call_export::<i32, i32>("proxy_on_memory_allocate", 8)
                    .map(drop),
            ),
            refused_name(guest.call_export::<i32, i32>("malloc", 8).map(drop)),
        ];

        // Assert
        for (message, name) in refused.iter().zip([
            "proxy_on_context_create",
            "proxy_on_memory_allocate",
            "malloc",
        ]) {
            assert!(
                message.as_deref().is_some_and(|m| m.contains(name)),
                "{name} was not refused: {message:?}"
            );
        }
        assert!(matches!(guest.call_export::<(), i32>("created", ()), Ok(0)));
        assert_eq!(
            guest.context_type(crate::abi::v0_2_1::ContextId::try_from(99).unwrap()),
            None
        );
        assert!(!guest.is_poisoned());
    }

    #[test]
    fn a_call_after_a_callback_that_never_returned_is_refused_as_poisoned() {
        // Arrange
        let engine = engine();
        let mut guest = guest(&engine);
        guest
            .instance_mut()
            .state_mut()
            .abi_mut()
            .set_current_callback(Some(Callback::Done));

        // Act
        let result = guest.call_export::<(i32, i32), i32>("add", (2, 3));

        // Assert
        assert!(matches!(result, Err(GuestError::Runtime(Error::Poisoned))));
        assert!(guest.is_poisoned());
    }

    #[test]
    fn exports_reports_what_the_module_exports() {
        // Arrange
        let engine = engine();
        let guest = guest(&engine);

        // Act
        let observed = (guest.exports("add"), guest.exports("absent"));

        // Assert
        assert_eq!(observed, (true, false));
    }

    #[test]
    fn a_trap_is_observable_through_is_poisoned() {
        // Arrange
        let engine = engine();
        let wat = r#"(module
            (memory (export "memory") 1)
            (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 1024)
            (func (export "proxy_abi_version_0_2_1"))
            (func (export "crash") unreachable))"#;
        let module = Module::new(&engine, &wat_bytes(wat)).unwrap();
        let mut guest = Guest::new(
            &Host::new(&engine).unwrap(),
            &module,
            services(),
            &Limits::default(),
        )
        .unwrap();
        let before = (guest.is_poisoned(), guest.is_serving());

        // Act
        let result = guest.call_export::<(), ()>("crash", ());

        // Assert
        assert!(matches!(
            result,
            Err(GuestError::Runtime(Error::Trap { .. }))
        ));
        assert_eq!(before, (false, true));
        assert_eq!((guest.is_poisoned(), guest.is_serving()), (true, false));
    }
}
