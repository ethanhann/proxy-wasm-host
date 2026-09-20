# Configuration

todo topics to cover:

- VmServices builder: log sink, clock, environment, VM identity, shared services, callouts, max open callouts.
- InMemoryStoreLimits: queue and metric caps.

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

There are three types of guest limits: CPU time, memory ceiling, and fuel.

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
