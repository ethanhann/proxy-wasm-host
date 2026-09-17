//! A compiled guest module.

use std::collections::BTreeSet;
use std::sync::Arc;

use crate::Error;
use crate::abi::AbiVersion;
use crate::runtime::Engine;

const ABI_PREFIX: &str = "proxy_abi_version_";

/// A compiled module and its export names.
///
/// The module and its export names are reference counted, so a clone is two
/// reference count increments.
/// Compiling does not check the ABI version.
/// Call [`Module::abi`] when you load a guest, so a guest with an unsupported
/// version is rejected by name.
#[derive(Clone)]
pub struct Module {
    inner: wasmtime::Module,
    exports: Arc<Exports>,
}

struct Exports {
    names: BTreeSet<String>,
    abi: Vec<String>,
}

impl Module {
    /// Compiles `bytes` for `engine`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Compile`] when the bytes are not a valid module.
    pub fn new(engine: &Engine, bytes: &[u8]) -> Result<Self, Error> {
        let inner =
            wasmtime::Module::new(engine.wasmtime(), bytes).map_err(|source| Error::Compile {
                source: source.into(),
            })?;
        let names: BTreeSet<String> = inner
            .exports()
            .map(|export| export.name().to_owned())
            .collect();
        let abi = names
            .iter()
            .filter(|name| name.starts_with(ABI_PREFIX))
            .cloned()
            .collect();
        Ok(Self {
            inner,
            exports: Arc::new(Exports { names, abi }),
        })
    }

    /// The newest ABI version the module advertises.
    ///
    /// # Errors
    ///
    /// Returns [`Error::UnsupportedAbi`] with the advertised names when none
    /// is accepted.
    pub fn abi(&self) -> Result<AbiVersion, Error> {
        AbiVersion::detect(&self.exports.abi)
    }

    /// Whether the module exports `name`.
    pub fn has_export(&self, name: &str) -> bool {
        self.exports.names.contains(name)
    }

    /// The `proxy_abi_version_*` exports, in name order.
    pub fn abi_exports(&self) -> &[String] {
        &self.exports.abi
    }

    pub(crate) fn wasmtime(&self) -> &wasmtime::Module {
        &self.inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::test_support::{engine, wat_bytes};

    #[test]
    fn abi_reports_the_advertised_version() {
        // Arrange
        let engine = engine();
        let module = Module::new(
            &engine,
            &wat_bytes(r#"(module (func (export "proxy_abi_version_0_2_1")))"#),
        )
        .unwrap();

        // Act
        let version = module.abi();

        // Assert
        assert!(matches!(version, Ok(AbiVersion::V0_2_1)));
        assert_eq!(module.abi_exports(), ["proxy_abi_version_0_2_1".to_owned()]);
    }

    #[test]
    fn abi_rejects_a_module_without_a_version_export() {
        // Arrange
        let engine = engine();
        let module = Module::new(&engine, &wat_bytes("(module)")).unwrap();

        // Act
        let version = module.abi();

        // Assert
        assert!(matches!(version, Err(Error::UnsupportedAbi { found }) if found.is_empty()));
    }

    #[test]
    fn has_export_reads_the_export_table() {
        // Arrange
        let engine = engine();
        let module =
            Module::new(&engine, &wat_bytes(r#"(module (func (export "present")))"#)).unwrap();

        // Act
        let observed = (module.has_export("present"), module.has_export("absent"));

        // Assert
        assert_eq!(observed, (true, false));
    }

    #[test]
    fn a_clone_shares_the_export_table() {
        // Arrange
        let engine = engine();
        let module =
            Module::new(&engine, &wat_bytes(r#"(module (func (export "present")))"#)).unwrap();

        // Act
        let clone = module.clone();

        // Assert
        assert!(Arc::ptr_eq(&module.exports, &clone.exports));
    }

    #[test]
    fn invalid_bytes_do_not_compile() {
        // Arrange
        let engine = engine();
        let bytes = [0x00, 0x61, 0x73, 0x6d, 0x99, 0x99, 0x99, 0x99];

        // Act
        let result = Module::new(&engine, &bytes);

        // Assert
        assert!(matches!(result, Err(Error::Compile { .. })));
    }
}
