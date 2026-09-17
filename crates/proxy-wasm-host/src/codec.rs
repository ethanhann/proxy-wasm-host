//! The serialization rules of the ABI.
//!
//! A guest and the host exchange maps of byte string pairs and property paths
//! as flat byte sequences.
//! [`pairs`] handles the map form.
//! [`path`] handles the property path form.
//! Both follow the Serialization section of the ABI document, and every
//! integer is little-endian.

pub mod pairs;
pub mod path;
