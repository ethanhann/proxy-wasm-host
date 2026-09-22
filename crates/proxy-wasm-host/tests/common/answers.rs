//! The answers a test asks the doubles to give.
//!
//! The doubles accept every call by default, which is what most tests want.
//! A test of a refusal path fills one of these tables in its Arrange section,
//! and the double then reports that refusal in place of its usual answer.
//!
//! A guest of the Rust SDK ends its stream or panics on a refusal, so a test
//! that fills a table is usually testing what the guest does next.

use std::collections::BTreeMap;

use proxy_wasm_host::abi::v0_2_1::types::Status;
use proxy_wasm_host::abi::v0_2_1::{GrpcOpenRefusal, HttpCallRefusal};

/// A method of `StreamState` that a test can make refuse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum StreamCall {
    HeaderMap,
    Buffer,
    ContinueStream,
    CloseStream,
    SendLocalResponse,
    Property,
    SetProperty,
    CallForeignFunction,
}

/// A method of `Callouts` that a test can make refuse.
///
/// The three methods that answer nothing are absent, because a host that
/// cannot refuse cannot be asked to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CalloutCall {
    HttpCall,
    GrpcCall,
    GrpcStream,
}

/// The refusal one callout method answers, which differs by method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalloutRefusal {
    Http(HttpCallRefusal),
    GrpcOpen(GrpcOpenRefusal),
}

/// The refusals of the stream methods, by method.
pub type StreamRefusals = BTreeMap<StreamCall, Status>;

/// The refusals of the callout methods, by method.
pub type CalloutRefusals = BTreeMap<CalloutCall, CalloutRefusal>;
