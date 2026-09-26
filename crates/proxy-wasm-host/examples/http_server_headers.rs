//! The request headers of the `http_server` example, kept as `tiny_http`
//! keeps them.
//!
//! The crate reads and writes headers through the `HeaderMap` trait, so a
//! proxy lends the header type it already has rather than a copy.
//! This map holds the three pseudo headers the ABI expects beside the
//! `tiny_http` headers of the request, and it gives the `tiny_http` headers
//! back for the answer without a second conversion.

use std::borrow::Cow;
use std::ops::ControlFlow;

use proxy_wasm_host::{HeaderMap, NotAllowed, PairVisitor};
use tiny_http::Header;

/// The headers of one request, as the guest sees them.
#[derive(Default)]
pub struct RequestHeaders {
    pseudo: Vec<(Vec<u8>, Vec<u8>)>,
    fields: Vec<Header>,
}

impl RequestHeaders {
    /// A map with the three pseudo headers first and `fields` after them.
    pub fn new(method: &str, path: &str, authority: &str, fields: Vec<Header>) -> Self {
        Self {
            pseudo: vec![
                (b":method".to_vec(), method.into()),
                (b":path".to_vec(), path.into()),
                (b":authority".to_vec(), authority.into()),
            ],
            fields,
        }
    }

    /// The `tiny_http` headers, which the answer can send as they are.
    pub fn fields(&self) -> &[Header] {
        &self.fields
    }
}

fn named(header: &Header, key: &[u8]) -> bool {
    header.field.as_str().as_bytes().eq_ignore_ascii_case(key)
}

fn field(key: &[u8], value: &[u8]) -> Result<Header, NotAllowed> {
    Header::from_bytes(key, value).map_err(|()| NotAllowed)
}

impl HeaderMap for RequestHeaders {
    fn get(&self, key: &[u8]) -> Option<Cow<'_, [u8]>> {
        if let Some((_, value)) = self.pseudo.iter().find(|(name, _)| name == key) {
            return Some(Cow::Borrowed(value));
        }
        self.fields
            .iter()
            .find(|header| named(header, key))
            .map(|header| Cow::Borrowed(header.value.as_bytes()))
    }

    fn for_each_pair(&self, f: &mut PairVisitor<'_>) -> ControlFlow<()> {
        for (name, value) in &self.pseudo {
            f(name, value)?;
        }
        for header in &self.fields {
            f(header.field.as_str().as_bytes(), header.value.as_bytes())?;
        }
        ControlFlow::Continue(())
    }

    fn set(&mut self, key: &[u8], value: &[u8]) -> Result<(), NotAllowed> {
        if key.starts_with(b":") {
            return Err(NotAllowed);
        }
        let header = field(key, value)?;
        self.fields.retain(|existing| !named(existing, key));
        self.fields.push(header);
        Ok(())
    }

    fn add(&mut self, key: &[u8], value: &[u8]) -> Result<(), NotAllowed> {
        if key.starts_with(b":") {
            return Err(NotAllowed);
        }
        self.fields.push(field(key, value)?);
        Ok(())
    }

    fn remove(&mut self, key: &[u8]) -> Result<(), NotAllowed> {
        if key.starts_with(b":") {
            return Err(NotAllowed);
        }
        self.fields.retain(|existing| !named(existing, key));
        Ok(())
    }

    fn replace_all(&mut self, pairs: &[(&[u8], &[u8])]) -> Result<(), NotAllowed> {
        let mut fields = Vec::with_capacity(pairs.len());
        for (key, value) in pairs {
            if key.starts_with(b":") {
                return Err(NotAllowed);
            }
            fields.push(field(key, value)?);
        }
        self.fields = fields;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use proxy_wasm_host::HeaderMapExt;

    use super::*;

    fn map() -> RequestHeaders {
        let fields = vec![
            "Host: example".parse().unwrap(),
            "Accept: text/plain".parse().unwrap(),
        ];
        RequestHeaders::new("GET", "/a", "example:80", fields)
    }

    fn names(map: &RequestHeaders) -> Vec<String> {
        map.pairs()
            .into_iter()
            .map(|(name, _)| String::from_utf8(name).unwrap())
            .collect()
    }

    #[test]
    fn the_pseudo_headers_come_first_and_a_lookup_ignores_the_case() {
        // Arrange
        let map = map();

        // Act
        let observed = (
            names(&map),
            map.get(b":path").map(Cow::into_owned),
            map.get(b"ACCEPT").map(Cow::into_owned),
        );

        // Assert
        assert_eq!(
            observed.0,
            [":method", ":path", ":authority", "Host", "Accept"]
        );
        assert_eq!(observed.1, Some(b"/a".to_vec()));
        assert_eq!(observed.2, Some(b"text/plain".to_vec()));
    }

    #[test]
    fn a_write_changes_the_fields_and_leaves_the_pseudo_headers() {
        // Arrange
        let mut map = map();

        // Act
        let results = (
            map.set(b"accept", b"*/*"),
            map.add(b"x-added", b"1"),
            map.remove(b"host"),
            map.set(b":path", b"/b"),
        );

        // Assert
        assert_eq!(results, (Ok(()), Ok(()), Ok(()), Err(NotAllowed)));
        assert_eq!(
            names(&map),
            [":method", ":path", ":authority", "accept", "x-added"]
        );
        assert_eq!(
            map.get(b"accept").map(Cow::into_owned),
            Some(b"*/*".to_vec())
        );
        assert_eq!(map.fields().len(), 2, "the answer carries the two fields");
    }

    #[test]
    fn replace_all_keeps_the_pseudo_headers_and_refuses_a_pseudo_pair() {
        // Arrange
        let mut map = map();

        // Act
        let results = (
            map.replace_all(&[(b"a", b"1")]),
            map.replace_all(&[(b":method", b"POST")]),
        );

        // Assert
        assert_eq!(results, (Ok(()), Err(NotAllowed)));
        assert_eq!(names(&map), [":method", ":path", ":authority", "a"]);
    }
}
