//! What a guest exports beside the ABI's own callbacks.

use crate::Error;
use crate::abi::v0_2_1::{AbiAccess, Guest};

impl Guest {
    /// Whether an earlier failure unwound a guest call.
    ///
    /// A poisoned guest refuses every later call.
    /// Recovery is a new guest from the same module, which keeps the
    /// compiled artifact and loses the context table and the identifiers
    /// this guest obtained.
    pub fn is_poisoned(&self) -> bool {
        self.instance.is_poisoned()
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
    ///
    /// # Errors
    ///
    /// Returns [`Error::Config`] while a callback is running, because a call
    /// from there would leave the two views of the contexts disagreeing.
    /// Returns the errors of a guest call otherwise.
    pub fn call_export<P: wasmtime::WasmParams, R: wasmtime::WasmResults>(
        &mut self,
        name: &str,
        params: P,
    ) -> Result<R, Error> {
        if self.instance.state().abi().current_callback().is_some() {
            return Err(Error::Config {
                message: format!("{name} cannot be called while a callback is running"),
            });
        }
        self.instance.call(name, params)
    }
}
