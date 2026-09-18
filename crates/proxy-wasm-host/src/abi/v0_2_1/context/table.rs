//! The table of live contexts, and what each one holds.

use std::collections::BTreeMap;
use std::num::NonZeroU32;
use std::time::Duration;

use crate::Error;
use crate::abi::v0_2_1::{
    Callback, ContextId, ContextProblem, ContextState, ContextType, PluginConfig,
};

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
    plugin: Option<PluginConfig>,
    tick_period: Option<Duration>,
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
        let id = ContextId::new(self.next);
        self.next = successor;
        self.entries.insert(
            id,
            ContextEntry {
                context_type,
                parent,
                state: ContextState::Active,
                rejected: false,
                plugin: None,
                tick_period: None,
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

    /// Records the plugin of a root context, and reports whether it did.
    pub(crate) fn set_plugin(&mut self, root: ContextId, plugin: PluginConfig) -> bool {
        match self.entries.get_mut(&root) {
            Some(entry) if entry.context_type == ContextType::Root => {
                entry.plugin = Some(plugin);
                true
            }
            _ => false,
        }
    }

    /// The plugin of the root context of `id`.
    pub(crate) fn plugin(&self, id: ContextId) -> Option<&PluginConfig> {
        let root = self.root_of(id)?;
        self.entries.get(&root)?.plugin.as_ref()
    }

    /// Records a tick period on the root context of `id`, and reports whether
    /// it did.
    pub(crate) fn set_tick_period(&mut self, id: ContextId, period: Option<Duration>) -> bool {
        let Some(root) = self.root_of(id) else {
            return false;
        };
        match self.entries.get_mut(&root) {
            Some(entry) => {
                entry.tick_period = period;
                true
            }
            None => false,
        }
    }

    /// The tick period of a root context.
    pub(crate) fn tick_period(&self, root: ContextId) -> Option<Duration> {
        let entry = self.entries.get(&root)?;
        if entry.context_type == ContextType::Root {
            entry.tick_period
        } else {
            None
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
    fn a_plugin_is_recorded_on_a_root_and_read_through_its_streams() {
        // Arrange
        let (mut table, root, stream) = table_with_root_and_stream();
        let plugin = PluginConfig::new().with_name(b"auth".to_vec());

        // Act
        let recorded = table.set_plugin(root, plugin);

        // Assert
        assert!(recorded);
        assert_eq!(table.plugin(root).unwrap().name(), b"auth");
        assert_eq!(table.plugin(stream).unwrap().name(), b"auth");
    }

    #[test]
    fn a_plugin_is_refused_on_a_stream_context() {
        // Arrange
        let (mut table, root, stream) = table_with_root_and_stream();

        // Act
        let recorded = table.set_plugin(stream, PluginConfig::new().with_name(b"auth".to_vec()));

        // Assert
        assert!(!recorded);
        assert!(table.plugin(root).is_none());
        assert!(table.plugin(stream).is_none());
    }

    #[test]
    fn a_plugin_is_none_for_an_unknown_context() {
        // Arrange
        let table = ContextTable::new();

        // Act
        let plugin = table.plugin(id(7));

        // Assert
        assert!(plugin.is_none());
    }

    #[test]
    fn a_tick_period_set_from_a_stream_lands_on_its_root() {
        // Arrange
        let (mut table, root, stream) = table_with_root_and_stream();

        // Act
        let recorded = table.set_tick_period(stream, Some(Duration::from_millis(250)));

        // Assert
        assert!(recorded);
        assert_eq!(table.tick_period(root), Some(Duration::from_millis(250)));
        assert_eq!(table.tick_period(stream), None);
    }

    #[test]
    fn a_tick_period_set_on_a_root_reads_back() {
        // Arrange
        let (mut table, root, _) = table_with_root_and_stream();

        // Act
        let recorded = table.set_tick_period(root, Some(Duration::from_millis(40)));

        // Assert
        assert!(recorded);
        assert_eq!(table.tick_period(root), Some(Duration::from_millis(40)));
    }

    #[test]
    fn a_tick_period_of_none_clears_a_recorded_one() {
        // Arrange
        let (mut table, root, _) = table_with_root_and_stream();
        table.set_tick_period(root, Some(Duration::from_millis(250)));

        // Act
        let cleared = table.set_tick_period(root, None);

        // Assert
        assert!(cleared);
        assert_eq!(table.tick_period(root), None);
    }

    #[test]
    fn a_tick_period_on_an_unknown_context_is_refused() {
        // Arrange
        let mut table = ContextTable::new();

        // Act
        let recorded = table.set_tick_period(id(7), Some(Duration::from_millis(1)));

        // Assert
        assert!(!recorded);
        assert_eq!(table.tick_period(id(7)), None);
    }

    #[test]
    fn remove_drops_the_plugin_and_the_period_of_a_root() {
        // Arrange
        let mut table = ContextTable::new();
        let root = table.create(None).unwrap();
        table.set_plugin(root, PluginConfig::new().with_name(b"auth".to_vec()));
        table.set_tick_period(root, Some(Duration::from_millis(250)));

        // Act
        let removed = table.remove(root);

        // Assert
        assert_eq!(removed, Some(ContextType::Root));
        assert!(table.plugin(root).is_none());
        assert_eq!(table.tick_period(root), None);
    }
}
