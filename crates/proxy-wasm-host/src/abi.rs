//! One module per supported ABI version.
//!
//! Each version module owns the types, host functions, and callbacks that
//! differ between versions.
//! The codec, the buffer trait, and the header map trait are shared.
//! [`AbiVersion`] names the versions the crate accepts and detects one from a
//! guest's exports.

pub mod v0_2_1;
mod version;

pub use version::AbiVersion;
