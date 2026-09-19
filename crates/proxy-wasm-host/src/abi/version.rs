//! Detection of the ABI version a guest was built for.

use std::fmt;

/// A Proxy-Wasm ABI version that the crate accepts.
///
/// A guest advertises its version with an exported function named
/// `proxy_abi_version_<major>_<minor>_<patch>`.
/// The enum is non exhaustive, because a later ABI version adds a variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum AbiVersion {
    /// ABI v0.2.0, which differs from v0.2.1 only by `proxy_get_log_level`.
    V0_2_0,
    /// ABI v0.2.1, the version this crate implements.
    V0_2_1,
}

impl AbiVersion {
    /// The name of the export that advertises this version.
    pub fn export_name(self) -> &'static str {
        match self {
            Self::V0_2_0 => "proxy_abi_version_0_2_0",
            Self::V0_2_1 => "proxy_abi_version_0_2_1",
        }
    }

    /// Picks the newest accepted version among a guest's ABI exports.
    ///
    /// # Errors
    ///
    /// Returns [`UnsupportedAbi`] with the offered names when no accepted
    /// version is among them.
    pub fn detect(exports: &[String]) -> Result<Self, UnsupportedAbi> {
        [Self::V0_2_1, Self::V0_2_0]
            .into_iter()
            .find(|version| exports.iter().any(|name| name == version.export_name()))
            .ok_or_else(|| UnsupportedAbi {
                found: exports.to_vec(),
            })
    }
}

impl fmt::Display for AbiVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.export_name())
    }
}

/// A module advertises no ABI version that the crate accepts.
///
/// [`Module::abi_exports`](crate::Module::abi_exports) lists what a module
/// advertises, and [`AbiVersion::detect`] reads that list.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("no supported proxy_abi_version export, found {found:?}")]
pub struct UnsupportedAbi {
    /// The `proxy_abi_version_*` exports the module has.
    pub found: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::v0_2_1::test_support::{engine, wat_bytes};

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn detect_prefers_the_newest_accepted_version() {
        // Arrange
        let lists = [
            names(&["proxy_abi_version_0_2_1"]),
            names(&["proxy_abi_version_0_2_0"]),
            names(&["proxy_abi_version_0_2_0", "proxy_abi_version_0_2_1"]),
        ];

        // Act
        let detected: Vec<_> = lists
            .iter()
            .map(|list| AbiVersion::detect(list).unwrap())
            .collect();

        // Assert
        assert_eq!(
            detected,
            vec![AbiVersion::V0_2_1, AbiVersion::V0_2_0, AbiVersion::V0_2_1]
        );
    }

    #[test]
    fn detect_rejects_an_old_version_and_names_it() {
        // Arrange
        let exports = names(&["proxy_abi_version_0_1_0"]);

        // Act
        let result = AbiVersion::detect(&exports);

        // Assert
        assert_eq!(result, Err(UnsupportedAbi { found: exports }));
    }

    #[test]
    fn detect_rejects_an_empty_list() {
        // Arrange
        let exports: Vec<String> = Vec::new();

        // Act
        let result = AbiVersion::detect(&exports);

        // Assert
        assert_eq!(result, Err(UnsupportedAbi { found: Vec::new() }));
    }

    #[test]
    fn detect_reads_the_markers_a_module_lists() {
        // Arrange
        let engine = engine();
        let bytes = wat_bytes(r#"(module (func (export "proxy_abi_version_0_2_1")))"#);
        let module = crate::Module::new(&engine, &bytes).unwrap();

        // Act
        let version = AbiVersion::detect(module.abi_exports());

        // Assert
        assert_eq!(version, Ok(AbiVersion::V0_2_1));
    }

    #[test]
    fn display_prints_the_export_name() {
        // Arrange
        let versions = [AbiVersion::V0_2_0, AbiVersion::V0_2_1];

        // Act
        let texts: Vec<String> = versions.iter().map(ToString::to_string).collect();

        // Assert
        assert_eq!(
            texts,
            vec!["proxy_abi_version_0_2_0", "proxy_abi_version_0_2_1"]
        );
    }
}
