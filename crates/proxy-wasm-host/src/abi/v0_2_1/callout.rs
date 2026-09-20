//! The callouts a guest has open, and the identifiers that name them.

use std::collections::BTreeMap;
use std::fmt;
use std::num::NonZeroU32;

use crate::abi::v0_2_1::ContextId;

/// The identifier of one callout.
///
/// The crate gives one out when your [`Callouts`](crate::abi::v0_2_1::Callouts)
/// service accepts a callout, and you give it back when you deliver the
/// result.
/// One guest never has two open callouts with one identifier, whatever their
/// kinds.
/// Zero is never one, because the ABI uses it for an absent value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CalloutId(NonZeroU32);

impl CalloutId {
    /// The identifier as the ABI carries it.
    pub fn get(self) -> u32 {
        self.0.get()
    }
}

impl TryFrom<u32> for CalloutId {
    type Error = InvalidCalloutId;

    fn try_from(value: u32) -> Result<Self, InvalidCalloutId> {
        NonZeroU32::new(value)
            .map(Self)
            .ok_or(InvalidCalloutId { value })
    }
}

impl TryFrom<i32> for CalloutId {
    type Error = InvalidCalloutId;

    /// Reads the raw bits as unsigned, because the ABI types the argument as
    /// unsigned, and rejects zero.
    fn try_from(value: i32) -> Result<Self, InvalidCalloutId> {
        Self::try_from(value.cast_unsigned())
    }
}

impl fmt::Display for CalloutId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A value that cannot be a callout identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("{value} is not a valid callout identifier")]
pub struct InvalidCalloutId {
    /// The value that was rejected.
    pub value: u32,
}

/// What a callout asks the embedder for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CalloutKind {
    /// An HTTP call, which one response ends.
    HttpCall,
    /// A gRPC call, which sends one message and gets one answer.
    GrpcCall,
    /// A gRPC stream, which stays open for many messages.
    GrpcStream,
}

impl fmt::Display for CalloutKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HttpCall => f.write_str("an HTTP call"),
            Self::GrpcCall => f.write_str("a gRPC call"),
            Self::GrpcStream => f.write_str("a gRPC stream"),
        }
    }
}

/// Why a delivery for a callout was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CalloutProblem {
    /// No open callout has the identifier.
    NotOpen,
    /// Another context made the callout, and this is the context you named.
    NotMadeBy(ContextId),
    /// The response you gave arrived and has no header.
    /// The ABI reads a header count of zero as a call that failed, so give
    /// [`HttpCallResponse::failed`](crate::abi::v0_2_1::HttpCallResponse::failed)
    /// when that is what you mean.
    NoResponseHeader,
    /// The callout is of another kind than the delivery you gave, and this
    /// is the kind it has.
    WrongKind(CalloutKind),
}

impl fmt::Display for CalloutProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotOpen => f.write_str("is not open"),
            Self::NotMadeBy(context) => write!(f, "was not made by context {context}"),
            Self::NoResponseHeader => f.write_str("got a received response with no header"),
            Self::WrongKind(kind) => write!(f, "is {kind}"),
        }
    }
}

/// One callout that a guest has open, as [`Guest`](crate::abi::v0_2_1::Guest)
/// reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct OpenCallout {
    /// The identifier of the callout.
    pub callout: CalloutId,
    /// The context that made the call, which you name when you deliver the
    /// result.
    pub caller: ContextId,
    /// The root of the caller.
    /// When it is the caller, deliver under
    /// [`Guest::enter_root`](crate::abi::v0_2_1::Guest::enter_root).
    pub root: ContextId,
    /// What the callout asks for.
    pub kind: CalloutKind,
}

/// One open callout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Callout {
    pub(crate) kind: CalloutKind,
    /// The context that made the call.
    pub(crate) caller: ContextId,
    /// The root of the caller, which a callback names as the plugin context.
    pub(crate) root: ContextId,
    /// Whether the guest closed its side of a gRPC stream, which it does
    /// with `proxy_grpc_close` or with a send that ends the stream.
    pub(crate) closed_by_guest: bool,
}

impl Callout {
    pub(crate) fn new(kind: CalloutKind, caller: ContextId, root: ContextId) -> Self {
        Self {
            kind,
            caller,
            root,
            closed_by_guest: false,
        }
    }

    pub(crate) fn report(self, callout: CalloutId) -> OpenCallout {
        OpenCallout {
            callout,
            caller: self.caller,
            root: self.root,
            kind: self.kind,
        }
    }
}

/// The open callouts of one guest.
///
/// One counter serves every kind, so no two open callouts share an
/// identifier.
#[derive(Debug)]
pub(crate) struct CalloutTable {
    open: BTreeMap<CalloutId, Callout>,
    next: NonZeroU32,
}

impl CalloutTable {
    pub(crate) fn new() -> Self {
        Self {
            open: BTreeMap::new(),
            next: NonZeroU32::MIN,
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.open.len()
    }

    /// The next identifier that is not open, which is not entered yet.
    ///
    /// The counter moves forward, skips zero when it wraps, and skips an open
    /// identifier.
    /// It examines one identifier more than the table holds at most, and the
    /// table is far smaller than the space, so it finds one.
    pub(crate) fn reserve(&mut self) -> CalloutId {
        loop {
            let candidate = CalloutId(self.next);
            self.next = self.next.checked_add(1).unwrap_or(NonZeroU32::MIN);
            if !self.open.contains_key(&candidate) {
                return candidate;
            }
        }
    }

    pub(crate) fn enter(&mut self, id: CalloutId, callout: Callout) {
        self.open.insert(id, callout);
    }

    pub(crate) fn get(&self, id: CalloutId) -> Option<Callout> {
        self.open.get(&id).copied()
    }

    pub(crate) fn remove(&mut self, id: CalloutId) -> Option<Callout> {
        self.open.remove(&id)
    }

    /// Whether this table ever gave out the identifier.
    ///
    /// The counter moves forward, so every identifier below it was given to
    /// the guest.
    /// The answer is false for every identifier after the counter wraps,
    /// which needs about four billion callouts in one guest.
    pub(crate) fn issued(&self, id: CalloutId) -> bool {
        id.get() < self.next.get()
    }

    /// Records that the guest closed its side of a gRPC stream.
    pub(crate) fn close_by_guest(&mut self, id: CalloutId) {
        if let Some(callout) = self.open.get_mut(&id) {
            callout.closed_by_guest = true;
        }
    }

    /// The callouts `caller` made, in identifier order.
    pub(crate) fn made_by(&self, caller: ContextId) -> Vec<(CalloutId, CalloutKind)> {
        self.open
            .iter()
            .filter(|(_, callout)| callout.caller == caller)
            .map(|(id, callout)| (*id, callout.kind))
            .collect()
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = (CalloutId, Callout)> + '_ {
        self.open.iter().map(|(id, callout)| (*id, *callout))
    }

    #[cfg(test)]
    pub(crate) fn set_next(&mut self, next: u32) {
        self.next = NonZeroU32::new(next).unwrap_or(NonZeroU32::MIN);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context(value: u32) -> ContextId {
        ContextId::try_from(value).unwrap()
    }

    fn http(caller: u32, root: u32) -> Callout {
        Callout::new(CalloutKind::HttpCall, context(caller), context(root))
    }

    #[test]
    fn a_callout_identifier_reads_its_bits_as_unsigned_and_rejects_zero() {
        // Arrange
        let values = [1_i32, -1, 0];

        // Act
        let converted = values.map(CalloutId::try_from);

        // Assert
        assert_eq!(converted[0].map(CalloutId::get), Ok(1));
        assert_eq!(converted[1].map(CalloutId::get), Ok(u32::MAX));
        assert_eq!(converted[2], Err(InvalidCalloutId { value: 0 }));
        assert_eq!(
            InvalidCalloutId { value: 0 }.to_string(),
            "0 is not a valid callout identifier"
        );
    }

    #[test]
    fn a_reserved_identifier_that_is_not_entered_leaves_the_table_empty() {
        // Arrange
        let mut table = CalloutTable::new();

        // Act
        let reserved: Vec<u32> = (0..1000).map(|_| table.reserve().get()).collect();

        // Assert
        assert_eq!(table.len(), 0);
        assert_eq!(reserved.first(), Some(&1));
        assert_eq!(reserved.last(), Some(&1000));
    }

    #[test]
    fn the_counter_skips_zero_and_an_open_identifier_when_it_wraps() {
        // Arrange
        let mut table = CalloutTable::new();
        let one = table.reserve();
        table.enter(one, http(2, 1));
        table.set_next(u32::MAX);

        // Act
        let given: Vec<u32> = (0..2).map(|_| table.reserve().get()).collect();

        // Assert
        assert_eq!(given, [u32::MAX, 2], "zero and the open 1 are skipped");
    }

    #[test]
    fn made_by_lists_the_callouts_of_one_caller_in_order() {
        // Arrange
        let mut table = CalloutTable::new();
        for caller in [2, 3, 2] {
            let id = table.reserve();
            table.enter(id, http(caller, 1));
        }

        // Act
        let made: Vec<u32> = table
            .made_by(context(2))
            .into_iter()
            .map(|(id, _)| id.get())
            .collect();

        // Assert
        assert_eq!(made, [1, 3]);
        assert_eq!(table.made_by(context(9)), Vec::new());
    }

    #[test]
    fn a_removed_callout_is_gone_and_its_siblings_stay() {
        // Arrange
        let mut table = CalloutTable::new();
        let first = table.reserve();
        table.enter(first, http(2, 1));
        let second = table.reserve();
        table.enter(second, http(2, 1));

        // Act
        let removed = table.remove(first);

        // Assert
        assert_eq!(removed, Some(http(2, 1)));
        assert_eq!(table.get(first), None);
        assert_eq!(table.get(second), Some(http(2, 1)));
        assert_eq!(table.remove(first), None);
    }

    #[test]
    fn the_problems_print_after_the_identifier() {
        // Arrange
        let problems = [
            CalloutProblem::NotOpen,
            CalloutProblem::NotMadeBy(context(4)),
            CalloutProblem::NoResponseHeader,
        ];

        // Act
        let texts = problems.map(|problem| problem.to_string());

        // Assert
        assert_eq!(
            texts,
            [
                "is not open",
                "was not made by context 4",
                "got a received response with no header"
            ]
        );
    }
}
