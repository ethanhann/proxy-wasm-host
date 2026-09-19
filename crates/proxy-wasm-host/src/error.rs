//! The errors that you observe.
//!
//! A guest observes only [`Status`] values, which never unwind.
//! Everything that unwinds a guest call, fails to build an instance, or
//! rejects a guest address is one of the types here.
//! The failures that an ABI version defines are on that version's module,
//! for example [`GuestError`].
//!
//! [`Status`]: crate::abi::v0_2_1::types::Status
//! [`GuestError`]: crate::abi::v0_2_1::GuestError

#[doc(inline)]
pub use proxy_wasm_host_internal::{Error, Limit, MemoryError};
