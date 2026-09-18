//! Who answers a resource the guest asks for.

/// Whether the crate answers a resource itself or the embedder does.
///
/// A resource the crate serves is never offered to the embedder, in either
/// direction.
/// The guest reads it from the crate.
/// A write to it is refused, and no implementation sees the call.
///
/// Each family has one predicate that decides this from the guest's own key.
/// The predicate never reads the state, so the read body and the write body
/// of a family share it, and the rule is written once per family rather than
/// once per direction.
///
/// The crate arm names which resource, so a body that resolves it stays
/// exhaustive and the compiler reports a family that gains a resource the
/// body does not answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Served<T> {
    /// The crate answers, from what the embedder supplied before the call.
    Crate(T),
    /// The embedder answers, through the stream host.
    Embedder,
}
