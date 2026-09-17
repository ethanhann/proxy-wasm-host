//! The callbacks a guest exports.

use std::fmt;

/// A `proxy_on_*` callback that this crate can call.
///
/// Every callback is optional for a guest.
/// [`Guest::exports_callback`](super::Guest::exports_callback) tells you
/// whether a guest exports one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Callback {
    /// `proxy_on_context_create`.
    ContextCreate,
    /// `proxy_on_vm_start`.
    VmStart,
    /// `proxy_on_configure`.
    Configure,
    /// `proxy_on_request_headers`.
    RequestHeaders,
    /// `proxy_on_done`.
    Done,
    /// `proxy_on_log`.
    Log,
    /// `proxy_on_delete`.
    Delete,
}

impl Callback {
    /// The export name the ABI gives the callback.
    pub fn export_name(self) -> &'static str {
        match self {
            Self::ContextCreate => "proxy_on_context_create",
            Self::VmStart => "proxy_on_vm_start",
            Self::Configure => "proxy_on_configure",
            Self::RequestHeaders => "proxy_on_request_headers",
            Self::Done => "proxy_on_done",
            Self::Log => "proxy_on_log",
            Self::Delete => "proxy_on_delete",
        }
    }

    /// Every callback this crate drives, in lifecycle order.
    pub const ALL: &[Self] = &[
        Self::ContextCreate,
        Self::VmStart,
        Self::Configure,
        Self::RequestHeaders,
        Self::Done,
        Self::Log,
        Self::Delete,
    ];
}

impl fmt::Display for Callback {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.export_name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn position(callback: Callback) -> usize {
        match callback {
            Callback::ContextCreate => 0,
            Callback::VmStart => 1,
            Callback::Configure => 2,
            Callback::RequestHeaders => 3,
            Callback::Done => 4,
            Callback::Log => 5,
            Callback::Delete => 6,
        }
    }

    #[test]
    fn every_callback_is_listed_once_in_lifecycle_order() {
        // Arrange
        let callbacks = Callback::ALL;

        // Act
        let positions: Vec<usize> = callbacks
            .iter()
            .map(|callback| position(*callback))
            .collect();

        // Assert
        assert_eq!(positions, (0..7).collect::<Vec<usize>>());
        assert_eq!(Callback::Delete.to_string(), "proxy_on_delete");
        assert!(
            callbacks
                .iter()
                .all(|callback| callback.export_name().starts_with("proxy_on_"))
        );
    }
}
