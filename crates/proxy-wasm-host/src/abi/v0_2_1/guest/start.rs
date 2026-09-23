//! The start sequence of a root context.

use crate::abi::v0_2_1::AbiAccess;
use crate::abi::v0_2_1::{
    CallScope, Callback, ContextId, ContextTable, Guest, GuestError, NoStream, PluginConfig,
};

/// How [`Guest::start`] ended when no callback failed.
///
/// A root that passed both of its start callbacks serves stream contexts, and
/// a root that answered false to either one serves nothing.
/// [`Guest::start`] answers this so that you can tell the two apart before
/// you send a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use = "a start can be refused, and a refused root serves nothing"]
pub enum Started {
    /// Both callbacks accepted, and the root serves stream contexts.
    Serving(ContextId),
    /// A callback answered false.
    ///
    /// `VmStart` means the whole guest is refused.
    /// Every later callback answers [`GuestError::GuestRejected`], and
    /// [`Guest::is_serving`] answers false.
    ///
    /// `Configure` means this root is refused, and you may start another
    /// root on the same guest.
    /// The refused root stays in the guest with its plugin and with every
    /// callout it opened, and it keeps its root id, so a second start with
    /// that root id is refused.
    /// To remove it, call
    /// [`CallScope::on_done`](crate::abi::v0_2_1::CallScope::on_done) and then
    /// [`CallScope::on_delete`](crate::abi::v0_2_1::CallScope::on_delete) on
    /// the root.
    /// `on_delete` ends its callouts and answers their identifiers.
    Refused {
        /// The root context that was created.
        root: ContextId,
        /// The callback that answered false.
        callback: Callback,
    },
}

impl Started {
    /// The root context the start created, whether it serves or not.
    pub fn root(&self) -> ContextId {
        match *self {
            Self::Serving(root) | Self::Refused { root, .. } => root,
        }
    }
}

impl Guest {
    /// Creates a root context, runs `proxy_on_vm_start`, and runs
    /// `proxy_on_configure` with `plugin`.
    ///
    /// A root must pass three steps before it serves a request.
    /// This method runs them in one call.
    /// The method returns the guest to you whether the start succeeded or
    /// failed.
    /// After a refusal or a failure, [`Guest::open_callouts`] still names the
    /// callouts the root opened.
    /// A plugin with a second root calls `start` again with the
    /// [`PluginConfig`] of that root.
    ///
    /// A callback that fails refuses the root, as an answer of false does.
    /// A failed VM start refuses the whole guest.
    /// A failed configuration refuses the root alone.
    ///
    /// A guest validates its own configuration, which [`PluginConfig`]
    /// describes.
    /// A guest that rejects what it reads answers false.
    /// This method then answers [`Started::Refused`] with
    /// [`Callback::Configure`](crate::abi::v0_2_1::Callback::Configure).
    ///
    /// # Errors
    ///
    /// Returns [`GuestError::DuplicateRootId`] before any callback when a
    /// root of this guest already holds the root id of `plugin`.
    /// Returns the [`GuestError`] of the callback that failed, which
    /// [`CallScope::on_context_create`](crate::abi::v0_2_1::CallScope::on_context_create),
    /// [`CallScope::on_vm_start`](crate::abi::v0_2_1::CallScope::on_vm_start),
    /// and [`CallScope::on_configure`](crate::abi::v0_2_1::CallScope::on_configure)
    /// describe.
    /// A guest whose VM start refused answers [`GuestError::GuestRejected`].
    pub fn start(&mut self, plugin: PluginConfig) -> Result<Started, GuestError> {
        if let Some(root) = self.contexts().root_with_id(plugin.root_id()) {
            return Err(GuestError::DuplicateRootId {
                root_id: plugin.root_id().to_vec(),
                root,
            });
        }
        let mut scope = self.enter_root();
        let root = scope.on_context_create(None)?;
        let answer = run(&mut scope, root, plugin);
        drop(scope);
        answer.map_err(|(callback, error)| {
            let contexts = self.instance_mut().state_mut().abi_mut().contexts_mut();
            if callback == Callback::VmStart {
                contexts.reject_vm(root);
            } else {
                contexts.reject(root);
            }
            error
        })
    }

    fn contexts(&self) -> &ContextTable {
        self.instance().state().abi().contexts()
    }
}

/// Runs the two callbacks of a start, and names the callback of a failure.
fn run(
    scope: &mut CallScope<'_, NoStream>,
    root: ContextId,
    plugin: PluginConfig,
) -> Result<Started, (Callback, GuestError)> {
    let failed = |callback| move |error| (callback, error);
    if !scope.on_vm_start(root).map_err(failed(Callback::VmStart))? {
        return Ok(Started::Refused {
            root,
            callback: Callback::VmStart,
        });
    }
    if !scope
        .on_configure(root, plugin)
        .map_err(failed(Callback::Configure))?
    {
        return Ok(Started::Refused {
            root,
            callback: Callback::Configure,
        });
    }
    Ok(Started::Serving(root))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::Error;
    use crate::abi::v0_2_1::test_support::{RecordingSink, engine, wat_bytes};
    use crate::abi::v0_2_1::types::LogLevel;
    use crate::abi::v0_2_1::{Host, VmServices};
    use crate::runtime::{Engine, Limits, Module};

    const ACCEPT: &str = "i32.const 1";
    const REFUSE: &str = "i32.const 0";
    const TRAP: &str = "unreachable";
    const NOT_A_BOOLEAN: &str = "i32.const 2";

    /// A guest that logs `vm` and `cf` from its two start callbacks and then
    /// runs the given tails.
    fn starter(vm_start: &str, configure: &str) -> String {
        format!(
            r#"(module
            (import "env" "proxy_log" (func $log (param i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 4096)
            (func (export "proxy_abi_version_0_2_1"))
            (func (export "proxy_on_vm_start") (param i32 i32) (result i32)
                (drop (call $log (i32.const 2) (i32.const 100) (i32.const 2))) {vm_start})
            (func (export "proxy_on_configure") (param i32 i32) (result i32)
                (drop (call $log (i32.const 2) (i32.const 102) (i32.const 2))) {configure})
            (data (i32.const 100) "vmcf"))"#
        )
    }

    fn guest(engine: &Engine, wat: &str, sink: &Arc<RecordingSink>) -> Guest {
        let module = Module::new(engine, &wat_bytes(wat)).unwrap();
        Guest::new(
            &Host::new(engine).unwrap(),
            &module,
            VmServices::new(sink.clone()),
            &Limits::default(),
        )
        .unwrap()
    }

    #[test]
    fn start_runs_the_vm_start_and_the_configuration_on_one_root() {
        // Arrange
        let sink = Arc::new(RecordingSink::default());
        let mut guest = guest(&engine(), &starter(ACCEPT, ACCEPT), &sink);
        let plugin = PluginConfig::new().with_root_id(*b"http");

        // Act
        let started = guest.start(plugin.clone());

        // Assert
        let root = ContextId::try_from(1).unwrap();
        assert_eq!(started.unwrap(), Started::Serving(root));
        assert_eq!(
            sink.entries(),
            [
                (LogLevel::Info, b"vm".to_vec()),
                (LogLevel::Info, b"cf".to_vec())
            ]
        );
        assert_eq!(guest.plugin(root), Some(&plugin));
        assert_eq!(guest.rejected_by(root), None);
    }

    #[test]
    fn a_refused_vm_start_is_refused_and_runs_no_configuration() {
        // Arrange
        let sink = Arc::new(RecordingSink::default());
        let mut guest = guest(&engine(), &starter(REFUSE, ACCEPT), &sink);
        let plugin = PluginConfig::new();

        // Act
        let started = guest.start(plugin);

        // Assert
        let root = ContextId::try_from(1).unwrap();
        assert_eq!(
            started.unwrap(),
            Started::Refused {
                root,
                callback: Callback::VmStart
            }
        );
        assert_eq!(sink.entries(), [(LogLevel::Info, b"vm".to_vec())]);
        assert_eq!(guest.plugin(root), None);
        assert_eq!(guest.rejected_by(root), Some(Callback::VmStart));
    }

    #[test]
    fn a_refused_configuration_is_refused() {
        // Arrange
        let sink = Arc::new(RecordingSink::default());
        let mut guest = guest(&engine(), &starter(ACCEPT, REFUSE), &sink);
        let plugin = PluginConfig::new();

        // Act
        let started = guest.start(plugin);

        // Assert
        let root = ContextId::try_from(1).unwrap();
        assert_eq!(
            started.unwrap(),
            Started::Refused {
                root,
                callback: Callback::Configure
            }
        );
        assert_eq!(sink.entries().len(), 2);
        assert_eq!(guest.rejected_by(root), Some(Callback::Configure));
    }

    #[test]
    fn a_second_start_after_a_refused_vm_is_guest_rejected() {
        // Arrange
        let sink = Arc::new(RecordingSink::default());
        let mut guest = guest(&engine(), &starter(REFUSE, ACCEPT), &sink);
        let _ = guest.start(PluginConfig::new()).unwrap();
        let plugin = PluginConfig::new();

        // Act
        let second = guest.start(plugin);

        // Assert
        assert!(matches!(
            second,
            Err(GuestError::GuestRejected {
                callback: Callback::VmStart,
                ..
            })
        ));
        assert_eq!(sink.entries().len(), 1);
    }

    #[test]
    fn a_second_root_starts_with_its_own_plugin() {
        // Arrange
        let sink = Arc::new(RecordingSink::default());
        let mut guest = guest(&engine(), &starter(ACCEPT, ACCEPT), &sink);
        let first = PluginConfig::new().with_root_id(*b"http");
        let second = PluginConfig::new().with_root_id(*b"tcp");
        let first_root = guest.start(first.clone()).unwrap().root();

        // Act
        let started = guest.start(second.clone());

        // Assert
        let second_root = ContextId::try_from(2).unwrap();
        assert_eq!(started.unwrap(), Started::Serving(second_root));
        assert_eq!(guest.plugin(first_root), Some(&first));
        assert_eq!(guest.plugin(second_root), Some(&second));
    }

    #[test]
    fn a_trap_in_the_vm_start_keeps_the_guest() {
        // Arrange
        let sink = Arc::new(RecordingSink::default());
        let mut guest = guest(&engine(), &starter(TRAP, ACCEPT), &sink);
        let plugin = PluginConfig::new();

        // Act
        let started = guest.start(plugin);

        // Assert
        assert!(matches!(
            started,
            Err(GuestError::Runtime(Error::Trap { .. }))
        ));
        assert!(guest.is_poisoned());
        assert!(guest.open_callouts().is_empty());
        assert_eq!(sink.entries(), [(LogLevel::Info, b"vm".to_vec())]);
    }

    #[test]
    fn a_root_id_that_a_root_holds_is_refused_before_any_callback() {
        // Arrange
        let sink = Arc::new(RecordingSink::default());
        let mut guest = guest(&engine(), &starter(ACCEPT, ACCEPT), &sink);
        let plugin = PluginConfig::new().with_root_id(*b"http");
        let first = guest.start(plugin.clone()).unwrap().root();

        // Act
        let second = guest.start(plugin);

        // Assert
        assert!(matches!(
            second,
            Err(GuestError::DuplicateRootId { root_id, root })
                if root_id == b"http" && root == first
        ));
        assert_eq!(guest.context_type(ContextId::try_from(2).unwrap()), None);
        assert_eq!(sink.entries().len(), 2);
    }

    #[test]
    fn a_root_id_is_free_again_after_its_root_is_deleted() {
        // Arrange
        let sink = Arc::new(RecordingSink::default());
        let mut guest = guest(&engine(), &starter(ACCEPT, REFUSE), &sink);
        let plugin = PluginConfig::new().with_root_id(*b"http");
        let refused = guest.start(plugin.clone()).unwrap().root();
        let mut scope = guest.enter_root();
        scope.on_done(refused).unwrap();
        scope.on_delete(refused).unwrap();
        drop(scope);

        // Act
        let started = guest.start(plugin);

        // Assert
        assert_eq!(
            started.unwrap(),
            Started::Refused {
                root: ContextId::try_from(2).unwrap(),
                callback: Callback::Configure
            }
        );
    }

    #[test]
    fn a_configuration_that_answers_no_boolean_refuses_its_root() {
        // Arrange
        let sink = Arc::new(RecordingSink::default());
        let mut guest = guest(&engine(), &starter(ACCEPT, NOT_A_BOOLEAN), &sink);
        let plugin = PluginConfig::new();
        let root = ContextId::try_from(1).unwrap();

        // Act
        let started = guest.start(plugin);

        // Assert
        assert!(matches!(
            started,
            Err(GuestError::UnexpectedReturn {
                callback: Callback::Configure,
                value: 2
            })
        ));
        assert!(!guest.is_poisoned());
        assert_eq!(guest.rejected_by(root), Some(Callback::Configure));
        assert!(guest.is_serving());
    }

    #[test]
    fn a_vm_start_that_answers_no_boolean_refuses_the_guest() {
        // Arrange
        let sink = Arc::new(RecordingSink::default());
        let mut guest = guest(&engine(), &starter(NOT_A_BOOLEAN, ACCEPT), &sink);
        let plugin = PluginConfig::new();
        let root = ContextId::try_from(1).unwrap();

        // Act
        let started = guest.start(plugin);

        // Assert
        assert!(matches!(
            started,
            Err(GuestError::UnexpectedReturn {
                callback: Callback::VmStart,
                value: 2
            })
        ));
        assert_eq!(guest.rejected_by(root), Some(Callback::VmStart));
        assert!(!guest.is_serving());
        assert_eq!(sink.entries(), [(LogLevel::Info, b"vm".to_vec())]);
    }

    #[test]
    fn the_root_of_each_answer_is_the_root_that_was_created() {
        // Arrange
        let root = ContextId::try_from(3).unwrap();
        let answers = [
            Started::Serving(root),
            Started::Refused {
                root,
                callback: Callback::Configure,
            },
        ];

        // Act
        let roots = answers.map(|answer| answer.root());

        // Assert
        assert_eq!(roots, [root, root]);
    }
}
