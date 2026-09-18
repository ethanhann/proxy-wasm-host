//! The report a default body makes about itself.

/// Records that a default body answered, so an embedder that forgot a method
/// finds it in the log rather than in a guest that stopped.
///
/// The level is warn, because a default body answering is an embedder mistake
/// and the guest sees only a status.
/// A guest built with the Rust SDK stops on that status, so without this line
/// the visible result is a trap with no cause beside it.
pub(crate) fn unserved(method: &'static str) {
    tracing::warn!(method, "the embedder does not serve this");
}
