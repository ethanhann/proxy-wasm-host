//! The report a default body makes about itself.

/// Records that a default body answered, so an embedder that forgot a method
/// finds it in the log rather than in a guest that stopped.
///
/// This is the level the crate uses for a host function it does not serve, so
/// one subscriber setting shows both.
pub(crate) fn unserved(method: &'static str) {
    tracing::debug!(method, "the embedder does not serve this");
}
