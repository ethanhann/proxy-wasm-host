//! `proxy_log` and `proxy_get_log_level`.
//!
//! A message longer than the embedder allows reaches the sink cut to that
//! length, and the guest receives the answer of a message that fits. A guest
//! reports its own failures through this call, so a refusal here would trap
//! the guest on the line that says why it is failing.

use wasmtime::AsContextMut;

use crate::abi::v0_2_1::AbiAccess;
use crate::abi::v0_2_1::host_functions::Failure;
use crate::abi::v0_2_1::host_functions::log_context::log_context;
use crate::abi::v0_2_1::types::LogLevel;
use crate::runtime::{GuestPtr, GuestSlice, HostState, split};

pub(super) fn proxy_log(
    ctx: &mut impl AsContextMut<Data = HostState>,
    log_level: i32,
    message_data: i32,
    message_size: i32,
) -> Result<(), Failure> {
    let level = LogLevel::try_from(log_level)?;
    let slice = GuestSlice::try_from((message_data, message_size))?;
    let (memory, state) = split(ctx)?;
    let message = memory.read(slice)?;
    let message = match state.max_log_bytes() {
        Some(max) if message.len() > max => &message[..max],
        _ => message,
    };
    state
        .abi()
        .services()
        .log()
        .log(log_context(state), level, message);
    Ok(())
}

pub(super) fn proxy_get_log_level(
    ctx: &mut impl AsContextMut<Data = HostState>,
    return_log_level: i32,
) -> Result<(), Failure> {
    let return_log_level = GuestPtr::try_from(return_log_level)?;
    let (mut memory, state) = split(ctx)?;
    let level = i32::from(state.abi().services().log_level()).cast_unsigned();
    memory.write_u32(return_log_level, level)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::abi::v0_2_1::test_support::{
        RecordingSink, engine, instance_with, instance_with_limits, instance_with_sink, outcome,
        status, wat_bytes,
    };
    use crate::abi::v0_2_1::types::Status;
    use std::borrow::Cow;

    use crate::abi::v0_2_1::payload::Delivery;
    use crate::abi::v0_2_1::{
        AbiAccess, Callback, CalloutId, ContextId, LogSink, PluginConfig, VmServices,
    };
    use crate::runtime::{Instance, Limits, Module};

    const LOGGER: &str = r#"(module
        (import "env" "proxy_log" (func $log (param i32 i32 i32) (result i32)))
        (memory (export "memory") 1)
        (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 1024)
        (func (export "log") (param i32 i32 i32) (result i32)
            local.get 0 local.get 1 local.get 2 call $log)
        (data (i32.const 16) "hello"))"#;

    /// An instance of `LOGGER` whose sink the test keeps, with a root and a
    /// stream context, and the callback a request would be in.
    fn serving(sink: &Arc<RecordingSink>) -> (Instance, ContextId, ContextId) {
        let engine = engine();
        let services = VmServices::new(Arc::clone(sink) as Arc<dyn LogSink>).with_vm_id(*b"vm-1");
        let module = Module::new(&engine, &wat_bytes(LOGGER)).unwrap();
        let mut instance = instance_with(&engine, &module, services).unwrap();
        let state = instance.state_mut();
        let root = state.abi_mut().contexts_mut().create(None).unwrap();
        let stream = state.abi_mut().contexts_mut().create(Some(root)).unwrap();
        state.abi_mut().contexts_mut().set_effective(stream);
        state
            .abi_mut()
            .set_current_callback(Some(Callback::RequestHeaders));
        (instance, root, stream)
    }

    /// An instance of `LOGGER` whose sink the test keeps, with the log limit
    /// an embedder chose.
    fn bounded(sink: &Arc<RecordingSink>, max_log_bytes: impl Into<Option<usize>>) -> Instance {
        let engine = engine();
        let services = VmServices::new(Arc::clone(sink) as Arc<dyn LogSink>);
        let module = Module::new(&engine, &wat_bytes(LOGGER)).unwrap();
        let limits = Limits::default().with_max_log_bytes(max_log_bytes);
        instance_with_limits(&engine, &module, services, &limits).unwrap()
    }

    /// The plugin the tests configure their root with.
    fn plugin() -> PluginConfig {
        PluginConfig::new()
            .with_name(*b"authz")
            .with_root_id(*b"main")
    }

    /// Logs "hello" at the info level through the guest.
    fn log_hello(instance: &mut Instance) -> Status {
        instance
            .call::<(i32, i32, i32), i32>("log", (2, 16, 5))
            .map(status)
            .unwrap()
    }

    #[test]
    fn a_line_from_a_stream_callback_names_its_plugin_and_its_call() {
        // Arrange
        let sink = Arc::new(RecordingSink::default());
        let (mut instance, root, stream) = serving(&sink);
        instance
            .state_mut()
            .abi_mut()
            .contexts_mut()
            .set_plugin(root, plugin());
        let guest = instance.state().abi().guest();

        // Act
        let result = log_hello(&mut instance);

        // Assert
        assert_eq!(result, Status::Ok);
        let (context, level, message) = sink.lines().pop().expect("one line was recorded");
        assert_eq!(level, LogLevel::Info);
        assert_eq!(message, b"hello");
        assert_eq!(
            context.vm_id.as_ref(),
            b"vm-1",
            "the VM id comes from the services the embedder supplied"
        );
        assert_eq!(context.guest, guest);
        assert_eq!(context.plugin_name.as_deref(), Some(b"authz".as_slice()));
        assert_eq!(context.root_id.as_deref(), Some(b"main".as_slice()));
        let call = context.call.expect("a callback was running");
        assert_eq!(call.context, stream);
        assert_eq!(call.callback, Some(Callback::RequestHeaders));
        assert_eq!(call.callout, None);
    }

    #[test]
    fn a_line_with_no_callback_running_carries_no_call() {
        // Arrange
        let sink = Arc::new(RecordingSink::default());
        let (mut instance, root, _) = serving(&sink);
        instance
            .state_mut()
            .abi_mut()
            .contexts_mut()
            .set_plugin(root, plugin());
        instance.state_mut().abi_mut().set_current_callback(None);

        // Act
        let result = log_hello(&mut instance);

        // Assert
        assert_eq!(result, Status::Ok);
        let (context, _, _) = sink.lines().pop().expect("one line was recorded");
        assert_eq!(
            context.call, None,
            "the context table keeps the last context after a callback returns"
        );
        assert_eq!(context.plugin_name, None);
        assert_eq!(context.root_id, None);
    }

    #[test]
    fn a_line_before_a_configuration_names_no_plugin() {
        // Arrange
        let sink = Arc::new(RecordingSink::default());
        let (mut instance, _, stream) = serving(&sink);

        // Act
        let result = log_hello(&mut instance);

        // Assert
        assert_eq!(result, Status::Ok);
        let (context, _, _) = sink.lines().pop().expect("one line was recorded");
        assert_eq!(context.plugin_name, None, "no configuration has run");
        assert_eq!(context.root_id, None);
        assert_eq!(context.call.map(|call| call.context), Some(stream));
    }

    #[test]
    fn a_line_with_a_callback_and_no_context_carries_no_call() {
        // Arrange
        let sink = Arc::new(RecordingSink::default());
        let (mut instance, root, stream) = serving(&sink);
        instance
            .state_mut()
            .abi_mut()
            .contexts_mut()
            .set_plugin(root, plugin());
        instance.state_mut().abi_mut().contexts_mut().remove(stream);

        // Act
        let result = log_hello(&mut instance);

        // Assert
        assert_eq!(result, Status::Ok);
        let (context, _, _) = sink.lines().pop().expect("one line was recorded");
        assert_eq!(
            context.call, None,
            "a callback with no effective context leaves the call absent"
        );
        assert_eq!(context.plugin_name, None);
    }

    #[test]
    fn a_line_inside_a_delivery_names_its_callout() {
        // Arrange
        let sink = Arc::new(RecordingSink::default());
        let (mut instance, _, _) = serving(&sink);
        let callout = CalloutId::try_from(1).unwrap();
        let delivery = Delivery::grpc_message(callout, Cow::Borrowed(b"body"));
        instance.state_mut().abi_mut().set_delivery(Some(delivery));

        // Act
        let result = log_hello(&mut instance);

        // Assert
        assert_eq!(result, Status::Ok);
        let (context, _, _) = sink.lines().pop().expect("one line was recorded");
        assert_eq!(context.call.and_then(|call| call.callout), Some(callout));
    }

    #[test]
    fn the_call_of_a_line_is_the_effective_context() {
        // Arrange
        let sink = Arc::new(RecordingSink::default());
        let (mut instance, root, _) = serving(&sink);
        instance
            .state_mut()
            .abi_mut()
            .contexts_mut()
            .set_effective(root);

        // Act
        let result = log_hello(&mut instance);

        // Assert
        assert_eq!(result, Status::Ok);
        let (context, _, _) = sink.lines().pop().expect("one line was recorded");
        assert_eq!(context.call.map(|call| call.context), Some(root));
    }

    #[test]
    fn two_guests_of_one_plugin_are_told_apart_by_their_identity() {
        // Arrange
        let sink = Arc::new(RecordingSink::default());
        let (mut first, first_root, _) = serving(&sink);
        let (mut second, second_root, _) = serving(&sink);
        for (instance, root) in [(&mut first, first_root), (&mut second, second_root)] {
            instance
                .state_mut()
                .abi_mut()
                .contexts_mut()
                .set_plugin(root, plugin());
        }
        log_hello(&mut first);

        // Act
        log_hello(&mut second);

        // Assert
        let lines = sink.lines();
        let (first, second) = (&lines[0].0, &lines[1].0);
        assert_ne!(
            first.guest, second.guest,
            "two guests of one plugin differ here and nowhere else"
        );
        assert_eq!(first.plugin_name, second.plugin_name);
    }

    #[test]
    fn a_message_is_logged_at_its_level() {
        // Arrange
        let engine = engine();
        let sink = Arc::new(RecordingSink::default());
        let mut instance = instance_with_sink(&engine, LOGGER, Arc::clone(&sink)).unwrap();

        // Act
        let result = instance
            .call::<(i32, i32, i32), i32>("log", (2, 16, 5))
            .map(status);

        // Assert
        assert_eq!(result.unwrap(), Status::Ok);
        assert_eq!(sink.entries(), vec![(LogLevel::Info, b"hello".to_vec())]);
    }

    #[test]
    fn an_empty_message_is_logged_as_an_empty_entry() {
        // Arrange
        let engine = engine();
        let sink = Arc::new(RecordingSink::default());
        let mut instance = instance_with_sink(&engine, LOGGER, Arc::clone(&sink)).unwrap();

        // Act
        let result = instance
            .call::<(i32, i32, i32), i32>("log", (4, 16, 0))
            .map(status);

        // Assert
        assert_eq!(result.unwrap(), Status::Ok);
        assert_eq!(sink.entries(), vec![(LogLevel::Error, Vec::new())]);
    }

    #[test]
    fn a_bad_level_and_a_bad_address_log_nothing() {
        // Arrange
        let engine = engine();
        let sink = Arc::new(RecordingSink::default());
        let mut instance = instance_with_sink(&engine, LOGGER, Arc::clone(&sink)).unwrap();

        // Act
        let results = (
            outcome(proxy_log(instance.store_mut(), 9, 16, 5)),
            outcome(proxy_log(instance.store_mut(), 2, 65_530, 10)),
            outcome(proxy_log(instance.store_mut(), 2, -1, 5)),
        );

        // Assert
        assert_eq!(
            results,
            (
                Status::BadArgument,
                Status::InvalidMemoryAccess,
                Status::InvalidMemoryAccess
            )
        );
        assert!(sink.entries().is_empty());
    }

    const LEVEL_GUEST: &str = r#"(module
        (import "env" "proxy_get_log_level" (func $level (param i32) (result i32)))
        (memory (export "memory") 1)
        (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 1024)
        (func (export "level") (param i32) (result i32) local.get 0 call $level))"#;

    #[test]
    fn the_level_in_the_services_is_written() {
        // Arrange
        let engine = engine();
        let sink = Arc::new(RecordingSink::default());
        let mut instance = instance_with_sink(&engine, LEVEL_GUEST, sink).unwrap();

        // Act
        let result = instance.call::<i32, i32>("level", 16).map(status);

        // Assert
        assert_eq!(result.unwrap(), Status::Ok);
        assert_eq!(read_level(&mut instance, 16), LogLevel::Info);
    }

    #[test]
    fn a_changed_level_is_written() {
        // Arrange
        let engine = engine();
        let sink = Arc::new(RecordingSink::default());
        let mut instance = instance_with_sink(&engine, LEVEL_GUEST, sink).unwrap();
        instance
            .state_mut()
            .abi_mut()
            .services_mut()
            .set_log_level(LogLevel::Critical);

        // Act
        let result = instance.call::<i32, i32>("level", 16).map(status);

        // Assert
        assert_eq!(result.unwrap(), Status::Ok);
        assert_eq!(read_level(&mut instance, 16), LogLevel::Critical);
    }

    #[test]
    fn a_level_return_pointer_past_memory_is_an_invalid_access() {
        // Arrange
        let engine = engine();
        let sink = Arc::new(RecordingSink::default());
        let mut instance = instance_with_sink(&engine, LEVEL_GUEST, sink).unwrap();

        // Act
        let result = outcome(proxy_get_log_level(instance.store_mut(), 65_534));

        // Assert
        assert_eq!(result, Status::InvalidMemoryAccess);
    }

    fn read_level(instance: &mut crate::runtime::Instance, at: u32) -> LogLevel {
        let value = instance
            .memory()
            .unwrap()
            .read_u32(crate::runtime::GuestPtr::from_address(at))
            .unwrap();
        LogLevel::try_from(value.cast_signed()).unwrap()
    }

    #[test]
    fn a_message_above_the_log_bound_reaches_the_sink_cut_to_it() {
        // Arrange
        let sink = Arc::new(RecordingSink::default());
        let mut instance = bounded(&sink, 3);

        // Act
        let result = log_hello(&mut instance);

        // Assert
        assert_eq!(result, Status::Ok, "a guest never traps on this limit");
        assert_eq!(sink.entries(), vec![(LogLevel::Info, b"hel".to_vec())]);
    }

    #[test]
    fn a_message_at_the_log_bound_reaches_the_sink_whole() {
        // Arrange
        let sink = Arc::new(RecordingSink::default());
        let mut instance = bounded(&sink, 5);

        // Act
        let result = log_hello(&mut instance);

        // Assert
        assert_eq!(result, Status::Ok);
        assert_eq!(sink.entries(), vec![(LogLevel::Info, b"hello".to_vec())]);
    }

    #[test]
    fn a_removed_log_bound_gives_the_sink_the_whole_message() {
        // Arrange
        let sink = Arc::new(RecordingSink::default());
        let mut instance = bounded(&sink, None);

        // Act
        let result = log_hello(&mut instance);

        // Assert
        assert_eq!(result, Status::Ok);
        assert_eq!(sink.entries(), vec![(LogLevel::Info, b"hello".to_vec())]);
    }
}
