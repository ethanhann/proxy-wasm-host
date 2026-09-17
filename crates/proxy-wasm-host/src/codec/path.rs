//! Serialization of a property path.
//!
//! A property path is a sequence of segments separated by `0x00` bytes.
//! The `proxy_get_property` and `proxy_set_property` host functions expect
//! this form.
//! The ABI document asks a host to tolerate one `0x00` byte at the end of the
//! path, and [`decode_path`] does.

const SEPARATOR: u8 = 0;

/// Joins segments with `0x00` and adds no trailing byte.
///
/// This function and [`decode_path`] are not inverses when the last segment
/// is empty, because the decoder drops one trailing `0x00`.
/// `[""]` encodes to no bytes.
/// `["", ""]` encodes to one `0x00` byte.
/// Both decode to no segments.
pub fn encode_path<S: AsRef<[u8]>>(segments: &[S]) -> Vec<u8> {
    segments
        .iter()
        .map(AsRef::as_ref)
        .collect::<Vec<_>>()
        .join(&SEPARATOR)
}

/// Splits a path on `0x00` into segments that borrow from `data`.
///
/// One trailing `0x00` is dropped before the split.
/// It does not produce an empty final segment.
/// No bytes decode to no segments.
/// An empty segment between two separators is kept.
///
/// This function and [`encode_path`] are not inverses when the last segment
/// is empty.
/// The encodings of `[""]` and `["", ""]` both decode to no segments.
pub fn decode_path(data: &[u8]) -> Vec<&[u8]> {
    let trimmed = data.strip_suffix(&[SEPARATOR]).unwrap_or(data);
    if trimmed.is_empty() {
        return Vec::new();
    }
    trimmed.split(|&byte| byte == SEPARATOR).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_example_encodes_without_a_trailing_byte() {
        // Arrange
        let segments = ["foo", "bar"];

        // Act
        let encoded = encode_path(&segments);

        // Assert
        assert_eq!(encoded, [0x66, 0x6f, 0x6f, 0x00, 0x62, 0x61, 0x72]);
    }

    #[test]
    fn segments_round_trip() {
        // Arrange
        let paths: Vec<Vec<&[u8]>> = vec![
            vec![],
            vec![b"one"],
            vec![b"a", b"b", b"c"],
            vec![b"a", b"", b"c"],
        ];

        // Act
        let round_trips: Vec<Vec<Vec<u8>>> = paths
            .iter()
            .map(|path| {
                decode_path(&encode_path(path))
                    .iter()
                    .map(|s| s.to_vec())
                    .collect()
            })
            .collect();

        // Assert
        let expected: Vec<Vec<Vec<u8>>> = paths
            .iter()
            .map(|path| path.iter().map(|s| s.to_vec()).collect())
            .collect();
        assert_eq!(round_trips, expected);
    }

    #[test]
    fn one_trailing_nul_is_tolerated() {
        // Arrange
        let with_nul = b"foo\x00bar\x00";
        let without_nul = b"foo\x00bar";

        // Act
        let decoded = decode_path(with_nul);

        // Assert
        assert_eq!(decoded, decode_path(without_nul));
        assert_eq!(decoded, [b"foo".as_slice(), b"bar".as_slice()]);
    }

    #[test]
    fn two_trailing_nuls_leave_one_empty_segment() {
        // Arrange
        let data = b"foo\x00\x00";

        // Act
        let decoded = decode_path(data);

        // Assert
        assert_eq!(decoded, [b"foo".as_slice(), b"".as_slice()]);
    }

    #[test]
    fn empty_input_decodes_to_no_segments() {
        // Arrange
        let data: &[u8] = &[];

        // Act
        let decoded = decode_path(data);

        // Assert
        assert!(decoded.is_empty());
    }

    #[test]
    fn trailing_empty_segments_do_not_round_trip() {
        // Arrange
        let one_empty: [&[u8]; 1] = [b""];
        let two_empty: [&[u8]; 2] = [b"", b""];

        // Act
        let encodings = [encode_path(&one_empty), encode_path(&two_empty)];

        // Assert
        assert_eq!(encodings[0], Vec::<u8>::new());
        assert_eq!(encodings[1], vec![0x00]);
        assert!(decode_path(&encodings[0]).is_empty());
        assert!(decode_path(&encodings[1]).is_empty());
    }
}
