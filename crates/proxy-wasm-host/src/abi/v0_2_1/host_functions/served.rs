//! Who answers a resource the guest asks for.

/// Whether the crate answers a resource itself or the embedder does.
///
/// A resource the crate serves is never offered to the embedder, in either
/// direction.
/// The guest reads it from the crate and a write to it is refused, and no
/// implementation ever sees the call.
///
/// Each family has one predicate that decides this from the guest's own key.
/// The predicate never reads the state, so the read body and the write body
/// of a family can share it, and the rule is written once per family rather
/// than once per direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Served {
    /// The crate answers, from what the embedder supplied before the call.
    Crate,
    /// The embedder answers, through the stream host.
    Embedder,
}
