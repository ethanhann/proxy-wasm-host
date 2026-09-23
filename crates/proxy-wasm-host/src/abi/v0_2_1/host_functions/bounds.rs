//! The two limits a guest input meets before a service is called.
//!
//! The crate keeps a record of every queue and metric a guest holds, and one
//! of those records keeps the name the guest sent. Both grow on a call that
//! any [`SharedServices`](crate::abi::v0_2_1::SharedServices) answers, so the
//! limit is here rather than in one implementation of that trait.

use crate::abi::v0_2_1::AbiAccess;
use crate::abi::v0_2_1::host_functions::Failure;
use crate::abi::v0_2_1::types::Status;
use crate::runtime::HostState;

/// Refuses a name or a key that is longer than the embedder allows.
pub(super) fn within_name_bytes(state: &HostState, name: &[u8]) -> Result<(), Failure> {
    match state.max_name_bytes() {
        Some(max) if name.len() > max => Err(Status::InternalFailure.into()),
        _ => Ok(()),
    }
}

/// Refuses a new queue or metric to a guest that holds as many as the
/// embedder allows.
///
/// Call it before the service, because the service creates what the guest
/// asked for.
pub(super) fn within_shared_names(state: &HostState) -> Result<(), Failure> {
    match state.max_shared_names() {
        Some(max) if state.abi().shared_names() >= max => Err(Status::InternalFailure.into()),
        _ => Ok(()),
    }
}
