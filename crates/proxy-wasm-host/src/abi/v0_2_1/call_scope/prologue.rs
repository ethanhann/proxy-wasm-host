//! The checks and the call every callback method shares.

use wasmtime::{TypedFunc, WasmParams, WasmResults};

use crate::Error;
use crate::abi::v0_2_1::AbiAccess;
use crate::abi::v0_2_1::GuestError;
use crate::abi::v0_2_1::{
    Callback, ContextId, ContextProblem, ContextState, ContextType, Guest, StreamKind,
};

fn problem(context: ContextId, problem: ContextProblem) -> GuestError {
    GuestError::Context {
        id: context,
        problem,
    }
}

pub(super) fn require(guest: &Guest, context: ContextId) -> Result<ContextType, GuestError> {
    guest
        .context_type(context)
        .ok_or_else(|| problem(context, ContextProblem::Unknown))
}

pub(super) fn require_root(guest: &Guest, context: ContextId) -> Result<(), GuestError> {
    match require(guest, context)? {
        ContextType::Root => Ok(()),
        ContextType::Stream => Err(problem(context, ContextProblem::NotRoot)),
    }
}

pub(super) fn require_stream(guest: &Guest, context: ContextId) -> Result<(), GuestError> {
    match require(guest, context)? {
        ContextType::Stream => Ok(()),
        ContextType::Root => Err(problem(context, ContextProblem::NotStream)),
    }
}

pub(super) fn require_done(guest: &Guest, context: ContextId) -> Result<(), GuestError> {
    require(guest, context)?;
    match guest.context_state(context) {
        Some(ContextState::Done) => Ok(()),
        _ => Err(problem(context, ContextProblem::NotDone)),
    }
}

pub(super) fn require_deletable(guest: &Guest, context: ContextId) -> Result<(), GuestError> {
    require_done(guest, context)?;
    if guest
        .instance()
        .state()
        .abi()
        .contexts()
        .has_children(context)
    {
        return Err(problem(context, ContextProblem::HasChildren));
    }
    Ok(())
}

/// Refuses a stream callback of the family that `context` does not serve.
///
/// A guest decides the family of a stream context inside its own SDK, and
/// both SDKs panic on a callback of the other family, so the crate refuses
/// the callback before the guest runs.
/// The family is recorded when a callback reaches the guest, which
/// [`record_stream_kind`] does.
pub(super) fn require_stream_kind(
    guest: &Guest,
    context: ContextId,
    kind: StreamKind,
) -> Result<(), GuestError> {
    match guest
        .instance()
        .state()
        .abi()
        .contexts()
        .stream_kind(context)
    {
        Some(recorded) if recorded != kind => Err(problem(
            context,
            ContextProblem::WrongStreamKind {
                recorded,
                attempted: kind,
            },
        )),
        _ => Ok(()),
    }
}

/// Records the family of a stream context, which the first callback that
/// reaches the guest fills.
pub(super) fn record_stream_kind(guest: &mut Guest, context: ContextId, kind: StreamKind) {
    guest
        .instance_mut()
        .state_mut()
        .abi_mut()
        .contexts_mut()
        .set_stream_kind(context, kind);
}

pub(super) fn accepted(guest: &Guest, context: ContextId) -> Result<(), GuestError> {
    let contexts = guest.instance().state().abi().contexts();
    match contexts.rejection_of(context) {
        None => Ok(()),
        Some(callback @ Callback::VmStart) => Err(GuestError::GuestRejected {
            callback,
            root: contexts.vm_rejected().unwrap_or(context),
        }),
        Some(callback) => Err(GuestError::GuestRejected {
            callback,
            root: contexts.root_of(context).unwrap_or(context),
        }),
    }
}

pub(super) fn vm_accepted(guest: &Guest) -> Result<(), GuestError> {
    match guest.instance().state().abi().contexts().vm_rejected() {
        Some(root) => Err(GuestError::GuestRejected {
            callback: Callback::VmStart,
            root,
        }),
        None => Ok(()),
    }
}

pub(super) fn wire_u32(value: u32) -> Result<i32, Error> {
    i32::try_from(value).map_err(|_| Error::ValueTooLarge {
        size: value as usize,
    })
}

/// A byte length in the form the ABI gives a guest.
pub(super) fn wire_size(len: usize) -> Result<i32, Error> {
    i32::try_from(len).map_err(|_| Error::ValueTooLarge { size: len })
}

pub(super) fn boolean(callback: Callback, value: i32) -> Result<bool, GuestError> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        value => Err(GuestError::UnexpectedReturn { callback, value }),
    }
}

/// Sets the effective context and the current callback, calls the guest or
/// takes the default, and clears the current callback on return.
///
/// A panic inside the call leaves the current callback set, which is how
/// the scope knows that the callback did not return.
pub(super) fn run<P: WasmParams, R: WasmResults>(
    guest: &mut Guest,
    context: ContextId,
    callback: Callback,
    func: Option<TypedFunc<P, R>>,
    params: P,
    default: R,
) -> Result<R, Error> {
    let state = guest.instance_mut().state_mut();
    state.abi_mut().contexts_mut().set_effective(context);
    state.abi_mut().set_current_callback(Some(callback));
    let result = match func {
        None => Ok(default),
        Some(func) => guest.instance_mut().call_typed(&func, params),
    };
    guest
        .instance_mut()
        .state_mut()
        .abi_mut()
        .set_current_callback(None);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_length_the_abi_can_carry_converts_to_the_wire_form() {
        // Arrange
        let lengths = [0, 6, usize::try_from(i32::MAX).unwrap()];

        // Act
        let results = lengths.map(wire_size);

        // Assert
        assert_eq!(results[0].as_ref().unwrap(), &0);
        assert_eq!(results[1].as_ref().unwrap(), &6);
        assert_eq!(results[2].as_ref().unwrap(), &i32::MAX);
    }

    #[test]
    fn a_length_above_the_wire_form_is_too_large() {
        // Arrange
        let length = usize::try_from(i32::MAX).unwrap() + 1;

        // Act
        let result = wire_size(length);

        // Assert
        assert!(matches!(result, Err(Error::ValueTooLarge { size }) if size == length));
    }
}
