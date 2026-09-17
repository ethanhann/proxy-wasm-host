//! `proxy_done` and `proxy_set_effective_context`.

use wasmtime::AsContextMut;

use crate::abi::v0_2_1::ContextId;
use crate::abi::v0_2_1::host_functions::Failure;
use crate::abi::v0_2_1::types::Status;
use crate::runtime::HostState;

pub(super) fn proxy_done(ctx: &mut impl AsContextMut<Data = HostState>) -> Result<(), Failure> {
    if ctx
        .as_context_mut()
        .data_mut()
        .abi_mut()
        .contexts_mut()
        .done()
    {
        Ok(())
    } else {
        Err(Status::NotFound.into())
    }
}

/// Moves the effective context inside the root the callback is serving.
///
/// A guest may name only a context under that root, because the crate serves
/// the plugin configuration and the tick period of a root from the effective
/// context, and one instance can hold a root per plugin.
pub(super) fn proxy_set_effective_context(
    ctx: &mut impl AsContextMut<Data = HostState>,
    context_id: i32,
) -> Result<(), Failure> {
    let id = ContextId::try_from(context_id)?;
    let mut ctx = ctx.as_context_mut();
    let contexts = ctx.data_mut().abi_mut().contexts_mut();
    let Some(current) = contexts.effective() else {
        return Err(Status::BadArgument.into());
    };
    if contexts.root_of(id) != contexts.root_of(current) {
        return Err(Status::BadArgument.into());
    }
    if contexts.is_rejected(id) || !contexts.set_effective(id) {
        return Err(Status::BadArgument.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::v0_2_1::ContextState;
    use crate::abi::v0_2_1::test_support::{outcome, status};
    use crate::runtime::test_support::{MINIMAL_GUEST, engine, instance};

    const CALLERS: &str = r#"(module
        (import "env" "proxy_done" (func $done (result i32)))
        (import "env" "proxy_set_effective_context" (func $set (param i32) (result i32)))
        (memory (export "memory") 1)
        (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 1024)
        (func (export "done") (result i32) call $done)
        (func (export "set") (param i32) (result i32) local.get 0 call $set))"#;

    #[test]
    fn the_effective_context_changes_only_to_a_known_identifier() {
        // Arrange
        let engine = engine();
        let mut instance = instance(&engine, CALLERS).unwrap();
        let contexts = instance.state_mut().abi_mut().contexts_mut();
        let root = contexts.create(None).unwrap();
        let stream = contexts.create(Some(root)).unwrap();
        contexts.set_effective(root);

        // Act
        let results = [
            instance.call::<i32, i32>("set", stream.wire()).map(status),
            instance.call::<i32, i32>("set", 9).map(status),
            instance.call::<i32, i32>("set", 0).map(status),
        ];

        // Assert
        assert_eq!(results[0].as_ref().unwrap(), &Status::Ok);
        assert_eq!(results[1].as_ref().unwrap(), &Status::BadArgument);
        assert_eq!(results[2].as_ref().unwrap(), &Status::BadArgument);
        assert_eq!(instance.state().abi().contexts().effective(), Some(stream));
    }

    #[test]
    fn a_refused_root_cannot_become_effective() {
        // Arrange
        let engine = engine();
        let mut instance = instance(&engine, CALLERS).unwrap();
        let contexts = instance.state_mut().abi_mut().contexts_mut();
        let root = contexts.create(None).unwrap();
        let stream = contexts.create(Some(root)).unwrap();
        let other = contexts.create(None).unwrap();
        contexts.set_effective(other);
        contexts.reject(root);

        // Act
        let result = instance.call::<i32, i32>("set", stream.wire()).map(status);

        // Assert
        assert_eq!(result.unwrap(), Status::BadArgument);
        assert_eq!(instance.state().abi().contexts().effective(), Some(other));
    }

    #[test]
    fn done_needs_a_pending_effective_context() {
        // Arrange
        let engine = engine();
        let mut instance = instance(&engine, CALLERS).unwrap();
        let contexts = instance.state_mut().abi_mut().contexts_mut();
        let root = contexts.create(None).unwrap();
        contexts.set_effective(root);
        let while_active = instance.call::<(), i32>("done", ()).map(status);
        instance
            .state_mut()
            .abi_mut()
            .contexts_mut()
            .set_state(root, ContextState::Pending);

        // Act
        let while_pending = instance.call::<(), i32>("done", ()).map(status);

        // Assert
        assert_eq!(while_active.unwrap(), Status::NotFound);
        assert_eq!(while_pending.unwrap(), Status::Ok);
        assert_eq!(
            instance.state().abi().contexts().state(root),
            Some(ContextState::Done)
        );
    }

    #[test]
    fn the_bodies_run_on_a_bare_store() {
        // Arrange
        let engine = engine();
        let mut instance = instance(&engine, MINIMAL_GUEST).unwrap();

        // Act
        let results = (
            outcome(proxy_done(instance.store_mut())),
            outcome(proxy_set_effective_context(instance.store_mut(), 1)),
        );

        // Assert
        assert_eq!(results, (Status::NotFound, Status::BadArgument));
    }

    #[test]
    fn the_effective_context_never_moves_to_another_root() {
        // Arrange
        let engine = engine();
        let mut instance = instance(&engine, CALLERS).unwrap();
        let contexts = instance.state_mut().abi_mut().contexts_mut();
        let mine = contexts.create(None).unwrap();
        let theirs = contexts.create(None).unwrap();
        let under_theirs = contexts.create(Some(theirs)).unwrap();
        contexts.set_effective(mine);

        // Act
        let results = [
            instance.call::<i32, i32>("set", theirs.wire()).map(status),
            instance
                .call::<i32, i32>("set", under_theirs.wire())
                .map(status),
        ];

        // Assert
        assert_eq!(results[0].as_ref().unwrap(), &Status::BadArgument);
        assert_eq!(results[1].as_ref().unwrap(), &Status::BadArgument);
        assert_eq!(instance.state().abi().contexts().effective(), Some(mine));
    }

    #[test]
    fn the_effective_context_moves_with_no_callback_running_only_inside_its_root() {
        // Arrange
        let engine = engine();
        let mut instance = instance(&engine, CALLERS).unwrap();
        let root = instance
            .state_mut()
            .abi_mut()
            .contexts_mut()
            .create(None)
            .unwrap();

        // Act
        let result = instance.call::<i32, i32>("set", root.wire()).map(status);

        // Assert
        assert_eq!(result.unwrap(), Status::BadArgument);
        assert_eq!(instance.state().abi().contexts().effective(), None);
    }
}
