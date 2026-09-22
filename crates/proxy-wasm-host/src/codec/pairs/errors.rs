//! The errors of the pair codec.

use std::fmt;

/// Which half of a pair an error refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    /// The key of the pair.
    Key,
    /// The value of the pair.
    Value,
}

impl fmt::Display for Field {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Key => f.write_str("key"),
            Self::Value => f.write_str("value"),
        }
    }
}

/// Why an encoded map could not be decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum DecodeError {
    /// The input is shorter than the pair count.
    #[error("input ends before the pair count")]
    TruncatedCount,
    /// The input ends inside the table of key and value lengths.
    #[error("input ends inside the length table, {pairs} pairs declared")]
    TruncatedLengths {
        /// The pair count the input declared.
        pairs: u32,
    },
    /// The input ends inside a key or before its terminator.
    #[error("input ends inside the key of pair {pair}")]
    TruncatedKey {
        /// The zero based index of the pair.
        pair: u32,
    },
    /// The input ends inside a value or before its terminator.
    #[error("input ends inside the value of pair {pair}")]
    TruncatedValue {
        /// The zero based index of the pair.
        pair: u32,
    },
    /// The byte after a key or a value is not `0x00`.
    #[error("pair {pair} {field} is not terminated by 0x00")]
    MissingTerminator {
        /// The zero based index of the pair.
        pair: u32,
        /// Which half of the pair lacks its terminator.
        field: Field,
    },
    /// Bytes remain after the last pair.
    #[error("{count} bytes remain after the last pair")]
    TrailingBytes {
        /// How many bytes remain.
        count: usize,
    },
    /// The input declares more pairs than the limit allows.
    #[error("{pairs} pairs exceed the limit of {limit}")]
    PairLimit {
        /// The pair count the input declared.
        pairs: u32,
        /// The limit in force.
        limit: u32,
    },
    /// The input is longer than the limit allows.
    #[error("{bytes} bytes exceed the limit of {limit}")]
    ByteLimit {
        /// The length of the input.
        bytes: usize,
        /// The limit in force.
        limit: usize,
    },
}

/// Why a map could not be encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum EncodeError {
    /// A key or a value is longer than a `u32` length can express.
    #[error("pair {pair} {field} is {len} bytes long, which exceeds u32::MAX")]
    TooLong {
        /// The zero based index of the pair.
        pair: usize,
        /// Which half of the pair is too long.
        field: Field,
        /// The length in bytes.
        len: usize,
    },
    /// There are more pairs than a `u32` count can express.
    #[error("{count} pairs exceeds u32::MAX")]
    TooManyPairs {
        /// The pair count.
        count: usize,
    },
    /// The encoded map would not fit in memory.
    #[error("the encoded map exceeds usize::MAX bytes")]
    TooLarge,
    /// The source yielded different pairs on its second walk.
    #[error("the pairs changed while they were being encoded")]
    Changed,
}
