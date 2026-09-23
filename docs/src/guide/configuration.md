# Configuration

## EngineConfig

The `EngineConfig` struct is used to configure the engine.

An engine has a `epoch_period`, `external_ticks`, `fuel_enabled`, and `max_wasm_stack`.

An engine has a `epoch_period` which is the duration between epochs.
A guest's CPU time limit is measured in these periods.
This relates to the per-guest CPU time budget.

The `external_ticks` setting allows the epoch to be advanced manually from the controlling program.
This is an all-or-nothing setting: either the engine launches a thread to advance the epoch period or the implementing server manually increments the epoch.
Depending on the architecture of a reverse proxy, this might be a useful feature.

The `fuel_enabled` setting is a per-engine switch that turns fuel metering on or off.
If it is set to off, individual fuel limits set for guests will have no effect.

The `max_wasm_stack` setting sets an upperbound on the size, in bytes, of the engine's Wasm stack.
This dictates the maximum amount of stack space that can be used by an engine instance to execute WebAssembly instructions.

## PluginConfig

The `PluginConfig` struct is used to configure a plugin instance.

A plugin has a `name`, a `root_id`, and an unstructured `configuration` blob represented as bytes.

A plugin config can be built like this (assuming it expects a JSON configuration):

```rust
let plugin = plugin
    .with_name(*b"my_plugin")
    .with_root_id(*b"my_plugin_root")
    .with_configuration(*b"{}");
```

## Limits

The limits are used to control the behavior of the VM and prevent it from consuming too many resources.
Limits are set per-guest.

Every limit has a default, so you can start with `Limits::default()` and change only the limits you care about.
Each limit is set with a `with_*` method, and passing `None` removes that limit entirely:

```rust
use proxy_wasm_host::Limits;

let limits = Limits::default()
    .with_max_log_bytes(4 * 1024 * 1024)
    .with_max_name_bytes(None);
```

### CPU Time

CPU time (epoch) bounds wall-clock duration.
It catches guests that run too long.

Note that the CPU time applies to each guest call.
The budget is refilled before every call.

### Memory ceiling

The memory ceiling applies to the guest instance's linear memory.
It limits how much memory it can consume.

### Fuel

Fuel is a [Wasmtime feature](https://docs.wasmtime.dev/examples-interrupting-wasm.html#deterministic-fuel) that counts how many Wasm instructions a guest executes.
The engine burns one unit of fuel for each Wasm instruction.
When the fuel runs out, Wasmtime stops the guest with a trap.
This gives the host a deterministic bound on how much work a guest can do in a single call.

This is different from CPU time because the exact instruction count depends on the underlying hardware.
It is also, generally speaking, slower than epochs.
By default, guests have unlimited fuel and are only bound by CPU time and memory ceiling.

### Guest input limits

A guest decides how much data it hands to the host, whether that is a map of headers, the name of a shared queue, or a log message.
The following limits are checked before that data reaches any of your services:

| Method | Default | What it limits |
|---|---|---|
| `with_max_decoded_pairs` | 1024 | The number of pairs in one map a guest sends |
| `with_max_decoded_map_bytes` | 1 MiB | The size of one map a guest sends |
| `with_max_shared_names` | 1024 | The number of queues and metrics one guest may hold |
| `with_max_name_bytes` | 4096 | The length of one queue name, metric name, or shared data key |
| `with_max_log_bytes` | 1 MiB | The length of one log message |

When a guest goes over one of these limits, the host function returns an error status to the guest instead of calling your service.
Keep in mind that guests built with the Rust SDK treat most error statuses as fatal and panic, which poisons the guest.
If your plugins legitimately need more queues or longer names, raise the limit rather than letting the guest fail:

```rust
let limits = Limits::default()
    .with_max_shared_names(4096)
    .with_max_name_bytes(16 * 1024);
```

The shared name limit counts the queues and metrics a single guest has opened.
Opening a queue or metric the guest already holds does not count against the limit again.

Reading a shared data key that is longer than the name limit returns `NOT_FOUND` rather than an error, since a key that long could never have been stored.

Log messages are the one exception to the rule.
A message longer than the log limit is truncated to the limit and passed to your `LogSink`, and the guest is told the write succeeded.
Guests commonly log the reason they are about to fail, so rejecting a long message would make the guest fail on the very line that explains why.
The same limit applies to anything the guest writes to standard output or standard error.

## VmServices

When you create a guest, you supply a `VmServices` that connects it to the outside world.
The only required argument is a log sink.
Everything else has a sensible default.

A `VmServices` controls where guest log output goes, what time source the guest reads, what environment variables it sees, and where its HTTP and gRPC callouts are sent.
If your proxy has shared state across guests (shared data, queues, or metrics), you provide a `SharedServices` implementation here as well.

For example:

```rust
let services = VmServices::new(sink.clone())
    .with_vm_id(*b"my_vm")
    .with_vm_configuration(*b"{}")
    .with_max_open_callouts(64);
```

See [Services](services.md) for details on logging, callouts, and shared state.

## InMemoryStoreLimits

If you use the shipped `InMemoryStore` for shared services, you can bound what a guest stores.
These limits protect the host process from the guest filling the store with data.

| Limit | Method | Default |
|---|---|---|
| Bytes in one value or queue item | `with_value_bytes` | 64 KiB |
| Keys of the shared data | `with_keys` | 4096 |
| Items in one queue | `with_queue_items` | 1024 |
| Queues | `with_queues` | 4096 |
| Metrics | `with_metrics` | 4096 |

Any bound being exceeded causes a failure status to be reported to the guest.

Unlike the guest limits above, these limits apply to the store as a whole.
The store outlives the guests that use it, so a queue that one guest registers is still there after that guest is dropped and replaced.
Capping the total number of queues and metrics keeps a misbehaving plugin from growing the store every time you rebuild it:

```rust
use proxy_wasm_host::abi::v0_2_1::{InMemoryStore, InMemoryStoreLimits};

let store = InMemoryStore::new().with_limits(
    InMemoryStoreLimits::new()
        .with_queues(1024)
        .with_metrics(1024),
);
```
