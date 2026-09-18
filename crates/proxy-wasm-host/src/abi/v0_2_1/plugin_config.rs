//! The identity and the configuration of one plugin.

/// What one root context serves.
///
/// The ABI calls a root context the plugin context, and it gives each one a
/// name, a root id, and a configuration.
/// You pass the value to
/// [`CallScope::on_configure`](super::CallScope::on_configure), which records
/// it on the root context and reports the length of the configuration to the
/// guest.
/// The guest then reads the bytes from the `PLUGIN_CONFIGURATION` buffer.
///
/// One root context holds one plugin.
/// If you run several plugins against one instance, create one root context
/// for each and configure each with its own value.
///
/// For example, a plugin with a configuration and no name:
///
/// ```
/// use proxy_wasm_host::abi::v0_2_1::PluginConfig;
///
/// let plugin = PluginConfig::new().with_configuration(*br#"{"deny":["/admin"]}"#);
/// assert_eq!(plugin.configuration().len(), 19);
/// ```
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PluginConfig {
    name: Vec<u8>,
    root_id: Vec<u8>,
    configuration: Vec<u8>,
}

impl PluginConfig {
    /// A plugin with no name, no root id, and no configuration.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the plugin name.
    ///
    /// A guest reads it through the `plugin_name` property.
    #[must_use]
    pub fn with_name(mut self, name: impl Into<Vec<u8>>) -> Self {
        self.name = name.into();
        self
    }

    /// Sets the plugin root id.
    ///
    /// A guest reads it through the `plugin_root_id` property.
    /// This is the name the ABI gives the root context, and it is not the
    /// [`ContextId`](super::ContextId) the crate allocates.
    #[must_use]
    pub fn with_root_id(mut self, root_id: impl Into<Vec<u8>>) -> Self {
        self.root_id = root_id.into();
        self
    }

    /// Sets the bytes the guest reads from the `PLUGIN_CONFIGURATION` buffer.
    #[must_use]
    pub fn with_configuration(mut self, configuration: impl Into<Vec<u8>>) -> Self {
        self.configuration = configuration.into();
        self
    }

    /// The plugin name.
    pub fn name(&self) -> &[u8] {
        &self.name
    }

    /// The plugin root id.
    pub fn root_id(&self) -> &[u8] {
        &self.root_id
    }

    /// The bytes of the `PLUGIN_CONFIGURATION` buffer.
    pub fn configuration(&self) -> &[u8] {
        &self.configuration
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_plugin_has_three_empty_values() {
        // Arrange
        let expected: &[u8] = b"";

        // Act
        let plugin = PluginConfig::new();

        // Assert
        assert_eq!(plugin.name(), expected);
        assert_eq!(plugin.root_id(), expected);
        assert_eq!(plugin.configuration(), expected);
    }

    #[test]
    fn each_builder_method_stores_its_own_value() {
        // Arrange
        let plugin = PluginConfig::new();

        // Act
        let plugin = plugin
            .with_name(*b"auth")
            .with_root_id(*b"auth_root")
            .with_configuration(*b"{}");

        // Assert
        assert_eq!(plugin.name(), b"auth");
        assert_eq!(plugin.root_id(), b"auth_root");
        assert_eq!(plugin.configuration(), b"{}");
    }
}
