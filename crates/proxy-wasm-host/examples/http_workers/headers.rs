//! The request headers, kept the way an HTTP proxy keeps them.

use std::borrow::Cow;
use std::ops::ControlFlow;

use proxy_wasm_host::codec::pairs::PairVisitor;
use proxy_wasm_host::{HeaderMap, NotAllowed};

/// Request headers with the rules of a proxy: names are stored in lower case,
/// compared without regard to case, and a replaced header moves to the end.
///
/// The crate's own `VecHeaderMap` keeps names exactly as they were written,
/// which suits a host that wants to show the guest what it sent. A proxy
/// usually follows HTTP/2 and lowers every name, so this example does too.
#[derive(Debug, Default)]
pub struct ProxyHeaders(Vec<(Vec<u8>, Vec<u8>)>);

impl ProxyHeaders {
    /// How many pairs the headers hold.
    pub fn len(&self) -> usize {
        self.0.len()
    }
}

impl From<Vec<(Vec<u8>, Vec<u8>)>> for ProxyHeaders {
    fn from(pairs: Vec<(Vec<u8>, Vec<u8>)>) -> Self {
        Self(
            pairs
                .into_iter()
                .map(|(name, value)| (name.to_ascii_lowercase(), value))
                .collect(),
        )
    }
}

impl HeaderMap for ProxyHeaders {
    fn get(&self, key: &[u8]) -> Option<Cow<'_, [u8]>> {
        self.0
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(key))
            .map(|(_, value)| Cow::Borrowed(value.as_slice()))
    }

    fn for_each_pair(&self, f: &mut PairVisitor<'_>) -> ControlFlow<()> {
        for (name, value) in &self.0 {
            f(name, value)?;
        }
        ControlFlow::Continue(())
    }

    fn set(&mut self, key: &[u8], value: &[u8]) -> Result<(), NotAllowed> {
        self.0.retain(|(name, _)| !name.eq_ignore_ascii_case(key));
        self.0.push((key.to_ascii_lowercase(), value.to_vec()));
        Ok(())
    }

    fn add(&mut self, key: &[u8], value: &[u8]) -> Result<(), NotAllowed> {
        self.0.push((key.to_ascii_lowercase(), value.to_vec()));
        Ok(())
    }

    fn remove(&mut self, key: &[u8]) -> Result<(), NotAllowed> {
        self.0.retain(|(name, _)| !name.eq_ignore_ascii_case(key));
        Ok(())
    }

    fn replace_all(&mut self, pairs: &[(&[u8], &[u8])]) -> Result<(), NotAllowed> {
        self.0 = pairs
            .iter()
            .map(|(name, value)| (name.to_ascii_lowercase(), value.to_vec()))
            .collect();
        Ok(())
    }
}
