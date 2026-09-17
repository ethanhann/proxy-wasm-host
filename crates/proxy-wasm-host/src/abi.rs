//! One module per supported ABI version.
//!
//! Each version module owns the types, host functions, and callbacks that
//! differ between versions.
//! The codec, the buffer trait, and the header map trait are shared.

pub mod v0_2_1;
