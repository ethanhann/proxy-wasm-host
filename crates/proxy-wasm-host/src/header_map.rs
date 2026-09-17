//! The header map that a guest reads and writes through the header map host
//! functions.
//!
//! When a guest asks for a request header, the host functions call the
//! [`HeaderMap`] that you supply.
//! The trait asks for a lookup, a visitor over the pairs, and four writes.
//! The other methods are provided over the visitor.
//! You can therefore implement the trait on a map that computes its keys as
//! well as on stored pairs.

use std::borrow::Cow;
use std::ops::ControlFlow;

use crate::NotAllowed;
use crate::codec::pairs::{
    EncodeError, PairVisitor, encode_visited, pair_encoded_size, total_size,
};

/// An ordered multimap of byte string keys and values.
///
/// [`VecHeaderMap`] is the implementation the crate ships.
/// Its rustdoc states how it compares keys and where `set` places a value.
pub trait HeaderMap {
    /// The first value for `key`, or `None` when the key is absent.
    ///
    /// A map that stores its values returns a borrow.
    /// A map that computes them returns an owned value.
    fn get(&self, key: &[u8]) -> Option<Cow<'_, [u8]>>;

    /// Calls `f` with every pair in order until `f` breaks.
    ///
    /// The return value is the last value `f` returned, or `Continue` when
    /// the map has no pairs.
    fn for_each_pair(&self, f: &mut PairVisitor<'_>) -> ControlFlow<()>;

    /// Leaves exactly one pair for `key`, with `value`.
    ///
    /// Other pairs keep their relative order.
    ///
    /// # Errors
    ///
    /// Returns [`NotAllowed`] when the embedder refuses the write.
    fn set(&mut self, key: &[u8], value: &[u8]) -> Result<(), NotAllowed>;

    /// Appends a pair, even when `key` already has a value.
    ///
    /// # Errors
    ///
    /// Returns [`NotAllowed`] when the embedder refuses the write.
    fn add(&mut self, key: &[u8], value: &[u8]) -> Result<(), NotAllowed>;

    /// Removes every pair for `key`, and succeeds when there is none.
    ///
    /// # Errors
    ///
    /// Returns [`NotAllowed`] when the embedder refuses the write.
    fn remove(&mut self, key: &[u8]) -> Result<(), NotAllowed>;

    /// Discards every pair and takes `pairs` in order.
    ///
    /// # Errors
    ///
    /// Returns [`NotAllowed`] when the embedder refuses the write.
    fn replace_all(&mut self, pairs: &[(&[u8], &[u8])]) -> Result<(), NotAllowed>;

    /// Every value for `key` in order.
    fn get_all(&self, key: &[u8]) -> Vec<Vec<u8>> {
        let mut values = Vec::new();
        let _ = self.for_each_pair(&mut |k, v| {
            if k == key {
                values.push(v.to_vec());
            }
            ControlFlow::Continue(())
        });
        values
    }

    /// Every value for `key` joined by `separator`, or `None` when the key is
    /// absent.
    fn get_joined(&self, key: &[u8], separator: &[u8]) -> Option<Vec<u8>> {
        let values = self.get_all(key);
        if values.is_empty() {
            None
        } else {
            Some(values.join(separator))
        }
    }

    /// Every pair in order, as owned bytes.
    fn pairs(&self) -> Vec<(Vec<u8>, Vec<u8>)> {
        let mut pairs = Vec::new();
        let _ = self.for_each_pair(&mut |k, v| {
            pairs.push((k.to_vec(), v.to_vec()));
            ControlFlow::Continue(())
        });
        pairs
    }

    /// The number of pairs.
    fn len(&self) -> usize {
        let mut count = 0;
        let _ = self.for_each_pair(&mut |_, _| {
            count += 1;
            ControlFlow::Continue(())
        });
        count
    }

    /// Whether the map has no pairs.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The size of the map when serialized with [`crate::codec::pairs`].
    ///
    /// An empty map has size zero.
    fn encoded_size(&self) -> usize {
        let mut count = 0usize;
        let mut pairs_size = 0usize;
        let _ = self.for_each_pair(&mut |k, v| {
            count += 1;
            pairs_size = pairs_size.saturating_add(pair_encoded_size(k.len(), v.len()));
            ControlFlow::Continue(())
        });
        total_size(count, pairs_size)
    }

    /// Serializes the map with [`crate::codec::pairs`], without an owned
    /// copy of any pair.
    ///
    /// # Errors
    ///
    /// Returns [`EncodeError`] when a key or a value is longer than `u32::MAX`
    /// bytes, when there are more than `u32::MAX` pairs, or when the map
    /// changed while it was being walked.
    fn encode(&self) -> Result<Vec<u8>, EncodeError> {
        encode_visited(&mut |visitor| self.for_each_pair(visitor))
    }
}

/// A header map stored as a vector of pairs in insertion order.
///
/// Keys are compared as exact bytes, so `Host` and `host` are different keys.
/// If you wrap an HTTP library's header type instead, you decide case folding
/// in your own implementation.
/// `set` puts its single pair at the position of the first pair it removed.
/// When the key was absent, `set` appends.
/// `len` reads the vector length directly instead of walking the pairs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VecHeaderMap {
    pairs: Vec<(Vec<u8>, Vec<u8>)>,
}

impl HeaderMap for VecHeaderMap {
    fn get(&self, key: &[u8]) -> Option<Cow<'_, [u8]>> {
        self.pairs
            .iter()
            .find(|(k, _)| k.as_slice() == key)
            .map(|(_, v)| Cow::Borrowed(v.as_slice()))
    }

    fn for_each_pair(&self, f: &mut PairVisitor<'_>) -> ControlFlow<()> {
        for (k, v) in &self.pairs {
            f(k, v)?;
        }
        ControlFlow::Continue(())
    }

    fn set(&mut self, key: &[u8], value: &[u8]) -> Result<(), NotAllowed> {
        let mut kept_one = false;
        self.pairs.retain_mut(|(k, v)| {
            if k.as_slice() != key {
                return true;
            }
            if kept_one {
                return false;
            }
            kept_one = true;
            *v = value.to_vec();
            true
        });
        if !kept_one {
            self.pairs.push((key.to_vec(), value.to_vec()));
        }
        Ok(())
    }

    fn add(&mut self, key: &[u8], value: &[u8]) -> Result<(), NotAllowed> {
        self.pairs.push((key.to_vec(), value.to_vec()));
        Ok(())
    }

    fn remove(&mut self, key: &[u8]) -> Result<(), NotAllowed> {
        self.pairs.retain(|(k, _)| k.as_slice() != key);
        Ok(())
    }

    fn replace_all(&mut self, pairs: &[(&[u8], &[u8])]) -> Result<(), NotAllowed> {
        self.pairs = pairs
            .iter()
            .map(|(k, v)| (k.to_vec(), v.to_vec()))
            .collect();
        Ok(())
    }

    fn len(&self) -> usize {
        self.pairs.len()
    }
}

impl FromIterator<(Vec<u8>, Vec<u8>)> for VecHeaderMap {
    fn from_iter<I: IntoIterator<Item = (Vec<u8>, Vec<u8>)>>(iter: I) -> Self {
        Self {
            pairs: iter.into_iter().collect(),
        }
    }
}

impl From<Vec<(Vec<u8>, Vec<u8>)>> for VecHeaderMap {
    fn from(pairs: Vec<(Vec<u8>, Vec<u8>)>) -> Self {
        Self { pairs }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::pairs::{encode_pairs, encoded_size};

    fn map(pairs: &[(&str, &str)]) -> VecHeaderMap {
        owned_str(pairs).into_iter().collect()
    }

    fn owned_str(pairs: &[(&str, &str)]) -> Vec<(Vec<u8>, Vec<u8>)> {
        pairs
            .iter()
            .map(|(k, v)| (k.as_bytes().to_vec(), v.as_bytes().to_vec()))
            .collect()
    }

    /// A map whose only pair exists nowhere in memory until a call asks for it.
    struct Computed;

    impl HeaderMap for Computed {
        fn get(&self, key: &[u8]) -> Option<Cow<'_, [u8]>> {
            (key == b"x-computed").then(|| Cow::Owned(format!("yes-{}", key.len()).into_bytes()))
        }

        fn for_each_pair(&self, f: &mut PairVisitor<'_>) -> ControlFlow<()> {
            let key = format!("x-{}", "computed").into_bytes();
            f(&key, b"yes-10")
        }

        fn set(&mut self, _: &[u8], _: &[u8]) -> Result<(), NotAllowed> {
            Err(NotAllowed)
        }

        fn add(&mut self, _: &[u8], _: &[u8]) -> Result<(), NotAllowed> {
            Err(NotAllowed)
        }

        fn remove(&mut self, _: &[u8]) -> Result<(), NotAllowed> {
            Err(NotAllowed)
        }

        fn replace_all(&mut self, _: &[(&[u8], &[u8])]) -> Result<(), NotAllowed> {
            Err(NotAllowed)
        }
    }

    #[test]
    fn get_returns_the_first_value() {
        // Arrange
        let map = map(&[("k", "first"), ("k", "second")]);

        // Act
        let values = [map.get(b"k"), map.get(b"missing")];

        // Assert
        assert_eq!(values, [Some(Cow::Borrowed(b"first".as_slice())), None]);
    }

    #[test]
    fn get_all_returns_every_value_in_order() {
        // Arrange
        let map = map(&[("k", "first"), ("other", "x"), ("k", "second")]);

        // Act
        let values = map.get_all(b"k");

        // Assert
        assert_eq!(values, vec![b"first".to_vec(), b"second".to_vec()]);
    }

    #[test]
    fn get_joined_joins_values_and_is_none_when_absent() {
        // Arrange
        let map = map(&[("k", "a"), ("k", "b")]);

        // Act
        let joined = [
            map.get_joined(b"k", b", "),
            map.get_joined(b"missing", b", "),
        ];

        // Assert
        assert_eq!(joined, [Some(b"a, b".to_vec()), None]);
    }

    #[test]
    fn add_keeps_the_existing_pair_and_appends() {
        // Arrange
        // The Go CommonHeader.Add panics.
        // This map appends instead.
        let mut map = map(&[("k", "first")]);

        // Act
        let result = map.add(b"k", b"second");

        // Assert
        assert_eq!(result, Ok(()));
        assert_eq!(map.pairs(), owned_str(&[("k", "first"), ("k", "second")]));
    }

    #[test]
    fn set_leaves_one_pair_at_the_first_position() {
        // Arrange
        let mut map = map(&[("a", "1"), ("k", "first"), ("b", "2"), ("k", "second")]);

        // Act
        let result = map.set(b"k", b"only");

        // Assert
        assert_eq!(result, Ok(()));
        assert_eq!(
            map.pairs(),
            owned_str(&[("a", "1"), ("k", "only"), ("b", "2")])
        );
    }

    #[test]
    fn set_on_a_missing_key_appends() {
        // Arrange
        let mut map = map(&[("a", "1")]);

        // Act
        let result = map.set(b"k", b"v");

        // Assert
        assert_eq!(result, Ok(()));
        assert_eq!(map.pairs(), owned_str(&[("a", "1"), ("k", "v")]));
    }

    #[test]
    fn remove_drops_every_pair_for_the_key() {
        // Arrange
        let mut map = map(&[("k", "1"), ("a", "x"), ("k", "2"), ("b", "y")]);

        // Act
        let result = map.remove(b"k");

        // Assert
        assert_eq!(result, Ok(()));
        assert_eq!(map.pairs(), owned_str(&[("a", "x"), ("b", "y")]));
    }

    #[test]
    fn remove_of_a_missing_key_is_ok_and_changes_nothing() {
        // Arrange
        let mut map = map(&[("a", "x")]);

        // Act
        let result = map.remove(b"missing");

        // Assert
        assert_eq!(result, Ok(()));
        assert_eq!(map.pairs(), owned_str(&[("a", "x")]));
    }

    #[test]
    fn replace_all_takes_the_new_pairs_in_order() {
        // Arrange
        let mut map = map(&[("old", "1")]);
        let new_pairs: [(&[u8], &[u8]); 2] = [(b"b", b"2"), (b"a", b"1")];

        // Act
        let result = map.replace_all(&new_pairs);

        // Assert
        assert_eq!(result, Ok(()));
        assert_eq!(map.pairs(), owned_str(&[("b", "2"), ("a", "1")]));
    }

    #[test]
    fn for_each_pair_visits_in_insertion_order_and_stops_on_break() {
        // Arrange
        let map = map(&[("z", "1"), ("a", "2"), ("m", "3")]);
        let mut visited = Vec::new();

        // Act
        let flow = map.for_each_pair(&mut |k, v| {
            visited.push((k.to_vec(), v.to_vec()));
            if visited.len() == 2 {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        });

        // Assert
        assert_eq!(flow, ControlFlow::Break(()));
        assert_eq!(visited, owned_str(&[("z", "1"), ("a", "2")]));
    }

    #[test]
    fn keys_that_differ_by_case_are_different_keys() {
        // Arrange
        let map = map(&[("Host", "upper")]);

        // Act
        let values = [map.get(b"host"), map.get(b"Host")];

        // Assert
        assert_eq!(values, [None, Some(Cow::Borrowed(b"upper".as_slice()))]);
    }

    #[test]
    fn len_is_empty_and_encoded_size_agree_with_pairs() {
        // Arrange
        let empty = VecHeaderMap::default();
        let full = map(&[("ab", "cde"), ("", "")]);
        let expected_full_size = encoded_size(&full.pairs());

        // Act
        let observed = [
            (empty.len(), empty.is_empty(), empty.encoded_size()),
            (full.len(), full.is_empty(), full.encoded_size()),
        ];

        // Assert
        assert_eq!(observed[0], (0, true, 0));
        assert_eq!(observed[1], (2, false, expected_full_size));
        assert_eq!(observed[1].2, 4 + (8 + 2 + 1 + 3 + 1) + (8 + 2));
    }

    #[test]
    fn encode_matches_the_codec() {
        // Arrange
        let map = map(&[("a", "1"), ("", "x"), ("a", "2")]);
        let expected = encode_pairs(&map.pairs()).unwrap();

        // Act
        let encoded = map.encode();

        // Assert
        assert_eq!(encoded, Ok(expected));
    }

    #[test]
    fn from_iterator_and_from_vec_build_the_same_map() {
        // Arrange
        let pairs = owned_str(&[("a", "1"), ("b", "2")]);

        // Act
        let built = [
            pairs.iter().cloned().collect::<VecHeaderMap>(),
            VecHeaderMap::from(pairs.clone()),
        ];

        // Assert
        assert_eq!(built[0], built[1]);
        assert_eq!(built[0].pairs(), pairs);
    }

    #[test]
    fn vec_header_map_works_as_a_trait_object() {
        // Arrange
        let mut map: Box<dyn HeaderMap> = Box::new(map(&[("a", "1")]));
        let mut visited = 0;

        // Act
        let observed = (
            map.set(b"b", b"2"),
            map.get(b"b").map(Cow::into_owned),
            map.for_each_pair(&mut |_, _| {
                visited += 1;
                ControlFlow::Continue(())
            }),
        );

        // Assert
        assert_eq!(
            observed,
            (Ok(()), Some(b"2".to_vec()), ControlFlow::Continue(()))
        );
        assert_eq!(visited, 2);
    }

    #[test]
    fn computed_map_implements_the_trait() {
        // Arrange
        let mut computed = Computed;

        // Act
        let observed = (
            computed.pairs(),
            computed.get(b"x-computed").map(Cow::into_owned),
            computed.set(b"k", b"v"),
        );

        // Assert
        assert_eq!(observed.0, owned_str(&[("x-computed", "yes-10")]));
        assert_eq!(observed.1, Some(b"yes-10".to_vec()));
        assert_eq!(observed.2, Err(NotAllowed));
        assert_eq!(
            computed.encode(),
            Ok(encode_pairs(&owned_str(&[("x-computed", "yes-10")])).unwrap())
        );
    }
}
