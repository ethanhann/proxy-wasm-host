//! Detection of the ABI version a guest was built for.

use std::fmt;

use crate::Error;

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
    /// Returns [`Error::UnsupportedAbi`] with the offered names when no
    /// accepted version is among them.
    pub fn detect(exports: &[String]) -> Result<Self, Error> {
        [Self::V0_2_1, Self::V0_2_0]
            .into_iter()
            .find(|version| exports.iter().any(|name| name == version.export_name()))
            .ok_or_else(|| Error::UnsupportedAbi {
                found: exports.to_vec(),
            })
    }
}

impl fmt::Display for AbiVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.export_name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(matches!(result, Err(Error::UnsupportedAbi { found }) if found == exports));
    }

    #[test]
    fn detect_rejects_an_empty_list() {
        // Arrange
        let exports: Vec<String> = Vec::new();

        // Act
        let result = AbiVersion::detect(&exports);

        // Assert
        assert!(matches!(result, Err(Error::UnsupportedAbi { found }) if found.is_empty()));
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
