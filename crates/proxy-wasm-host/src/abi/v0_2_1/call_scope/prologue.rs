//! The checks and the call every callback method shares.

use wasmtime::{TypedFunc, WasmParams, WasmResults};

use crate::Error;
use crate::abi::v0_2_1::{Callback, ContextId, ContextProblem, ContextState, ContextType, Guest};

/// Refuses a poisoned instance, and poisons an instance whose last callback
/// did not return, which is what a caught panic leaves behind.
pub(super) fn live(guest: &mut Guest) -> Result<(), Error> {
    let state = guest.instance_mut().state_mut();
    if state.current_callback().is_some() {
        state.poison();
    }
    if state.is_poisoned() {
        Err(Error::Poisoned)
    } else {
        Ok(())
    }
}

fn problem(context: ContextId, problem: ContextProblem) -> Error {
    Error::Context {
        id: context,
        problem,
    }
}

pub(super) fn require(guest: &Guest, context: ContextId) -> Result<ContextType, Error> {
    guest
        .context_type(context)
        .ok_or_else(|| problem(context, ContextProblem::Unknown))
}

pub(super) fn require_root(guest: &Guest, context: ContextId) -> Result<(), Error> {
    match require(guest, context)? {
        ContextType::Root => Ok(()),
        ContextType::Stream => Err(problem(context, ContextProblem::NotRoot)),
    }
}

pub(super) fn require_stream(guest: &Guest, context: ContextId) -> Result<(), Error> {
    match require(guest, context)? {
        ContextType::Stream => Ok(()),
        ContextType::Root => Err(problem(context, ContextProblem::NotStream)),
    }
}

pub(super) fn require_done(guest: &Guest, context: ContextId) -> Result<(), Error> {
    require(guest, context)?;
    match guest.context_state(context) {
        Some(ContextState::Done) => Ok(()),
        _ => Err(problem(context, ContextProblem::NotDone)),
    }
}

pub(super) fn require_deletable(guest: &Guest, context: ContextId) -> Result<(), Error> {
    require_done(guest, context)?;
    if guest.instance().state().contexts().has_children(context) {
        return Err(problem(context, ContextProblem::HasChildren));
    }
    Ok(())
}

pub(super) fn accepted(guest: &Guest, context: ContextId) -> Result<(), Error> {
    let contexts = guest.instance().state().contexts();
    match contexts.rejection_of(context) {
        None => Ok(()),
        Some(callback @ Callback::VmStart) => Err(Error::GuestRejected {
            callback,
            root: contexts.vm_rejected().unwrap_or(context),
        }),
        Some(callback) => Err(Error::GuestRejected {
            callback,
            root: contexts.root_of(context).unwrap_or(context),
        }),
    }
}

pub(super) fn vm_accepted(guest: &Guest) -> Result<(), Error> {
    match guest.instance().state().contexts().vm_rejected() {
        Some(root) => Err(Error::GuestRejected {
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

pub(super) fn boolean(callback: Callback, value: i32) -> Result<bool, Error> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        value => Err(Error::UnexpectedReturn { callback, value }),
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
    state.contexts_mut().set_effective(context);
    state.set_current_callback(Some(callback));
    let result = match func {
        None => Ok(default),
        Some(func) => guest.instance_mut().call_typed(&func, params),
    };
    guest.instance_mut().state_mut().set_current_callback(None);
    result
}
