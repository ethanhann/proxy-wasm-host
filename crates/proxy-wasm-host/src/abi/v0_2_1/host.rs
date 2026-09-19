//! An engine with the host functions of this ABI version linked.

use std::fmt;
use std::sync::Arc;

use wasmtime::Linker;

use crate::Error;
use crate::abi::v0_2_1::host_functions;
use crate::runtime::{Engine, HostState};

/// An engine with the WASI functions and the host functions of ABI v0.2.1
/// linked.
///
/// Sometimes one process runs many guests, on many threads.
/// You build one `Host` from your [`Engine`] and give it to every
/// [`Guest::new`](crate::abi::v0_2_1::Guest::new).
/// A clone is two reference count increments, so each thread can keep its
/// own.
///
/// For example:
///
/// ```
/// use proxy_wasm_host::Engine;
/// use proxy_wasm_host::abi::v0_2_1::Host;
///
/// # fn main() -> Result<(), proxy_wasm_host::Error> {
/// let engine = Engine::new()?;
/// let host = Host::new(&engine)?;
/// assert_eq!(host.engine().epoch_period(), engine.epoch_period());
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct Host {
    engine: Engine,
    linker: Arc<Linker<HostState>>,
}

impl fmt::Debug for Host {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Host").finish_non_exhaustive()
    }
}

impl Host {
    /// Links the WASI functions and the host functions of ABI v0.2.1 on
    /// `engine`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Instantiate`] when wasmtime refuses a definition.
    pub fn new(engine: &Engine) -> Result<Self, Error> {
        let mut linker = Linker::new(engine.wasmtime());
        host_functions::register(&mut linker)?;
        Ok(Self {
            engine: engine.clone(),
            linker: Arc::new(linker),
        })
    }

    /// The engine this value was built on.
    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    pub(crate) fn linker(&self) -> &Linker<HostState> {
        &self.linker
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::v0_2_1::Guest;
    use crate::abi::v0_2_1::host_functions::table::HOST_FUNCTIONS;
    use crate::abi::v0_2_1::test_support::{engine, instance, services, wat_bytes};
    use crate::runtime::{Limits, Module};

    #[test]
    fn a_host_links_every_function_of_the_table() {
        // Arrange
        let engine = engine();
        let host = Host::new(&engine).unwrap();
        let mut store = wasmtime::Store::new(
            engine.wasmtime(),
            HostState::new(crate::abi::state(services())),
        );

        // Act
        let missing: Vec<&str> = HOST_FUNCTIONS
            .iter()
            .map(|function| function.name)
            .filter(|name| host.linker().get(&mut store, "env", name).is_err())
            .collect();

        // Assert
        assert_eq!(HOST_FUNCTIONS.len(), 39);
        assert_eq!(missing, Vec::<&str>::new());
    }

    #[test]
    fn a_guest_that_imports_abi_host_functions_instantiates() {
        // Arrange
        let engine = engine();
        let wat = r#"(module
            (import "env" "proxy_log" (func (param i32 i32 i32) (result i32)))
            (import "env" "proxy_call_foreign_function" (func (param i32 i32 i32 i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 1024))"#;

        // Act
        let result = instance(&engine, wat);

        // Assert
        assert!(result.is_ok(), "{:?}", result.err());
    }

    #[test]
    fn a_wasi_function_the_host_does_not_link_does_not_instantiate() {
        // Arrange
        let engine = engine();
        let wat = r#"(module
            (import "wasi_snapshot_preview1" "fd_read" (func (param i32 i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 0))"#;

        // Act
        let result = instance(&engine, wat);

        // Assert
        assert!(matches!(result, Err(Error::Instantiate { .. })));
    }

    #[test]
    fn a_clone_of_a_host_builds_a_guest_on_the_same_engine() {
        // Arrange
        let engine = engine();
        let host = Host::new(&engine).unwrap();
        let wat = r#"(module
            (memory (export "memory") 1)
            (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 1024)
            (func (export "proxy_abi_version_0_2_1")))"#;
        let module = Module::new(&engine, &wat_bytes(wat)).unwrap();
        let clone = host.clone();
        drop(host);

        // Act
        let guest = Guest::new(&clone, &module, services(), &Limits::default());

        // Assert
        assert!(guest.is_ok(), "{:?}", guest.err());
        engine.increment_epoch();
        assert_eq!(clone.engine().ticks(), engine.ticks());
        assert_eq!(format!("{clone:?}"), "Host { .. }");
    }
}
