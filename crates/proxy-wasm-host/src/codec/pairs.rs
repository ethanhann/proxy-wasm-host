//! Serialization of a map of byte string pairs.
//!
//! A non empty map is a `u32` pair count, then a `u32` key length and a `u32`
//! value length for each pair, then each key and value in turn with a `0x00`
//! byte after each one.
//! An empty map is either no bytes or a single `0x00` byte.
//!
//! The decoder is strict.
//! A guest controls the input, so a truncated table, a missing terminator, or
//! bytes after the last pair each produce a named [`DecodeError`] rather than
//! a partial map.
//!
//! The encoder can read its pairs from a visitor, so a header map that does
//! not store its pairs contiguously is encoded without an owned copy of each
//! pair.

use std::ops::ControlFlow;

mod errors;
mod limits;

pub use errors::{DecodeError, EncodeError, Field};
pub use limits::{DEFAULT_MAX_DECODED_MAP_BYTES, DEFAULT_MAX_DECODED_PAIRS, PairLimits};

/// The borrowed pairs that [`decode_pairs`] returns.
pub type Pairs<'a> = Vec<(&'a [u8], &'a [u8])>;

/// A closure that receives one pair at a time and says whether to continue.
pub type PairVisitor<'a> = dyn FnMut(&[u8], &[u8]) -> ControlFlow<()> + 'a;

/// A closure that walks every pair of a map in order and stops when the
/// visitor asks it to.
pub type PairSource<'a> = dyn FnMut(&mut PairVisitor<'_>) -> ControlFlow<()> + 'a;

/// The size of the pair count at the start of an encoded map.
pub const COUNT_SIZE: usize = 4;

const LENGTH_SIZE: usize = 4;
const TERMINATOR: u8 = 0;

/// How many pairs to reserve for a map that declares `pair_count`.
///
/// The data section that follows the length table holds at least two bytes
/// for each pair, which are the two terminators, so a map that declares more
/// pairs than the remaining bytes allow cannot decode them all.
/// The reserve follows the pairs that can decode rather than the number the
/// guest wrote.
fn reserve_for(pair_count: usize, data_len: usize, table_end: usize) -> usize {
    pair_count.min(data_len.saturating_sub(table_end) / 2)
}

/// The encoded size of one pair with the given key and value lengths.
///
/// The size covers the two length fields, the key, the value, and the two
/// terminators.
/// The addition saturates.
pub const fn pair_encoded_size(key_len: usize, value_len: usize) -> usize {
    (2 * LENGTH_SIZE)
        .saturating_add(key_len)
        .saturating_add(1)
        .saturating_add(value_len)
        .saturating_add(1)
}

/// The encoded size of a map with `count` pairs whose pair sizes sum to
/// `pairs_size`.
///
/// A map with no pairs has size zero.
/// Every other map adds [`COUNT_SIZE`] once.
/// The addition saturates.
pub const fn total_size(count: usize, pairs_size: usize) -> usize {
    if count == 0 {
        0
    } else {
        COUNT_SIZE.saturating_add(pairs_size)
    }
}

/// The size that [`encode_pairs`] would produce, without building it.
pub fn encoded_size<K: AsRef<[u8]>, V: AsRef<[u8]>>(pairs: &[(K, V)]) -> usize {
    let pairs_size = pairs.iter().fold(0usize, |sum, (key, value)| {
        sum.saturating_add(pair_encoded_size(key.as_ref().len(), value.as_ref().len()))
    });
    total_size(pairs.len(), pairs_size)
}

/// Encodes pairs in the order given.
///
/// Duplicate keys, empty keys, and empty values are kept as they are.
/// An empty slice encodes to no bytes.
///
/// # Errors
///
/// Returns [`EncodeError`] when a key or a value is longer than `u32::MAX`
/// bytes or when there are more than `u32::MAX` pairs.
pub fn encode_pairs<K: AsRef<[u8]>, V: AsRef<[u8]>>(
    pairs: &[(K, V)],
) -> Result<Vec<u8>, EncodeError> {
    encode_visited(&mut |visitor| {
        for (key, value) in pairs {
            visitor(key.as_ref(), value.as_ref())?;
        }
        ControlFlow::Continue(())
    })
}

/// Encodes the pairs that `source` yields, without an owned copy of any pair.
///
/// The source is walked twice.
/// The first walk validates every length and computes the exact size.
/// The second walk writes the keys and values into one allocation.
///
/// # Errors
///
/// Returns [`EncodeError`] when a key or a value is longer than `u32::MAX`
/// bytes, when there are more than `u32::MAX` pairs, when the encoded map
/// would not fit in memory, or when the second walk yields different pairs
/// than the first.
pub fn encode_visited(source: &mut PairSource<'_>) -> Result<Vec<u8>, EncodeError> {
    let lengths = measure(source)?;
    if lengths.is_empty() {
        return Ok(Vec::new());
    }
    let count = u32::try_from(lengths.len()).map_err(|_| EncodeError::TooManyPairs {
        count: lengths.len(),
    })?;
    let size = lengths
        .iter()
        .try_fold(COUNT_SIZE, |sum, &(key_len, value_len)| {
            sum.checked_add(pair_encoded_size(key_len as usize, value_len as usize))
        })
        .ok_or(EncodeError::TooLarge)?;

    let mut out = Vec::with_capacity(size);
    out.extend_from_slice(&count.to_le_bytes());
    for (key_len, value_len) in &lengths {
        out.extend_from_slice(&key_len.to_le_bytes());
        out.extend_from_slice(&value_len.to_le_bytes());
    }
    let mut written = 0usize;
    let _ = source(&mut |key, value| {
        if written == lengths.len() {
            return ControlFlow::Break(());
        }
        out.extend_from_slice(key);
        out.push(TERMINATOR);
        out.extend_from_slice(value);
        out.push(TERMINATOR);
        written += 1;
        ControlFlow::Continue(())
    });
    if out.len() == size {
        Ok(out)
    } else {
        Err(EncodeError::Changed)
    }
}

fn measure(source: &mut PairSource<'_>) -> Result<Vec<(u32, u32)>, EncodeError> {
    let mut lengths = Vec::new();
    let mut failure = None;
    let _ = source(&mut |key, value| {
        let pair = lengths.len();
        match (
            length_of(pair, Field::Key, key),
            length_of(pair, Field::Value, value),
        ) {
            (Ok(key_len), Ok(value_len)) => {
                lengths.push((key_len, value_len));
                ControlFlow::Continue(())
            }
            (Err(error), _) | (_, Err(error)) => {
                failure = Some(error);
                ControlFlow::Break(())
            }
        }
    });
    match failure {
        Some(error) => Err(error),
        None => Ok(lengths),
    }
}

fn length_of(pair: usize, field: Field, bytes: &[u8]) -> Result<u32, EncodeError> {
    u32::try_from(bytes.len()).map_err(|_| EncodeError::TooLong {
        pair,
        field,
        len: bytes.len(),
    })
}

/// Decodes an encoded map into pairs that borrow from `data`.
///
/// No bytes and a single `0x00` byte both decode to an empty map.
/// `limits` bounds the work one call can ask for, and
/// [`PairLimits::unlimited`] removes both bounds.
/// The byte length is checked first, then the declared pair count, and both
/// before the call allocates anything.
///
/// # Errors
///
/// Returns [`DecodeError::ByteLimit`] when the input is longer than the limit
/// allows, and [`DecodeError::PairLimit`] when it declares more pairs than
/// the limit allows.
/// Returns the other [`DecodeError`] values when the input is truncated at
/// any point, when a key or a value is not followed by `0x00`, or when bytes
/// remain after the last pair.
pub fn decode_pairs(data: &[u8], limits: PairLimits) -> Result<Pairs<'_>, DecodeError> {
    limits.check(data.len())?;
    if data.is_empty() || data == [TERMINATOR] {
        return Ok(Vec::new());
    }
    let count = data
        .first_chunk::<COUNT_SIZE>()
        .map(|word| u32::from_le_bytes(*word))
        .ok_or(DecodeError::TruncatedCount)?;
    limits.check_count(count)?;
    let truncated_lengths = DecodeError::TruncatedLengths { pairs: count };
    let pair_count = usize::try_from(count).map_err(|_| truncated_lengths)?;
    let table_end = pair_count
        .checked_mul(2 * LENGTH_SIZE)
        .and_then(|table_len| COUNT_SIZE.checked_add(table_len))
        .filter(|&end| end <= data.len())
        .ok_or(truncated_lengths)?;

    let mut pairs = Vec::with_capacity(reserve_for(pair_count, data.len(), table_end));
    let mut cursor = Cursor {
        data,
        pos: table_end,
        pair: 0,
    };
    let (table, _) = data[COUNT_SIZE..table_end].as_chunks::<{ 2 * LENGTH_SIZE }>();
    for lengths in table {
        let key_len = lengths
            .first_chunk::<LENGTH_SIZE>()
            .ok_or(truncated_lengths)?;
        let value_len = lengths
            .last_chunk::<LENGTH_SIZE>()
            .ok_or(truncated_lengths)?;
        let key = cursor.take(u32::from_le_bytes(*key_len), Field::Key)?;
        let value = cursor.take(u32::from_le_bytes(*value_len), Field::Value)?;
        pairs.push((key, value));
        cursor.pair += 1;
    }
    cursor.finish()?;
    Ok(pairs)
}

/// The read position inside the data section of an encoded map.
struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
    pair: u32,
}

impl<'a> Cursor<'a> {
    /// Takes `len` bytes and their terminator for one field of the current
    /// pair.
    fn take(&mut self, len: u32, field: Field) -> Result<&'a [u8], DecodeError> {
        let truncated = match field {
            Field::Key => DecodeError::TruncatedKey { pair: self.pair },
            Field::Value => DecodeError::TruncatedValue { pair: self.pair },
        };
        let len = usize::try_from(len).map_err(|_| truncated)?;
        let end = self.pos.checked_add(len).ok_or(truncated)?;
        let bytes = self.data.get(self.pos..end).ok_or(truncated)?;
        match self.data.get(end) {
            None => Err(truncated),
            Some(&TERMINATOR) => {
                self.pos = end + 1;
                Ok(bytes)
            }
            Some(_) => Err(DecodeError::MissingTerminator {
                pair: self.pair,
                field,
            }),
        }
    }

    fn finish(self) -> Result<(), DecodeError> {
        let remaining = self.data.len() - self.pos;
        if remaining == 0 {
            Ok(())
        } else {
            Err(DecodeError::TrailingBytes { count: remaining })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type OwnedPairs = Vec<(Vec<u8>, Vec<u8>)>;

    /// A map of `count` pairs whose keys and values are one byte each.
    fn map_of(count: usize) -> Vec<u8> {
        let pairs: Vec<(Vec<u8>, Vec<u8>)> = (0..count).map(|_| (vec![b'k'], vec![b'v'])).collect();
        encode_pairs(&pairs).unwrap()
    }

    #[test]
    fn a_map_above_the_pair_limit_is_refused_before_it_is_built() {
        // Arrange
        let input = map_of(1025);

        // Act
        let result = decode_pairs(&input, PairLimits::default());

        // Assert
        assert_eq!(
            result,
            Err(DecodeError::PairLimit {
                pairs: 1025,
                limit: DEFAULT_MAX_DECODED_PAIRS,
            })
        );
    }

    #[test]
    fn a_map_at_the_pair_limit_is_accepted() {
        // Arrange
        let input = map_of(1024);

        // Act
        let result = decode_pairs(&input, PairLimits::default());

        // Assert
        assert_eq!(result.map(|pairs| pairs.len()), Ok(1024));
    }

    #[test]
    fn a_map_above_the_byte_limit_is_refused_before_the_count_is_read() {
        // Arrange
        let input = vec![b'x'; 9];
        let limits = PairLimits::new(None, 8);

        // Act
        let result = decode_pairs(&input, limits);

        // Assert
        assert_eq!(
            result,
            Err(DecodeError::ByteLimit { bytes: 9, limit: 8 }),
            "a byte limit must be checked before the pair count is read"
        );
    }

    #[test]
    fn an_unlimited_decode_accepts_a_map_above_both_limits() {
        // Arrange
        let input = map_of(2000);

        // Act
        let result = decode_pairs(&input, PairLimits::unlimited());

        // Assert
        assert_eq!(result.map(|pairs| pairs.len()), Ok(2000));
    }

    #[test]
    fn the_reserve_grows_with_the_pairs_that_decode() {
        // Arrange
        let declared = 100_000usize;
        let table_end = COUNT_SIZE + declared * 2 * LENGTH_SIZE;
        let data_len = table_end + 20;

        // Act
        let reserve = reserve_for(declared, data_len, table_end);

        // Assert
        assert_eq!(reserve, 10, "the reserve must follow the bytes that remain");
        assert_eq!(reserve_for(3, COUNT_SIZE + 24 + 12, COUNT_SIZE + 24), 3);
    }

    fn owned(pairs: &[(&[u8], &[u8])]) -> OwnedPairs {
        pairs
            .iter()
            .map(|(k, v)| (k.to_vec(), v.to_vec()))
            .collect()
    }

    fn round_trip(map: &[(Vec<u8>, Vec<u8>)]) -> Result<OwnedPairs, DecodeError> {
        decode_pairs(&encode_pairs(map).unwrap(), PairLimits::unlimited())
            .map(|pairs| owned(&pairs))
    }

    /// A small deterministic generator, so the generated tests need no
    /// dependency and fail the same way every time.
    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            self.0 >> 33
        }

        fn below(&mut self, bound: u64) -> usize {
            usize::try_from(self.next() % bound).unwrap()
        }

        fn bytes(&mut self, len: usize) -> Vec<u8> {
            (0..len)
                .map(|_| u8::try_from(self.next() % 256).unwrap())
                .collect()
        }

        fn map(&mut self, max_pairs: u64, max_value_len: u64) -> Vec<(Vec<u8>, Vec<u8>)> {
            let count = self.below(max_pairs);
            (0..count)
                .map(|_| {
                    let key_len = self.below(9);
                    let value_len = self.below(max_value_len);
                    (self.bytes(key_len), self.bytes(value_len))
                })
                .collect()
        }
    }

    #[test]
    fn spec_example_encodes_to_the_rule_bytes() {
        // Arrange
        let pairs: [(&[u8], &[u8]); 2] = [(b"a", b"1"), (b"b", b"22")];
        let expected = [
            2, 0, 0, 0, // count
            1, 0, 0, 0, 1, 0, 0, 0, // lengths of pair 0
            1, 0, 0, 0, 2, 0, 0, 0, // lengths of pair 1
            0x61, 0x00, 0x31, 0x00, // "a", "1"
            0x62, 0x00, 0x32, 0x32, 0x00, // "b", "22"
        ];

        // Act
        let encoded = encode_pairs(&pairs).unwrap();

        // Assert
        assert_eq!(encoded, expected);
    }

    #[test]
    fn hand_written_maps_round_trip() {
        // Arrange
        let maps: Vec<Vec<(Vec<u8>, Vec<u8>)>> = vec![
            owned(&[]),
            owned(&[(b"k", b"v")]),
            owned(&[(b"dup", b"one"), (b"dup", b"two")]),
            owned(&[(b"", b"empty key")]),
            owned(&[(b"empty value", b"")]),
            owned(&[(b"bin", b"\x00\x01\x00"), (b"\x00", b"\x00")]),
        ];
        let expected: Vec<_> = maps.iter().cloned().map(Ok).collect();

        // Act
        let round_trips: Vec<_> = maps.iter().map(|map| round_trip(map)).collect();

        // Assert
        assert_eq!(round_trips, expected);
    }

    #[test]
    fn order_is_preserved_through_a_round_trip() {
        // Arrange
        let unsorted = owned(&[
            (b"zeta", b"1"),
            (b"alpha", b"2"),
            (b"mid", b"3"),
            (b"alpha", b"4"),
        ]);

        // Act
        let decoded = round_trip(&unsorted);

        // Assert
        assert_eq!(decoded, Ok(unsorted));
    }

    #[test]
    fn generated_maps_round_trip() {
        // Arrange
        let mut lcg = Lcg(0x5eed);
        let mut maps: Vec<_> = (0..100).map(|_| lcg.map(6, 40)).collect();
        maps.push(lcg.map(400, 600));
        maps.push(
            (0..300)
                .map(|i| (vec![b'k'], vec![u8::try_from(i % 256).unwrap(); 300]))
                .collect(),
        );
        let expected: Vec<_> = maps.iter().cloned().map(Ok).collect();

        // Act
        let round_trips: Vec<_> = maps.iter().map(|map| round_trip(map)).collect();

        // Assert
        assert_eq!(round_trips, expected);
    }

    #[test]
    fn generated_inputs_never_panic_and_accepted_inputs_are_canonical() {
        // Arrange
        let mut lcg = Lcg(0x0bad_5eed);
        let valid = encode_pairs(&[(b"ab".as_slice(), b"cde".as_slice()), (b"", b"x")]).unwrap();
        let mut inputs: Vec<Vec<u8>> = (0..64)
            .flat_map(|len| (0..8).map(move |_| len))
            .map(|len| lcg.bytes(len))
            .collect();
        for index in 0..valid.len() {
            for byte in [0x00, 0x01, 0xff] {
                let mut mutated = valid.clone();
                mutated[index] = byte;
                inputs.push(mutated);
            }
        }

        // Act
        let outcomes: Vec<Option<Vec<u8>>> = inputs
            .iter()
            .map(|input| {
                decode_pairs(input, PairLimits::unlimited())
                    .ok()
                    .map(|pairs| encode_pairs(&pairs).unwrap())
            })
            .collect();

        // Assert
        for (input, outcome) in inputs.iter().zip(&outcomes) {
            if let Some(reencoded) = outcome {
                let is_empty_form = input.is_empty() || input == &[0x00] || input == &[0, 0, 0, 0];
                assert!(
                    reencoded == input || (is_empty_form && reencoded.is_empty()),
                    "{input:?}"
                );
            }
        }
    }

    #[test]
    fn encoded_size_matches_the_encoded_length() {
        // Arrange
        let maps: Vec<Vec<(Vec<u8>, Vec<u8>)>> = vec![
            owned(&[]),
            owned(&[(b"k", b"v")]),
            owned(&[(b"", b"")]),
            owned(&[(b"abc", b"defg"), (b"", b"x")]),
        ];
        let lengths: Vec<_> = maps
            .iter()
            .map(|map| encode_pairs(map).unwrap().len())
            .collect();

        // Act
        let sizes: Vec<_> = maps.iter().map(|map| encoded_size(map)).collect();

        // Assert
        assert_eq!(sizes, lengths);
        assert_eq!(sizes, vec![0, 4 + 8 + 4, 4 + 8 + 2, 4 + 16 + 9 + 3]);
    }

    #[test]
    fn pair_encoded_size_saturates() {
        // Arrange
        let key_len = usize::MAX;

        // Act
        let size = pair_encoded_size(key_len, 1);

        // Assert
        assert_eq!(size, usize::MAX);
    }

    #[test]
    fn total_size_is_zero_for_no_pairs_and_adds_the_count_once() {
        // Arrange
        let cases = [(0, 0), (0, 10), (1, 10), (3, 30)];

        // Act
        let sizes: Vec<_> = cases
            .iter()
            .map(|&(count, sum)| total_size(count, sum))
            .collect();

        // Assert
        assert_eq!(sizes, vec![0, 0, 14, 34]);
    }

    #[test]
    fn encoder_accepts_owned_borrowed_and_str_pairs() {
        // Arrange
        let owned_pairs = vec![(b"k".to_vec(), b"v".to_vec())];
        let borrowed_pairs: [(&[u8], &[u8]); 1] = [(b"k", b"v")];
        let str_pairs = [("k", "v")];

        // Act
        let encodings = [
            encode_pairs(&owned_pairs).unwrap(),
            encode_pairs(&borrowed_pairs).unwrap(),
            encode_pairs(&str_pairs).unwrap(),
        ];

        // Assert
        assert_eq!(encodings[0], encodings[1]);
        assert_eq!(encodings[1], encodings[2]);
    }

    #[test]
    fn encode_visited_matches_encode_pairs() {
        // Arrange
        let pairs: [(&[u8], &[u8]); 3] = [(b"a", b"1"), (b"", b""), (b"c", b"333")];
        let expected = encode_pairs(&pairs).unwrap();

        // Act
        let encoded = encode_visited(&mut |visitor| {
            for (key, value) in pairs {
                visitor(key, value)?;
            }
            ControlFlow::Continue(())
        });

        // Assert
        assert_eq!(encoded, Ok(expected));
    }

    #[test]
    fn encode_visited_reports_a_source_that_changes() {
        // Arrange
        let mut walk = 0;

        // Act
        let encoded = encode_visited(&mut |visitor| {
            walk += 1;
            let value: &[u8] = if walk == 1 { b"long value" } else { b"short" };
            visitor(b"k", value)
        });

        // Assert
        assert_eq!(encoded, Err(EncodeError::Changed));
    }

    #[test]
    fn both_empty_encodings_decode_to_an_empty_map() {
        // Arrange
        let inputs: [&[u8]; 2] = [&[], &[0x00]];

        // Act
        let decoded: Vec<_> = inputs
            .iter()
            .map(|input| decode_pairs(input, PairLimits::unlimited()))
            .collect();

        // Assert
        assert_eq!(decoded, vec![Ok(Vec::new()), Ok(Vec::new())]);
    }

    #[test]
    fn short_inputs_are_truncated_count() {
        // Arrange
        let inputs: [&[u8]; 3] = [&[0x01], &[0x00, 0x00], &[0x2a, 0x2b]];

        // Act
        let decoded: Vec<_> = inputs
            .iter()
            .map(|input| decode_pairs(input, PairLimits::unlimited()))
            .collect();

        // Assert
        assert_eq!(decoded, vec![Err(DecodeError::TruncatedCount); 3]);
    }

    #[test]
    fn short_length_table_is_truncated_lengths() {
        // Arrange
        let input = [3, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0];

        // Act
        let decoded = decode_pairs(&input, PairLimits::unlimited());

        // Assert
        assert_eq!(decoded, Err(DecodeError::TruncatedLengths { pairs: 3 }));
    }

    #[test]
    fn key_length_past_the_end_is_truncated_key() {
        // Arrange
        let input = [1, 0, 0, 0, 9, 0, 0, 0, 0, 0, 0, 0, b'a', 0, 0];

        // Act
        let decoded = decode_pairs(&input, PairLimits::unlimited());

        // Assert
        assert_eq!(decoded, Err(DecodeError::TruncatedKey { pair: 0 }));
    }

    #[test]
    fn value_length_past_the_end_is_truncated_value() {
        // Arrange
        // The Go common.DecodeMap bounds checks the value against the key
        // length and accepts this input.
        let input = [1, 0, 0, 0, 1, 0, 0, 0, 9, 0, 0, 0, b'a', 0, b'1', 0];

        // Act
        let decoded = decode_pairs(&input, PairLimits::unlimited());

        // Assert
        assert_eq!(decoded, Err(DecodeError::TruncatedValue { pair: 0 }));
    }

    #[test]
    fn key_terminator_that_is_not_nul_is_rejected() {
        // Arrange
        // The byte 0x30 is the ASCII character "0" that the Go common.EncodeMap
        // writes as a terminator.
        // That function is dead code in the Go host.
        // Its output shape is the negative fixture here.
        let input = [1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, b'a', 0x30, b'1', 0x30];

        // Act
        let decoded = decode_pairs(&input, PairLimits::unlimited());

        // Assert
        assert_eq!(
            decoded,
            Err(DecodeError::MissingTerminator {
                pair: 0,
                field: Field::Key
            })
        );
    }

    #[test]
    fn value_terminator_that_is_not_nul_is_rejected() {
        // Arrange
        let input = [1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, b'a', 0x00, b'1', 0x30];

        // Act
        let decoded = decode_pairs(&input, PairLimits::unlimited());

        // Assert
        assert_eq!(
            decoded,
            Err(DecodeError::MissingTerminator {
                pair: 0,
                field: Field::Value
            })
        );
    }

    #[test]
    fn bytes_after_the_last_pair_are_rejected() {
        // Arrange
        let mut input = encode_pairs(&[(b"a", b"1")]).unwrap();
        input.push(0xff);

        // Act
        let decoded = decode_pairs(&input, PairLimits::unlimited());

        // Assert
        assert_eq!(decoded, Err(DecodeError::TrailingBytes { count: 1 }));
    }

    #[test]
    fn maximum_key_length_is_truncated_key_without_panic() {
        // Arrange
        let mut input = vec![1, 0, 0, 0];
        input.extend_from_slice(&u32::MAX.to_le_bytes());
        input.extend_from_slice(&[0, 0, 0, 0, b'a', 0, 0]);

        // Act
        let decoded = decode_pairs(&input, PairLimits::unlimited());

        // Assert
        assert_eq!(decoded, Err(DecodeError::TruncatedKey { pair: 0 }));
    }

    #[test]
    fn maximum_count_without_a_table_is_truncated_lengths() {
        // Arrange
        let input = u32::MAX.to_le_bytes();

        // Act
        let decoded = decode_pairs(&input, PairLimits::unlimited());

        // Assert
        assert_eq!(
            decoded,
            Err(DecodeError::TruncatedLengths { pairs: u32::MAX })
        );
    }

    #[test]
    fn errors_display_their_messages() {
        // Arrange
        let errors: [&dyn std::fmt::Display; 4] = [
            &Field::Key,
            &Field::Value,
            &DecodeError::MissingTerminator {
                pair: 2,
                field: Field::Value,
            },
            &EncodeError::TooManyPairs { count: 7 },
        ];

        // Act
        let texts: Vec<String> = errors.iter().map(ToString::to_string).collect();

        // Assert
        assert_eq!(
            texts,
            vec![
                "key",
                "value",
                "pair 2 value is not terminated by 0x00",
                "7 pairs exceeds u32::MAX",
            ]
        );
    }

    #[test]
    fn a_last_value_with_no_terminator_is_truncated_value() {
        // Arrange
        // One pair of one byte each, where the data ends with the value and
        // the terminator of that value is absent.
        let input = [1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, b'a', 0, b'1'];

        // Act
        let decoded = decode_pairs(&input, PairLimits::unlimited());

        // Assert
        assert_eq!(decoded, Err(DecodeError::TruncatedValue { pair: 0 }));
    }

    #[test]
    fn a_last_key_with_no_terminator_is_truncated_key() {
        // Arrange
        let input = [1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, b'a'];

        // Act
        let decoded = decode_pairs(&input, PairLimits::unlimited());

        // Assert
        assert_eq!(decoded, Err(DecodeError::TruncatedKey { pair: 0 }));
    }
}
