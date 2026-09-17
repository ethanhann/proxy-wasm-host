//! The buffer that a guest reads and writes through the buffer host functions.
//!
//! When a guest asks for a request body or a configuration blob, the host
//! functions call the [`Buffer`] that you supply.
//! The trait asks only for a length, a range copy, and a range replace.
//! You can implement it on a chain of segments as well as on a `Vec<u8>`.

use std::ops::Range;

use crate::NotAllowed;

/// A byte buffer that the guest reads by range and writes by range.
///
/// `Vec<u8>` implements this trait, so a test or a simple embedder can lend a
/// plain vector.
/// The crate reads [`Buffer::len`] and clamps the range with [`clamp_range`]
/// before it calls you, so a `start` and a length you receive are inside the
/// buffer even when the guest asked for the largest range the ABI allows.
pub trait Buffer {
    /// The number of bytes in the buffer.
    fn len(&self) -> usize;

    /// Whether the buffer has no bytes.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Appends up to `max_size` bytes that start at `start` to `out`.
    ///
    /// The crate clamps the range before it calls this, so `start` and
    /// `max_size` are inside the buffer.
    /// If you call it yourself with a range you did not compute, use
    /// [`clamp_range`], which cuts a range that runs past the end.
    fn copy_range_into(&self, start: usize, max_size: usize, out: &mut Vec<u8>);

    /// Copies up to `max_size` bytes that start at `start`.
    ///
    /// This is [`Buffer::copy_range_into`] with a new vector.
    fn copy_range(&self, start: usize, max_size: usize) -> Vec<u8> {
        let mut out = Vec::new();
        self.copy_range_into(start, max_size, &mut out);
        out
    }

    /// Replaces `size` bytes that start at `start` with `value`.
    ///
    /// The ABI defines four operations through this one call.
    /// `start` of zero and `size` of zero prepends.
    /// A `start` at or past the end appends.
    /// A `size` of zero inside the buffer injects.
    /// Any other range replaces the bytes in it.
    /// The crate clamps the range before it calls this, so an append arrives
    /// as an empty range at the end.
    /// If you call it yourself with a range you did not compute, use
    /// [`clamp_range`], which cuts a range that runs past the end.
    ///
    /// # Errors
    ///
    /// Returns [`NotAllowed`] when you refuse the write.
    /// The guest then reads `NOT_FOUND`, the status the ABI lists for a
    /// buffer that is not available.
    /// A refused write to a header map reads `BAD_ARGUMENT` instead, because
    /// that is the status the ABI lists for a map.
    fn replace(&mut self, start: usize, size: usize, value: &[u8]) -> Result<(), NotAllowed>;
}

/// The range of a buffer of `len` bytes that `start` and `size` select.
///
/// `start` is clamped to `len`.
/// The end is `start` plus `size`, saturating, and then clamped to `len`.
/// Use this in your own [`Buffer`] implementation so that it follows the same
/// rule as the one for `Vec<u8>`.
pub fn clamp_range(len: usize, start: usize, size: usize) -> Range<usize> {
    let start = start.min(len);
    let end = start.saturating_add(size).min(len);
    start..end
}

/// The buffer implementation for a plain vector.
///
/// The inherent `Vec::len` and `Vec::is_empty` shadow the trait methods when
/// you call them on a `Vec<u8>` directly.
/// Call `Buffer::len(&vec)` to reach the trait method, or use the vector as a
/// `&dyn Buffer`.
impl Buffer for Vec<u8> {
    fn len(&self) -> usize {
        Vec::len(self)
    }

    fn copy_range_into(&self, start: usize, max_size: usize, out: &mut Vec<u8>) {
        out.extend_from_slice(&self[clamp_range(Vec::len(self), start, max_size)]);
    }

    fn replace(&mut self, start: usize, size: usize, value: &[u8]) -> Result<(), NotAllowed> {
        let range = clamp_range(Vec::len(self), start, size);
        self.splice(range, value.iter().copied());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_range_clamps_start_and_saturates_the_end() {
        // Arrange
        let cases = [
            (10, 0, 3),
            (10, 4, 2),
            (10, 7, 100),
            (10, 10, 1),
            (10, 50, 5),
            (10, 3, usize::MAX),
            (0, 0, 0),
        ];

        // Act
        let ranges: Vec<_> = cases
            .iter()
            .map(|&(len, start, size)| clamp_range(len, start, size))
            .collect();

        // Assert
        assert_eq!(ranges, vec![0..3, 4..6, 7..10, 10..10, 10..10, 3..10, 0..0]);
    }

    #[test]
    fn copy_range_clamps_to_the_buffer() {
        // Arrange
        let buffer = b"0123456789".to_vec();
        let ranges = [(0, 3), (4, 2), (7, 100), (10, 1), (50, 5)];

        // Act
        let copies: Vec<Vec<u8>> = ranges
            .iter()
            .map(|&(s, n)| buffer.copy_range(s, n))
            .collect();

        // Assert
        assert_eq!(
            copies,
            vec![
                b"012".to_vec(),
                b"45".to_vec(),
                b"789".to_vec(),
                Vec::new(),
                Vec::new()
            ]
        );
    }

    #[test]
    fn copy_range_into_appends_to_the_output() {
        // Arrange
        let buffer = b"abcdef".to_vec();
        let mut out = b"<".to_vec();

        // Act
        buffer.copy_range_into(1, 3, &mut out);

        // Assert
        assert_eq!(out, b"<bcd");
    }

    #[test]
    fn copy_range_on_an_empty_buffer_is_empty() {
        // Arrange
        let buffer: Vec<u8> = Vec::new();

        // Act
        let copy = buffer.copy_range(0, 10);

        // Assert
        assert!(copy.is_empty());
    }

    #[test]
    fn replace_prepends_with_zero_start_and_size() {
        // Arrange
        let mut buffer = b"world".to_vec();

        // Act
        let result = buffer.replace(0, 0, b"hello ");

        // Assert
        assert_eq!(result, Ok(()));
        assert_eq!(buffer, b"hello world");
    }

    #[test]
    fn replace_appends_when_start_is_at_or_past_the_end() {
        // Arrange
        let mut at_end = b"ab".to_vec();
        let mut past_end = b"ab".to_vec();

        // Act
        let results = [at_end.replace(2, 0, b"c"), past_end.replace(99, 5, b"c")];

        // Assert
        assert_eq!(results, [Ok(()), Ok(())]);
        assert_eq!(at_end, b"abc");
        assert_eq!(past_end, b"abc");
    }

    #[test]
    fn replace_appends_with_maximum_sizes() {
        // Arrange
        let mut abi_boundary = b"ab".to_vec();
        let mut saturation = b"ab".to_vec();
        let u32_max = usize::try_from(u32::MAX).unwrap();

        // Act
        let results = [
            abi_boundary.replace(5, u32_max, b"c"),
            saturation.replace(5, usize::MAX, b"c"),
        ];

        // Assert
        assert_eq!(results, [Ok(()), Ok(())]);
        assert_eq!(abi_boundary, b"abc");
        assert_eq!(saturation, b"abc");
    }

    #[test]
    fn replace_injects_with_zero_size_inside_the_buffer() {
        // Arrange
        let mut buffer = b"ac".to_vec();

        // Act
        let result = buffer.replace(1, 0, b"b");

        // Assert
        assert_eq!(result, Ok(()));
        assert_eq!(buffer, b"abc");
    }

    #[test]
    fn replace_swaps_a_middle_range() {
        // Arrange
        let mut buffer = b"0123456789".to_vec();

        // Act
        let result = buffer.replace(3, 4, b"xy");

        // Assert
        assert_eq!(result, Ok(()));
        assert_eq!(buffer, b"012xy789");
    }

    #[test]
    fn replace_clamps_a_size_past_the_end() {
        // Arrange
        let mut buffer = b"0123456789".to_vec();

        // Act
        let result = buffer.replace(8, 100, b"z");

        // Assert
        assert_eq!(result, Ok(()));
        assert_eq!(buffer, b"01234567z");
    }

    #[test]
    fn replace_swaps_the_whole_buffer() {
        // Arrange
        let mut buffer = b"old".to_vec();

        // Act
        let result = buffer.replace(0, 3, b"new value");

        // Assert
        assert_eq!(result, Ok(()));
        assert_eq!(buffer, b"new value");
    }

    #[test]
    fn replace_on_an_empty_buffer_inserts() {
        // Arrange
        let mut buffer: Vec<u8> = Vec::new();

        // Act
        let result = buffer.replace(3, 3, b"x");

        // Assert
        assert_eq!(result, Ok(()));
        assert_eq!(buffer, b"x");
    }

    #[test]
    fn len_and_is_empty_follow_the_vector() {
        // Arrange
        let empty: Vec<u8> = Vec::new();
        let full = b"abc".to_vec();

        // Act
        let observed = [
            (Buffer::len(&empty), Buffer::is_empty(&empty)),
            (Buffer::len(&full), Buffer::is_empty(&full)),
        ];

        // Assert
        assert_eq!(observed, [(0, true), (3, false)]);
    }

    #[test]
    fn vector_works_as_a_trait_object() {
        // Arrange
        let mut buffer: Box<dyn Buffer> = Box::new(b"abc".to_vec());

        // Act
        let observed = (
            buffer.replace(3, 0, b"d"),
            buffer.len(),
            buffer.copy_range(0, 4),
        );

        // Assert
        assert_eq!(observed, (Ok(()), 4, b"abcd".to_vec()));
    }
}
