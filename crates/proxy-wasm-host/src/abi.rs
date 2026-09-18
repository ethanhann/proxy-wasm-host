//! One module per supported ABI version.
//!
//! Each version module owns the types, host functions, and callbacks that
//! differ between versions.
//! The codec, the buffer trait, and the header map trait are shared.
//! [`AbiVersion`] names the versions the crate accepts and detects one from a
//! guest's exports.

use std::any::Any;

pub mod v0_2_1;
mod version;

pub use version::AbiVersion;

/// Builds the ABI state of one instance, boxed for the store data to hold.
///
/// The runtime keeps this as an opaque value and never reads inside it, so
/// the ABI layer adds state without a change under `runtime/`.
/// The runtime decides when an instance gets one, and the ABI layer decides
/// what it is.
pub(crate) fn state() -> Box<dyn Any + Send> {
    Box::new(v0_2_1::AbiState::new())
}
