//! Contexts, their identifiers, and the table that tracks them.
//!
//! The ABI gives every root context and every stream context a `u32`
//! identifier.
//! This crate allocates those identifiers and records, for each live context,
//! its type, its parent, and how far through the done, log, delete sequence
//! it is.

use std::collections::BTreeMap;
use std::fmt;
use std::num::NonZeroU32;

use crate::Error;
use crate::abi::v0_2_1::Callback;

/// The identifier of a root context or a stream context.
///
/// Zero is never a context, because the ABI uses it for the absent parent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContextId(NonZeroU32);

impl ContextId {
    /// The identifier as the ABI carries it.
    pub fn get(self) -> u32 {
        self.0.get()
    }

    /// The identifier as a wasm `i32` parameter.
    pub(crate) fn wire(self) -> i32 {
        self.get().cast_signed()
    }
}

impl TryFrom<u32> for ContextId {
    type Error = InvalidContextId;

    fn try_from(value: u32) -> Result<Self, InvalidContextId> {
        NonZeroU32::new(value).map(Self).ok_or(InvalidContextId {
            value: i64::from(value),
        })
    }
}

impl TryFrom<i32> for ContextId {
    type Error = InvalidContextId;

    fn try_from(value: i32) -> Result<Self, InvalidContextId> {
        u32::try_from(value)
            .ok()
            .and_then(NonZeroU32::new)
            .map(Self)
            .ok_or(InvalidContextId {
                value: i64::from(value),
            })
    }
}

impl fmt::Display for ContextId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A value that cannot be a context identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("{value} is not a valid context identifier")]
pub struct InvalidContextId {
    /// The value that was rejected.
    pub value: i64,
}

/// Which kind of context an identifier names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ContextType {
    /// A context with no parent, which the ABI calls the plugin context.
    Root,
    /// A context created under a root context, one per stream.
    Stream,
}

/// How far a context is through its finalization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ContextState {
    /// The context is in use.
    Active,
    /// `proxy_on_done` returned false, and the guest will call `proxy_done`.
    Pending,
    /// The guest is done with the context, so `proxy_on_log` and
    /// `proxy_on_delete` may run.
    Done,
}

/// What is wrong with a context argument.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ContextProblem {
    /// No context with the identifier exists in this instance.
    Unknown,
    /// The context is a stream context where a root context is required.
    NotRoot,
    /// The context is a root context where a stream context is required.
    NotStream,
    /// The context is not done, so `proxy_on_log` and `proxy_on_delete`
    /// cannot run.
    NotDone,
    /// The root context still has stream contexts under it.
    HasChildren,
}

impl fmt::Display for ContextProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Unknown => "is unknown",
            Self::NotRoot => "is not a root context",
            Self::NotStream => "is not a stream context",
            Self::NotDone => "is not done",
            Self::HasChildren => "still has stream contexts",
        })
    }
}

/// The live contexts of one instance.
#[derive(Debug)]
pub(crate) struct ContextTable {
    entries: BTreeMap<ContextId, ContextEntry>,
    next: NonZeroU32,
    effective: Option<ContextId>,
    vm_rejected: Option<ContextId>,
}

#[derive(Debug)]
struct ContextEntry {
    context_type: ContextType,
    parent: Option<ContextId>,
    state: ContextState,
    rejected: bool,
}

impl ContextTable {
    pub(crate) fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
            next: NonZeroU32::MIN,
            effective: None,
            vm_rejected: None,
        }
    }

    pub(crate) fn create(&mut self, parent: Option<ContextId>) -> Result<ContextId, Error> {
        let context_type = match parent {
            None => ContextType::Root,
            Some(parent) => match self.context_type(parent) {
                Some(ContextType::Root) => ContextType::Stream,
                Some(ContextType::Stream) => {
                    return Err(Error::Context {
                        id: parent,
                        problem: ContextProblem::NotRoot,
                    });
                }
                None => {
                    return Err(Error::Context {
                        id: parent,
                        problem: ContextProblem::Unknown,
                    });
                }
            },
        };
        let successor = self.next.checked_add(1).ok_or(Error::ContextIdsExhausted)?;
        let id = ContextId(self.next);
        self.next = successor;
        self.entries.insert(
            id,
            ContextEntry {
                context_type,
                parent,
                state: ContextState::Active,
                rejected: false,
            },
        );
        Ok(id)
    }

    pub(crate) fn context_type(&self, id: ContextId) -> Option<ContextType> {
        self.entries.get(&id).map(|entry| entry.context_type)
    }

    pub(crate) fn parent(&self, id: ContextId) -> Option<ContextId> {
        self.entries.get(&id).and_then(|entry| entry.parent)
    }

    pub(crate) fn root_of(&self, id: ContextId) -> Option<ContextId> {
        self.entries
            .get(&id)
            .map(|entry| entry.parent.unwrap_or(id))
    }

    pub(crate) fn state(&self, id: ContextId) -> Option<ContextState> {
        self.entries.get(&id).map(|entry| entry.state)
    }

    pub(crate) fn set_state(&mut self, id: ContextId, state: ContextState) -> bool {
        match self.entries.get_mut(&id) {
            Some(entry) => {
                entry.state = state;
                true
            }
            None => false,
        }
    }

    pub(crate) fn has_children(&self, id: ContextId) -> bool {
        self.context_type(id) == Some(ContextType::Root)
            && self.entries.values().any(|entry| entry.parent == Some(id))
    }

    pub(crate) fn reject(&mut self, root: ContextId) -> bool {
        match self.entries.get_mut(&root) {
            Some(entry) => {
                entry.rejected = true;
                true
            }
            None => false,
        }
    }

    pub(crate) fn is_rejected(&self, id: ContextId) -> bool {
        self.root_of(id)
            .and_then(|root| self.entries.get(&root))
            .is_some_and(|entry| entry.rejected)
    }

    pub(crate) fn reject_vm(&mut self, root: ContextId) {
        self.vm_rejected = Some(root);
    }

    pub(crate) fn vm_rejected(&self) -> Option<ContextId> {
        self.vm_rejected
    }

    /// The callback that refused `id`, with a VM rejection outranking a root
    /// rejection.
    pub(crate) fn rejection_of(&self, id: ContextId) -> Option<Callback> {
        if self.vm_rejected.is_some() {
            return Some(Callback::VmStart);
        }
        self.is_rejected(id).then_some(Callback::Configure)
    }

    pub(crate) fn remove(&mut self, id: ContextId) -> Option<ContextType> {
        let removed = self.entries.remove(&id)?;
        if self.effective == Some(id) {
            self.effective = None;
        }
        Some(removed.context_type)
    }

    pub(crate) fn effective(&self) -> Option<ContextId> {
        self.effective
    }

    pub(crate) fn set_effective(&mut self, id: ContextId) -> bool {
        if self.entries.contains_key(&id) {
            self.effective = Some(id);
            true
        } else {
            false
        }
    }

    pub(crate) fn done(&mut self) -> bool {
        let Some(id) = self.effective else {
            return false;
        };
        match self.entries.get_mut(&id) {
            Some(entry) if entry.state == ContextState::Pending => {
                entry.state = ContextState::Done;
                true
            }
            _ => false,
        }
    }

    #[cfg(test)]
    pub(crate) fn starting_at(next: u32) -> Self {
        let mut table = Self::new();
        table.next = NonZeroU32::new(next).unwrap();
        table
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(value: u32) -> ContextId {
        ContextId::try_from(value).unwrap()
    }

    fn table_with_root_and_stream() -> (ContextTable, ContextId, ContextId) {
        let mut table = ContextTable::new();
        let root = table.create(None).unwrap();
        let stream = table.create(Some(root)).unwrap();
        (table, root, stream)
    }

    #[test]
    fn zero_and_negative_values_are_not_identifiers() {
        // Arrange
        let values = (0u32, 0i32, -1i32);

        // Act
        let results = (
            ContextId::try_from(values.0),
            ContextId::try_from(values.1),
            ContextId::try_from(values.2),
        );

        // Assert
        assert_eq!(results.0, Err(InvalidContextId { value: 0 }));
        assert_eq!(results.1, Err(InvalidContextId { value: 0 }));
        assert_eq!(results.2, Err(InvalidContextId { value: -1 }));
        assert_eq!(
            results.2.unwrap_err().to_string(),
            "-1 is not a valid context identifier"
        );
    }

    #[test]
    fn one_is_an_identifier_that_displays_itself() {
        // Arrange
        let value = 1u32;

        // Act
        let id = ContextId::try_from(value).unwrap();

        // Assert
        assert_eq!(id.get(), 1);
        assert_eq!(id.wire(), 1);
        assert_eq!(id.to_string(), "1");
    }

    #[test]
    fn roots_are_numbered_from_one_and_have_no_parent() {
        // Arrange
        let mut table = ContextTable::new();

        // Act
        let ids = (table.create(None).unwrap(), table.create(None).unwrap());

        // Assert
        assert_eq!((ids.0.get(), ids.1.get()), (1, 2));
        assert_eq!(table.context_type(ids.0), Some(ContextType::Root));
        assert_eq!(table.context_type(ids.1), Some(ContextType::Root));
        assert_eq!(table.state(ids.0), Some(ContextState::Active));
        assert_eq!(table.parent(ids.0), None);
        assert_eq!(table.root_of(ids.1), Some(ids.1));
    }

    #[test]
    fn a_stream_records_its_parent_and_cannot_be_a_parent() {
        // Arrange
        let (mut table, root, stream) = table_with_root_and_stream();

        // Act
        let result = table.create(Some(stream));

        // Assert
        assert_eq!(table.context_type(stream), Some(ContextType::Stream));
        assert_eq!(table.parent(stream), Some(root));
        assert_eq!(table.root_of(stream), Some(root));
        assert!(matches!(
            result,
            Err(Error::Context { id, problem: ContextProblem::NotRoot }) if id == stream
        ));
    }

    #[test]
    fn an_unknown_parent_is_reported() {
        // Arrange
        let mut table = ContextTable::new();

        // Act
        let result = table.create(Some(id(9)));

        // Assert
        assert!(matches!(
            result,
            Err(Error::Context { id, problem: ContextProblem::Unknown }) if id.get() == 9
        ));
    }

    #[test]
    fn the_last_identifier_is_never_handed_out() {
        // Arrange
        let mut table = ContextTable::starting_at(u32::MAX - 1);

        // Act
        let results = (table.create(None), table.create(None));

        // Assert
        assert_eq!(results.0.unwrap().get(), u32::MAX - 1);
        assert!(matches!(results.1, Err(Error::ContextIdsExhausted)));
    }

    #[test]
    fn the_effective_context_changes_only_to_a_known_one() {
        // Arrange
        let (mut table, root, _) = table_with_root_and_stream();
        table.set_effective(root);

        // Act
        let accepted = table.set_effective(id(9));

        // Assert
        assert!(!accepted);
        assert_eq!(table.effective(), Some(root));
    }

    #[test]
    fn done_succeeds_once_on_a_pending_effective_context() {
        // Arrange
        let (mut table, _, stream) = table_with_root_and_stream();
        let before_effective = table.done();
        table.set_effective(stream);
        let while_active = table.done();
        table.set_state(stream, ContextState::Pending);

        // Act
        let results = (table.done(), table.done());

        // Assert
        assert!(!before_effective);
        assert!(!while_active);
        assert_eq!(results, (true, false));
        assert_eq!(table.state(stream), Some(ContextState::Done));
    }

    #[test]
    fn children_are_counted_until_removed() {
        // Arrange
        let (mut table, root, stream) = table_with_root_and_stream();
        let before = table.has_children(root);

        // Act
        let removed = table.remove(stream);

        // Assert
        assert!(before);
        assert_eq!(removed, Some(ContextType::Stream));
        assert!(!table.has_children(root));
    }

    #[test]
    fn a_rejection_covers_the_root_and_its_streams_only() {
        // Arrange
        let (mut table, root, stream) = table_with_root_and_stream();
        let other = table.create(None).unwrap();

        // Act
        let marked = table.reject(root);

        // Assert
        assert!(marked);
        assert!(table.is_rejected(root));
        assert!(table.is_rejected(stream));
        assert!(!table.is_rejected(other));
        assert!(!table.is_rejected(id(9)));
    }

    #[test]
    fn a_vm_rejection_outranks_a_root_rejection() {
        // Arrange
        let (mut table, root, stream) = table_with_root_and_stream();
        let other = table.create(None).unwrap();
        table.reject(root);
        let before = (table.rejection_of(stream), table.rejection_of(other));

        // Act
        table.reject_vm(other);

        // Assert
        assert_eq!(before, (Some(Callback::Configure), None));
        assert_eq!(table.rejection_of(stream), Some(Callback::VmStart));
        assert_eq!(table.rejection_of(other), Some(Callback::VmStart));
        assert_eq!(table.vm_rejected(), Some(other));
    }

    #[test]
    fn a_stream_never_has_children() {
        // Arrange
        let (table, _, stream) = table_with_root_and_stream();

        // Act
        let has_children = table.has_children(stream);

        // Assert
        assert!(!has_children);
    }

    #[test]
    fn remove_clears_the_effective_context_it_named() {
        // Arrange
        let (mut table, root, stream) = table_with_root_and_stream();
        table.set_effective(stream);

        // Act
        let removed = (table.remove(stream), table.remove(stream));

        // Assert
        assert_eq!(removed, (Some(ContextType::Stream), None));
        assert_eq!(table.effective(), None);
        assert_eq!(table.state(stream), None);
        assert_eq!(table.context_type(root), Some(ContextType::Root));
    }

    #[test]
    fn problems_display_as_a_predicate() {
        // Arrange
        let problems = [
            ContextProblem::Unknown,
            ContextProblem::NotRoot,
            ContextProblem::NotStream,
            ContextProblem::NotDone,
            ContextProblem::HasChildren,
        ];

        // Act
        let texts: Vec<String> = problems.iter().map(ToString::to_string).collect();

        // Assert
        assert_eq!(
            texts,
            [
                "is unknown",
                "is not a root context",
                "is not a stream context",
                "is not done",
                "still has stream contexts"
            ]
        );
    }
}
