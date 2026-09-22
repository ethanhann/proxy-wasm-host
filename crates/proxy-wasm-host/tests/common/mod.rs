//! The recorder and the harness that the end to end tests share.
//!
//! Each test file compiles this module on its own and uses a part of it, so
//! the items that one file does not use would each warn as dead code.
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

pub mod harness;
pub mod recorder;
