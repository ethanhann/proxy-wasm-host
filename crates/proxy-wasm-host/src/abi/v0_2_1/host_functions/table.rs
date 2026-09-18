//! The table of every host function, and the linker registration it
//! generates.

use wasmtime::{Caller, Linker};

use crate::Error;
use crate::abi::v0_2_1::host_functions::{
    buffer, callout, clock, complete, context, foreign, header_map, local_response, log, metric,
    property, shared_data, shared_queue, stream, stub, timer,
};
use crate::runtime::HostState;

/// One row of the host function table.
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HostFunction {
    pub(crate) name: &'static str,
    pub(crate) params: &'static [WasmType],
    pub(crate) results: &'static [WasmType],
    /// Whether the row answers `UNIMPLEMENTED` rather than running a body.
    pub(crate) stub: bool,
}

/// A wasm value type a host function parameter can have.
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WasmType {
    I32,
    I64,
}

fn instantiate(source: wasmtime::Error) -> Error {
    Error::Instantiate {
        source: source.into(),
    }
}

macro_rules! wasm_type {
    (I32) => {
        i32
    };
    (I64) => {
        i64
    };
}

macro_rules! host_function {
    ($linker:ident, $name:ident, ( $( $param:ident : $ty:ident ),* ), [stub]) => {
        $linker
            .func_wrap(
                "env",
                stringify!($name),
                |_: Caller<'_, HostState> $(, _: wasm_type!($ty))*| -> Result<i32, wasmtime::Error> {
                    Ok(stub(stringify!($name)))
                },
            )
            .map_err(instantiate)?;
    };
    ($linker:ident, $name:ident, ( $( $param:ident : $ty:ident ),* ), [$body:path]) => {
        $linker
            .func_wrap(
                "env",
                stringify!($name),
                |mut caller: Caller<'_, HostState> $(, $param: wasm_type!($ty))*| -> Result<i32, wasmtime::Error> {
                    complete(stringify!($name), $body(&mut caller $(, $param)*))
                },
            )
            .map_err(instantiate)?;
    };
}

/// Defines the table and the registration from one entry per function.
///
/// An entry names the function, its parameters with their `WasmType`
/// variants, and in brackets either the path of the body that serves it or
/// the word `stub`.
/// The brackets let `host_function!` receive the implementation as one token
/// tree, and its `[stub]` rule must stay before its path rule, because
/// `stub` is also a valid path.
#[cfg(test)]
macro_rules! is_stub {
    ([stub]) => {
        true
    };
    ([$body:path]) => {
        false
    };
}

macro_rules! host_functions {
    ( $( $name:ident ( $( $param:ident : $ty:ident ),* ) = $imp:tt ; )* ) => {
        /// Every host function, in the order of the ABI document.
        #[cfg(test)]
        pub(crate) const HOST_FUNCTIONS: &[HostFunction] = &[
            $(
                HostFunction {
                    name: stringify!($name),
                    params: &[ $( WasmType::$ty ),* ],
                    results: &[WasmType::I32],
                    stub: is_stub!($imp),
                },
            )*
        ];

        /// Registers every host function under the `env` module.
        pub(crate) fn register(linker: &mut Linker<HostState>) -> Result<(), Error> {
            $( host_function!(linker, $name, ( $( $param : $ty ),* ), $imp); )*
            Ok(())
        }
    };
}

host_functions! {
    proxy_done() = [context::proxy_done];
    proxy_set_effective_context(context_id: I32) = [context::proxy_set_effective_context];
    proxy_log(log_level: I32, message_data: I32, message_size: I32) = [log::proxy_log];
    proxy_get_log_level(return_log_level: I32) = [log::proxy_get_log_level];
    proxy_get_current_time_nanoseconds(return_time: I32) = [clock::proxy_get_current_time_nanoseconds];
    proxy_set_tick_period_milliseconds(tick_period: I32) = [timer::proxy_set_tick_period_milliseconds];
    proxy_set_buffer_bytes(buffer_id: I32, start: I32, size: I32, value_data: I32, value_size: I32) = [buffer::proxy_set_buffer_bytes];
    proxy_get_buffer_bytes(buffer_id: I32, start: I32, max_size: I32, return_value_data: I32, return_value_size: I32) = [buffer::proxy_get_buffer_bytes];
    proxy_get_buffer_status(buffer_id: I32, return_buffer_size: I32, return_unused: I32) = [buffer::proxy_get_buffer_status];
    proxy_get_header_map_size(map_id: I32, return_size: I32) = [header_map::proxy_get_header_map_size];
    proxy_get_header_map_pairs(map_id: I32, return_data: I32, return_size: I32) = [header_map::proxy_get_header_map_pairs];
    proxy_set_header_map_pairs(map_id: I32, serialized_pairs_data: I32, serialized_pairs_size: I32) = [header_map::proxy_set_header_map_pairs];
    proxy_get_header_map_value(map_id: I32, key_data: I32, key_size: I32, return_data: I32, return_size: I32) = [header_map::proxy_get_header_map_value];
    proxy_add_header_map_value(map_id: I32, key_data: I32, key_size: I32, value_data: I32, value_size: I32) = [header_map::proxy_add_header_map_value];
    proxy_replace_header_map_value(map_id: I32, key_data: I32, key_size: I32, value_data: I32, value_size: I32) = [header_map::proxy_replace_header_map_value];
    proxy_remove_header_map_value(map_id: I32, key_data: I32, key_size: I32) = [header_map::proxy_remove_header_map_value];
    proxy_continue_stream(stream_type: I32) = [stream::proxy_continue_stream];
    proxy_close_stream(stream_type: I32) = [stream::proxy_close_stream];
    proxy_get_status(return_status_code: I32, return_status_message_data: I32, return_status_message_size: I32) = [callout::proxy_get_status];
    proxy_send_local_response(status_code: I32, status_code_details_data: I32, status_code_details_size: I32, body_data: I32, body_size: I32, serialized_headers_data: I32, serialized_headers_size: I32, grpc_status: I32) = [local_response::proxy_send_local_response];
    proxy_http_call(upstream_name_data: I32, upstream_name_size: I32, serialized_headers_data: I32, serialized_headers_size: I32, body_data: I32, body_size: I32, serialized_trailers_data: I32, serialized_trailers_size: I32, timeout: I32, return_call_id: I32) = [stub];
    proxy_grpc_call(upstream_name_data: I32, upstream_name_size: I32, service_name_data: I32, service_name_size: I32, method_name_data: I32, method_name_size: I32, serialized_initial_metadata_data: I32, serialized_initial_metadata_size: I32, message_data: I32, message_size: I32, timeout: I32, return_call_id: I32) = [stub];
    proxy_grpc_stream(upstream_name_data: I32, upstream_name_size: I32, service_name_data: I32, service_name_size: I32, method_name_data: I32, method_name_size: I32, serialized_initial_metadata_data: I32, serialized_initial_metadata_size: I32, return_stream_id: I32) = [stub];
    proxy_grpc_send(stream_id: I32, message_data: I32, message_size: I32, end_stream: I32) = [stub];
    proxy_grpc_cancel(call_or_stream_id: I32) = [stub];
    proxy_grpc_close(call_or_stream_id: I32) = [stub];
    proxy_set_shared_data(key_data: I32, key_size: I32, value_data: I32, value_size: I32, cas: I32) = [shared_data::proxy_set_shared_data];
    proxy_get_shared_data(key_data: I32, key_size: I32, return_value_data: I32, return_value_size: I32, return_cas: I32) = [shared_data::proxy_get_shared_data];
    proxy_register_shared_queue(name_data: I32, name_size: I32, return_queue_id: I32) = [shared_queue::proxy_register_shared_queue];
    proxy_resolve_shared_queue(vm_id_data: I32, vm_id_size: I32, name_data: I32, name_size: I32, return_queue_id: I32) = [shared_queue::proxy_resolve_shared_queue];
    proxy_enqueue_shared_queue(queue_id: I32, value_data: I32, value_size: I32) = [shared_queue::proxy_enqueue_shared_queue];
    proxy_dequeue_shared_queue(queue_id: I32, return_value_data: I32, return_value_size: I32) = [shared_queue::proxy_dequeue_shared_queue];
    proxy_define_metric(metric_type: I32, name_data: I32, name_size: I32, return_metric_id: I32) = [metric::proxy_define_metric];
    proxy_record_metric(metric_id: I32, value: I64) = [metric::proxy_record_metric];
    proxy_increment_metric(metric_id: I32, delta: I64) = [metric::proxy_increment_metric];
    proxy_get_metric(metric_id: I32, return_value: I32) = [metric::proxy_get_metric];
    proxy_get_property(path_data: I32, path_size: I32, return_value_data: I32, return_value_size: I32) = [property::proxy_get_property];
    proxy_set_property(path_data: I32, path_size: I32, value_data: I32, value_size: I32) = [property::proxy_set_property];
    proxy_call_foreign_function(name_data: I32, name_size: I32, arguments_data: I32, arguments_size: I32, return_results_data: I32, return_results_size: I32) = [foreign::proxy_call_foreign_function];
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::fmt::Write as _;

    use wasmtime::ExternType;

    use super::*;
    use crate::abi::v0_2_1::test_support::status;
    use crate::abi::v0_2_1::types::Status;
    use crate::runtime::test_support::{engine, instance};
    use crate::runtime::{Module, WASI_FUNCTIONS};

    fn wat_type(ty: WasmType) -> &'static str {
        match ty {
            WasmType::I32 => "i32",
            WasmType::I64 => "i64",
        }
    }

    fn import_everything() -> String {
        let mut wat = String::from("(module\n");
        for function in HOST_FUNCTIONS {
            let params: Vec<&str> = function.params.iter().copied().map(wat_type).collect();
            writeln!(
                wat,
                "  (import \"env\" \"{}\" (func (param {}) (result i32)))",
                function.name,
                params.join(" ")
            )
            .unwrap();
        }
        wat.push_str(
            r#"  (memory (export "memory") 1)
  (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 1024)
  (func (export "proxy_abi_version_0_2_1")))"#,
        );
        wat
    }

    #[test]
    fn the_table_has_39_distinct_proxy_functions_returning_i32() {
        // Arrange
        let table = HOST_FUNCTIONS;

        // Act
        let names: BTreeSet<&str> = table.iter().map(|function| function.name).collect();

        // Assert
        assert_eq!(table.len(), 39);
        assert_eq!(names.len(), 39);
        assert!(names.iter().all(|name| name.starts_with("proxy_")));
        assert_eq!(table[0].name, "proxy_done");
        let record = table
            .iter()
            .find(|function| function.name == "proxy_record_metric");
        assert_eq!(record.unwrap().params, [WasmType::I32, WasmType::I64]);
    }

    #[test]
    fn the_rows_with_no_body_are_the_six_callout_functions() {
        // Arrange
        let table = HOST_FUNCTIONS;

        // Act
        let stubs: Vec<&str> = table
            .iter()
            .filter(|function| function.stub)
            .map(|function| function.name)
            .collect();

        // Assert
        assert_eq!(
            stubs,
            vec![
                "proxy_http_call",
                "proxy_grpc_call",
                "proxy_grpc_stream",
                "proxy_grpc_send",
                "proxy_grpc_cancel",
                "proxy_grpc_close",
            ]
        );
    }

    #[test]
    fn a_guest_that_imports_every_function_instantiates() {
        // Arrange
        let engine = engine();
        let wat = import_everything();

        // Act
        let result = instance(&engine, &wat);

        // Assert
        assert!(result.is_ok());
    }

    #[test]
    fn a_stub_answers_unimplemented() {
        // Arrange
        let engine = engine();
        let wat = r#"(module
            (import "env" "proxy_grpc_cancel" (func $f (param i32) (result i32)))
            (memory (export "memory") 1)
            (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 1024)
            (func (export "call") (result i32) i32.const 1 call $f))"#;
        let mut instance = instance(&engine, wat).unwrap();

        // Act
        let result = instance.call::<(), i32>("call", ()).map(status);

        // Assert
        assert_eq!(result.unwrap(), Status::Unimplemented);
        assert!(!instance.is_poisoned());
    }

    #[test]
    fn a_row_with_a_body_no_longer_answers_unimplemented() {
        // Arrange
        let engine = engine();
        let wat = r#"(module
            (import "env" "proxy_get_current_time_nanoseconds" (func $f (param i32) (result i32)))
            (memory (export "memory") 1)
            (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 1024)
            (func (export "call") (result i32) i32.const 16 call $f))"#;
        let mut instance = instance(&engine, wat).unwrap();

        // Act
        let result = instance.call::<(), i32>("call", ()).map(status);

        // Assert
        assert_eq!(result.unwrap(), Status::Ok);
    }

    #[test]
    fn the_rust_sdk_fixture_imports_only_registered_functions() {
        // Arrange
        let engine = engine();
        let bytes = include_bytes!("../../../../tests/fixtures/add-request-header.wasm");
        let module = Module::new(&engine, bytes).unwrap();

        // Act
        let unknown: Vec<String> = module
            .wasmtime()
            .imports()
            .filter(|import| {
                let known = match (import.module(), import.ty()) {
                    ("env", ExternType::Func(ty)) => HOST_FUNCTIONS.iter().any(|function| {
                        function.name == import.name()
                            && ty.params().count() == function.params.len()
                    }),
                    ("wasi_snapshot_preview1", _) => WASI_FUNCTIONS.contains(&import.name()),
                    _ => false,
                };
                !known
            })
            .map(|import| format!("{}::{}", import.module(), import.name()))
            .collect();

        // Assert
        assert!(unknown.is_empty(), "unknown imports: {unknown:?}");
        assert!(module.wasmtime().imports().count() >= 32);
    }
}
