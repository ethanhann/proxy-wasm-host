//! The eight `wasi_snapshot_preview1` functions the ABI document names.
//!
//! Each one has the WASI signature and the ABI document's meaning.
//! `fd_write` is a log call, the clock and randomness functions read the
//! services in the host state, the environment functions serve the per guest
//! variables, and the argument functions report no arguments.
//! A guest that imports any other WASI function fails to instantiate.

use std::fmt::Display;

use wasmtime::{Caller, Linker};

use crate::Error;
use crate::abi::v0_2_1::types::{LogLevel, WasiClockId, WasiErrno, WasiFdId};
use crate::runtime::HostState;
use crate::runtime::memory::{GuestMemory, GuestPtr, GuestSlice, split};

const MODULE: &str = "wasi_snapshot_preview1";

/// The eight function names this module registers under `MODULE`.
#[cfg(test)]
pub(crate) const WASI_FUNCTIONS: &[&str] = &[
    "fd_write",
    "clock_time_get",
    "random_get",
    "environ_sizes_get",
    "environ_get",
    "args_sizes_get",
    "args_get",
    "proc_exit",
];
const IOVEC_SIZE: u32 = 8;
const MAX_RANDOM_BYTES: u32 = 64 * 1024;
const MAX_LOG_BYTES: u64 = 1024 * 1024;

type WasiResult = Result<(), WasiErrno>;

/// Registers the eight functions.
pub(crate) fn add_to_linker(linker: &mut Linker<HostState>) -> Result<(), Error> {
    let instantiate = |source: wasmtime::Error| Error::Instantiate {
        source: source.into(),
    };
    linker
        .func_wrap(MODULE, "fd_write", fd_write)
        .map_err(instantiate)?;
    linker
        .func_wrap(MODULE, "clock_time_get", clock_time_get)
        .map_err(instantiate)?;
    linker
        .func_wrap(MODULE, "random_get", random_get)
        .map_err(instantiate)?;
    linker
        .func_wrap(MODULE, "environ_sizes_get", environ_sizes_get)
        .map_err(instantiate)?;
    linker
        .func_wrap(MODULE, "environ_get", environ_get)
        .map_err(instantiate)?;
    linker
        .func_wrap(MODULE, "args_sizes_get", args_sizes_get)
        .map_err(instantiate)?;
    linker
        .func_wrap(MODULE, "args_get", args_get)
        .map_err(instantiate)?;
    linker
        .func_wrap(MODULE, "proc_exit", proc_exit)
        .map_err(instantiate)?;
    Ok(())
}

fn errno(result: WasiResult) -> i32 {
    i32::from(result.err().unwrap_or(WasiErrno::Success))
}

fn fault(error: impl Display) -> WasiErrno {
    tracing::debug!(%error, "wasi call faulted on a guest address");
    WasiErrno::Fault
}

fn pointer(raw: i32) -> Result<GuestPtr, WasiErrno> {
    GuestPtr::try_from(raw).map_err(fault)
}

fn read_u32(entry: &[u8]) -> u32 {
    u32::from_le_bytes([entry[0], entry[1], entry[2], entry[3]])
}

fn fd_write(
    mut caller: Caller<'_, HostState>,
    fd: i32,
    iovs: i32,
    iovs_len: i32,
    nwritten: i32,
) -> i32 {
    errno(fd_write_impl(&mut caller, fd, iovs, iovs_len, nwritten))
}

fn fd_write_impl(
    caller: &mut Caller<'_, HostState>,
    fd: i32,
    iovs: i32,
    iovs_len: i32,
    nwritten: i32,
) -> WasiResult {
    let level = match WasiFdId::try_from(fd) {
        Ok(WasiFdId::Stdout) => LogLevel::Info,
        Ok(WasiFdId::Stderr) => LogLevel::Error,
        Err(_) => return Err(WasiErrno::Badf),
    };
    let (mut memory, state) = split(caller).map_err(fault)?;
    let written_ptr = pointer(nwritten)?;
    memory.read_u32(written_ptr).map_err(fault)?;
    let count = u32::try_from(iovs_len).map_err(fault)?;
    let table_len = count.checked_mul(IOVEC_SIZE).ok_or(WasiErrno::Fault)?;
    let table = GuestSlice::new(pointer(iovs)?, table_len).map_err(fault)?;
    let entries = memory
        .read(table)
        .map_err(fault)?
        .as_chunks::<{ IOVEC_SIZE as usize }>()
        .0
        .iter()
        .map(|entry| {
            GuestSlice::new(
                GuestPtr::from_address(read_u32(entry)),
                read_u32(&entry[4..]),
            )
            .map_err(fault)
        })
        .collect::<Result<Vec<GuestSlice>, WasiErrno>>()?;
    let total: u64 = entries.iter().map(|entry| u64::from(entry.len())).sum();
    if total > MAX_LOG_BYTES {
        return Err(WasiErrno::Inval);
    }
    let mut message = Vec::with_capacity(usize::try_from(total).unwrap_or(0));
    for entry in &entries {
        message.extend_from_slice(memory.read(*entry).map_err(fault)?);
    }
    if message.last() == Some(&b'\n') {
        message.pop();
    }
    if count > 0 {
        state.services().log().log(level, &message);
    }
    let written = u32::try_from(total).unwrap_or(u32::MAX);
    memory.write_u32(written_ptr, written).map_err(fault)
}

fn clock_time_get(mut caller: Caller<'_, HostState>, id: i32, _precision: i64, time: i32) -> i32 {
    errno(clock_time_get_impl(&mut caller, id, time))
}

fn clock_time_get_impl(caller: &mut Caller<'_, HostState>, id: i32, time: i32) -> WasiResult {
    let id = WasiClockId::try_from(id).map_err(|_| WasiErrno::Notsup)?;
    let (mut memory, state) = split(caller).map_err(fault)?;
    let nanos = match id {
        WasiClockId::Realtime => state.services().clock().realtime_nanos(),
        WasiClockId::Monotonic => state.services().clock().monotonic_nanos(),
    };
    memory.write_u64(pointer(time)?, nanos).map_err(fault)
}

fn random_get(mut caller: Caller<'_, HostState>, buf: i32, len: i32) -> i32 {
    errno(random_get_impl(&mut caller, buf, len))
}

fn random_get_impl(caller: &mut Caller<'_, HostState>, buf: i32, len: i32) -> WasiResult {
    let slice = GuestSlice::try_from((buf, len)).map_err(fault)?;
    if slice.len() > MAX_RANDOM_BYTES {
        return Err(WasiErrno::Inval);
    }
    let (mut memory, _) = split(caller).map_err(fault)?;
    let target = memory.slice_mut(slice).map_err(fault)?;
    getrandom::fill(target).map_err(|_| WasiErrno::Inval)
}

/// The `KEY=VALUE\0` block of every variable, in order.
fn environment_block(state: &HostState) -> Vec<u8> {
    let mut block = Vec::new();
    for (key, value) in state.services().environment() {
        block.extend_from_slice(key);
        block.push(b'=');
        block.extend_from_slice(value);
        block.push(0);
    }
    block
}

fn environ_sizes_get(mut caller: Caller<'_, HostState>, count_ptr: i32, size_ptr: i32) -> i32 {
    errno(environ_sizes_get_impl(&mut caller, count_ptr, size_ptr))
}

fn environ_sizes_get_impl(
    caller: &mut Caller<'_, HostState>,
    count_ptr: i32,
    size_ptr: i32,
) -> WasiResult {
    let (mut memory, state) = split(caller).map_err(fault)?;
    let count = u32::try_from(state.services().environment().len()).map_err(fault)?;
    let size = u32::try_from(environment_block(state).len()).map_err(fault)?;
    memory
        .write_u32(pointer(count_ptr)?, count)
        .map_err(fault)?;
    memory.write_u32(pointer(size_ptr)?, size).map_err(fault)
}

fn environ_get(mut caller: Caller<'_, HostState>, array: i32, buffer: i32) -> i32 {
    errno(environ_get_impl(&mut caller, array, buffer))
}

fn environ_get_impl(caller: &mut Caller<'_, HostState>, array: i32, buffer: i32) -> WasiResult {
    let (mut memory, state) = split(caller).map_err(fault)?;
    let buffer_start = pointer(buffer)?;
    let block = environment_block(state);
    let mut pointers = Vec::new();
    let mut offset = buffer_start.address();
    for (key, value) in state.services().environment() {
        pointers.extend_from_slice(&offset.to_le_bytes());
        let entry_len = u32::try_from(key.len() + value.len() + 2).map_err(fault)?;
        offset = offset.checked_add(entry_len).ok_or(WasiErrno::Fault)?;
    }
    write_block(&mut memory, pointer(array)?, &pointers)?;
    write_block(&mut memory, buffer_start, &block)
}

fn write_block(memory: &mut GuestMemory<'_>, start: GuestPtr, block: &[u8]) -> WasiResult {
    let len = u32::try_from(block.len()).map_err(fault)?;
    let slice = GuestSlice::new(start, len).map_err(fault)?;
    memory.write(slice, block).map_err(fault)
}

fn args_sizes_get(mut caller: Caller<'_, HostState>, argc: i32, size: i32) -> i32 {
    errno(args_sizes_get_impl(&mut caller, argc, size))
}

fn args_sizes_get_impl(caller: &mut Caller<'_, HostState>, argc: i32, size: i32) -> WasiResult {
    let (mut memory, _) = split(caller).map_err(fault)?;
    memory.write_u32(pointer(argc)?, 0).map_err(fault)?;
    memory.write_u32(pointer(size)?, 0).map_err(fault)
}

fn args_get(_argv: i32, _buf: i32) -> i32 {
    errno(Ok(()))
}

fn proc_exit(code: i32) -> Result<(), wasmtime::Error> {
    Err(wasmtime::Error::new(Error::GuestExit { code }))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::runtime::test_support::{
        RecordingSink, engine, instance, instance_with_sink, wat_bytes,
    };
    use crate::runtime::{Clock, HostServices, Instance, Limits, Module};

    const HEADER: &str = r#"
        (memory (export "memory") 1)
        (func (export "proxy_on_memory_allocate") (param i32) (result i32) i32.const 1024)"#;

    fn guest(imports: &str, body: &str) -> String {
        format!("(module {imports} {HEADER} {body})")
    }

    fn ptr(address: u32) -> GuestPtr {
        GuestPtr::from_address(address)
    }

    struct FixedClock;

    impl Clock for FixedClock {
        fn realtime_nanos(&self) -> u64 {
            1_700_000_000_000_000_000
        }

        fn monotonic_nanos(&self) -> u64 {
            42
        }
    }

    const FD_WRITE_IMPORT: &str = r#"(import "wasi_snapshot_preview1" "fd_write" (func $w (param i32 i32 i32 i32) (result i32)))"#;

    #[test]
    fn fd_write_logs_joined_entries_and_counts_bytes() {
        // Arrange
        let engine = engine();
        let sink = Arc::new(RecordingSink::default());
        let wat = guest(
            FD_WRITE_IMPORT,
            r#"(data (i32.const 0) "hello, \nworld\n")
               (func (export "write") (param i32) (result i32)
                 (i32.store (i32.const 100) (i32.const 0)) (i32.store (i32.const 104) (i32.const 8))
                 (i32.store (i32.const 108) (i32.const 8)) (i32.store (i32.const 112) (i32.const 6))
                 (call $w (local.get 0) (i32.const 100) (i32.const 2) (i32.const 200)))"#,
        );
        let mut instance = instance_with_sink(&engine, &wat, Arc::clone(&sink)).unwrap();

        // Act
        let results = [
            instance.call::<(i32,), i32>("write", (1,)).unwrap(),
            instance.call::<(i32,), i32>("write", (2,)).unwrap(),
        ];

        // Assert
        assert_eq!(results, [0, 0]);
        assert_eq!(instance.memory().unwrap().read_u32(ptr(200)), Ok(14));
        assert_eq!(
            sink.entries(),
            vec![
                (LogLevel::Info, b"hello, \nworld".to_vec()),
                (LogLevel::Error, b"hello, \nworld".to_vec())
            ]
        );
    }

    #[test]
    fn fd_write_with_no_entries_logs_nothing_and_writes_zero() {
        // Arrange
        let engine = engine();
        let sink = Arc::new(RecordingSink::default());
        let wat = guest(
            FD_WRITE_IMPORT,
            r#"(func (export "write") (result i32) (call $w (i32.const 1) (i32.const 100) (i32.const 0) (i32.const 200)))"#,
        );
        let mut instance = instance_with_sink(&engine, &wat, Arc::clone(&sink)).unwrap();

        // Act
        let result = instance.call::<(), i32>("write", ()).unwrap();

        // Assert
        assert_eq!(result, 0);
        assert_eq!(instance.memory().unwrap().read_u32(ptr(200)), Ok(0));
        assert!(sink.entries().is_empty());
    }

    #[test]
    fn fd_write_rejects_bad_descriptors_and_bad_addresses_without_logging() {
        // Arrange
        let engine = engine();
        let sink = Arc::new(RecordingSink::default());
        let wat = guest(
            FD_WRITE_IMPORT,
            r#"(func (export "write") (param i32 i32 i32) (result i32)
                 (i32.store (i32.const 100) (i32.const 70000)) (i32.store (i32.const 104) (i32.const 4))
                 (i32.store (i32.const 116) (i32.const 0)) (i32.store (i32.const 120) (i32.const 4))
                 (call $w (local.get 0) (local.get 1) (i32.const 1) (local.get 2)))"#,
        );
        let mut instance = instance_with_sink(&engine, &wat, Arc::clone(&sink)).unwrap();

        // Act
        let results = [
            instance
                .call::<(i32, i32, i32), i32>("write", (3, 116, 200))
                .unwrap(),
            instance
                .call::<(i32, i32, i32), i32>("write", (1, 70_000, 200))
                .unwrap(),
            instance
                .call::<(i32, i32, i32), i32>("write", (1, 100, 200))
                .unwrap(),
            instance
                .call::<(i32, i32, i32), i32>("write", (1, 116, 70_000))
                .unwrap(),
        ];

        // Assert
        let fault = i32::from(WasiErrno::Fault);
        assert_eq!(results, [i32::from(WasiErrno::Badf), fault, fault, fault]);
        assert!(sink.entries().is_empty());
    }

    #[test]
    fn fd_write_refuses_a_message_above_the_cap() {
        // Arrange
        let engine = engine();
        let sink = Arc::new(RecordingSink::default());
        let wat = guest(
            FD_WRITE_IMPORT,
            r#"(func (export "write") (result i32)
                 (local $i i32)
                 (loop $fill
                   (i32.store (i32.add (i32.const 1024) (i32.mul (local.get $i) (i32.const 8))) (i32.const 0))
                   (i32.store (i32.add (i32.const 1028) (i32.mul (local.get $i) (i32.const 8))) (i32.const 1024))
                   (local.set $i (i32.add (local.get $i) (i32.const 1)))
                   (br_if $fill (i32.lt_u (local.get $i) (i32.const 2048))))
                 (call $w (i32.const 1) (i32.const 1024) (i32.const 2048) (i32.const 200)))"#,
        );
        let mut instance = instance_with_sink(&engine, &wat, Arc::clone(&sink)).unwrap();

        // Act
        let result = instance.call::<(), i32>("write", ()).unwrap();

        // Assert
        assert_eq!(result, i32::from(WasiErrno::Inval));
        assert!(sink.entries().is_empty());
    }

    #[test]
    fn clock_time_get_serves_both_clocks_and_rejects_others() {
        // Arrange
        let engine = engine();
        let wat = guest(
            r#"(import "wasi_snapshot_preview1" "clock_time_get" (func $c (param i32 i64 i32) (result i32)))"#,
            r#"(func (export "clock") (param i32 i32) (result i32) (call $c (local.get 0) (i64.const 0) (local.get 1)))"#,
        );
        let module = Module::new(&engine, &wat_bytes(&wat)).unwrap();
        let services =
            HostServices::new(Arc::new(RecordingSink::default())).with_clock(Arc::new(FixedClock));
        let mut instance = Instance::new(&engine, &module, services, &Limits::default()).unwrap();

        // Act
        let results = [
            instance.call::<(i32, i32), i32>("clock", (0, 64)).unwrap(),
            instance.call::<(i32, i32), i32>("clock", (1, 72)).unwrap(),
            instance.call::<(i32, i32), i32>("clock", (2, 80)).unwrap(),
        ];

        // Assert
        assert_eq!(results, [0, 0, i32::from(WasiErrno::Notsup)]);
        let memory = instance.memory().unwrap();
        assert_eq!(memory.read_u64(ptr(64)), Ok(1_700_000_000_000_000_000));
        assert_eq!(memory.read_u64(ptr(72)), Ok(42));
    }

    #[test]
    fn random_get_fills_in_place_and_bounds_the_length() {
        // Arrange
        let engine = engine();
        let wat = guest(
            r#"(import "wasi_snapshot_preview1" "random_get" (func $r (param i32 i32) (result i32)))"#,
            r#"(func (export "random") (param i32 i32) (result i32) (call $r (local.get 0) (local.get 1)))"#,
        );
        let mut instance = instance(&engine, &wat).unwrap();
        let first_slice = GuestSlice::new(ptr(0), 32).unwrap();
        let second_slice = GuestSlice::new(ptr(32), 32).unwrap();

        // Act
        let results = [
            instance.call::<(i32, i32), i32>("random", (0, 32)).unwrap(),
            instance
                .call::<(i32, i32), i32>("random", (32, 32))
                .unwrap(),
            instance.call::<(i32, i32), i32>("random", (64, 0)).unwrap(),
            instance
                .call::<(i32, i32), i32>("random", (0, 65_537))
                .unwrap(),
            instance
                .call::<(i32, i32), i32>("random", (65_530, 16))
                .unwrap(),
        ];

        // Assert
        assert_eq!(
            results,
            [
                0,
                0,
                0,
                i32::from(WasiErrno::Inval),
                i32::from(WasiErrno::Fault)
            ]
        );
        let memory = instance.memory().unwrap();
        assert_ne!(
            memory.read(first_slice).unwrap(),
            memory.read(second_slice).unwrap()
        );
        assert_ne!(memory.read(first_slice).unwrap(), [0u8; 32]);
    }

    #[test]
    fn environment_functions_serialize_the_variables() {
        // Arrange
        let engine = engine();
        let wat = guest(
            r#"(import "wasi_snapshot_preview1" "environ_sizes_get" (func $s (param i32 i32) (result i32)))
               (import "wasi_snapshot_preview1" "environ_get" (func $g (param i32 i32) (result i32)))"#,
            r#"(func (export "sizes") (result i32) (call $s (i32.const 8) (i32.const 12)))
               (func (export "get") (result i32) (call $g (i32.const 100) (i32.const 200)))"#,
        );
        let module = Module::new(&engine, &wat_bytes(&wat)).unwrap();
        let variables = vec![
            (b"A".to_vec(), b"1".to_vec()),
            (b"KEY".to_vec(), b"value".to_vec()),
        ];
        let services =
            HostServices::new(Arc::new(RecordingSink::default())).with_environment(variables);
        let mut instance = Instance::new(&engine, &module, services, &Limits::default()).unwrap();

        // Act
        let results = [
            instance.call::<(), i32>("sizes", ()).unwrap(),
            instance.call::<(), i32>("get", ()).unwrap(),
        ];

        // Assert
        assert_eq!(results, [0, 0]);
        let memory = instance.memory().unwrap();
        assert_eq!(
            (memory.read_u32(ptr(8)), memory.read_u32(ptr(12))),
            (Ok(2), Ok(14))
        );
        assert_eq!(
            (memory.read_u32(ptr(100)), memory.read_u32(ptr(104))),
            (Ok(200), Ok(204))
        );
        assert_eq!(
            memory.read(GuestSlice::new(ptr(200), 14).unwrap()),
            Ok(b"A=1\0KEY=value\0".as_slice())
        );
    }

    #[test]
    fn an_empty_environment_reports_zero_sizes() {
        // Arrange
        let engine = engine();
        let wat = guest(
            r#"(import "wasi_snapshot_preview1" "environ_sizes_get" (func $s (param i32 i32) (result i32)))"#,
            r#"(func (export "sizes") (result i32) (call $s (i32.const 8) (i32.const 12)))"#,
        );
        let mut instance = instance(&engine, &wat).unwrap();

        // Act
        let result = instance.call::<(), i32>("sizes", ()).unwrap();

        // Assert
        assert_eq!(result, 0);
        let memory = instance.memory().unwrap();
        assert_eq!(
            (memory.read_u32(ptr(8)), memory.read_u32(ptr(12))),
            (Ok(0), Ok(0))
        );
    }

    #[test]
    fn argument_functions_report_no_arguments() {
        // Arrange
        let engine = engine();
        let wat = guest(
            r#"(import "wasi_snapshot_preview1" "args_sizes_get" (func $s (param i32 i32) (result i32)))
               (import "wasi_snapshot_preview1" "args_get" (func $g (param i32 i32) (result i32)))"#,
            r#"(func (export "sizes") (result i32) (i32.store (i32.const 8) (i32.const 9)) (i32.store (i32.const 12) (i32.const 9)) (call $s (i32.const 8) (i32.const 12)))
               (func (export "get") (result i32) (call $g (i32.const 100) (i32.const 200)))"#,
        );
        let mut instance = instance(&engine, &wat).unwrap();

        // Act
        let results = [
            instance.call::<(), i32>("sizes", ()).unwrap(),
            instance.call::<(), i32>("get", ()).unwrap(),
        ];

        // Assert
        assert_eq!(results, [0, 0]);
        let memory = instance.memory().unwrap();
        assert_eq!(
            (memory.read_u32(ptr(8)), memory.read_u32(ptr(12))),
            (Ok(0), Ok(0))
        );
    }

    #[test]
    fn proc_exit_in_start_is_a_guest_exit() {
        // Arrange
        let engine = engine();
        let wat = guest(
            r#"(import "wasi_snapshot_preview1" "proc_exit" (func $e (param i32)))"#,
            r#"(func (export "_start") (call $e (i32.const 9)))"#,
        );

        // Act
        let result = instance(&engine, &wat);

        // Assert
        assert!(matches!(result, Err(Error::GuestExit { code: 9 })));
    }

    #[test]
    fn every_listed_wasi_name_is_registered() {
        // Arrange
        let engine = crate::runtime::test_support::engine();
        let services = crate::runtime::test_support::services();
        let mut store = wasmtime::Store::new(engine.wasmtime(), HostState::new(services));

        // Act
        let defined: Vec<bool> = WASI_FUNCTIONS
            .iter()
            .map(|name| engine.linker().get(&mut store, MODULE, name).is_ok())
            .collect();

        // Assert
        assert_eq!(WASI_FUNCTIONS.len(), 8);
        assert!(defined.iter().all(|defined| *defined));
    }
}
