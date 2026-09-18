//! The `env` functions the ABI names.
//!
//! Every one of the 39 functions is registered for every guest, because a
//! guest built with an SDK imports most of them whether or not it calls them.
//! The functions this crate serves have bodies, and the rest are stubs
//! that answer `UNIMPLEMENTED`.

mod buffer;
mod call;
mod callout;
mod clock;
mod context;
mod failure;
mod foreign;
mod header_map;
mod local_response;
mod log;
mod metric;
mod property;
mod served;
mod shared_data;
mod shared_queue;
mod stream;
pub(crate) mod table;
mod timer;

pub(crate) use failure::{Failure, complete, stub};
pub(super) use served::Served;
pub(crate) use table::register;
