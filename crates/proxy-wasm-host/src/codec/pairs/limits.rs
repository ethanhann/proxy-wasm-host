//! What one decode of a map a guest sends may read.

use crate::codec::pairs::DecodeError;

/// The most pairs one serialized map may declare by default.
///
/// The value is the one the C++ host uses, so a guest that a proxy built on
/// that host accepts is accepted here.
/// Read it through [`PairLimits::default`], which is the shape that survives
/// a change of the value.
pub(crate) const DEFAULT_MAX_DECODED_PAIRS: u32 = 1024;

/// The most bytes one serialized map may hold by default.
///
/// The value is the one the C++ host uses.
pub(crate) const DEFAULT_MAX_DECODED_MAP_BYTES: usize = 1024 * 1024;

/// What one call to [`decode_pairs`](super::decode_pairs) may read.
///
/// A guest writes the map, so a guest also chooses how large it is.
/// These two numbers bound the work one call can ask for.
/// [`PairLimits::default`] holds the values of the C++ host, and
/// [`PairLimits::unlimited`] reads whatever the input holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PairLimits {
    pairs: Option<u32>,
    bytes: Option<usize>,
}

impl Default for PairLimits {
    fn default() -> Self {
        Self {
            pairs: Some(DEFAULT_MAX_DECODED_PAIRS),
            bytes: Some(DEFAULT_MAX_DECODED_MAP_BYTES),
        }
    }
}

impl PairLimits {
    /// Sets the pair limit, or removes it with `None`.
    ///
    /// Start from [`PairLimits::default`] or [`PairLimits::unlimited`] and
    /// change the bound you care about.
    #[must_use]
    pub fn with_pairs(mut self, pairs: impl Into<Option<u32>>) -> Self {
        self.pairs = pairs.into();
        self
    }

    /// Sets the byte limit, or removes it with `None`.
    #[must_use]
    pub fn with_bytes(mut self, bytes: impl Into<Option<usize>>) -> Self {
        self.bytes = bytes.into();
        self
    }

    /// No limit of either kind.
    ///
    /// Use it when you decode bytes of your own and set your own rule.
    pub fn unlimited() -> Self {
        Self {
            pairs: None,
            bytes: None,
        }
    }

    /// The pair limit, or `None` when there is none.
    pub fn pairs(&self) -> Option<u32> {
        self.pairs
    }

    /// The byte limit, or `None` when there is none.
    pub fn bytes(&self) -> Option<usize> {
        self.bytes
    }

    pub(super) fn check_bytes(&self, data_len: usize) -> Result<(), DecodeError> {
        match self.bytes {
            Some(limit) if data_len > limit => Err(DecodeError::ByteLimit {
                bytes: data_len,
                limit,
            }),
            _ => Ok(()),
        }
    }

    pub(super) fn check_pairs(&self, pairs: u32) -> Result<(), DecodeError> {
        match self.pairs {
            Some(limit) if pairs > limit => Err(DecodeError::PairLimit { pairs, limit }),
            _ => Ok(()),
        }
    }
}
