# Security policy

This crate runs WebAssembly modules that you did not write.
It is the boundary between your proxy and a plugin, so a defect in it can let a plugin reach what your proxy holds.

## Reporting a vulnerability

Report a vulnerability through the private security advisory form of this repository, under the Security tab.
Do not open a public issue for one.

Include the version or the commit you tested, the module or the source of the guest that shows the problem, the host functions or the callbacks it calls, and what the guest reached that it should not have.
A guest you can share, even a small one, shortens the work more than a description does.

We will let you know whether the report is in scope and what happens next.
This is a small project, so we cannot promise how quickly that reply will come.

## Scope

A vulnerability of this crate is a way for a guest to reach something the host did not lend it.
These are in scope.

- A guest reads or writes the state of another guest, or of another request of the same guest.
- A guest reaches a metric or a shared key of a VM that is not its own.
- A guest makes the host read or write memory outside the guest's own memory.
- A guest makes the host spend memory or time without a limit, through a call that the documented limits say is bounded.
- The crate gives a guest a value that no service of yours supplied.

These are not in scope.

- A defect of wasmtime, which belongs to that project, so report it there.
- A service of yours that gives a guest more than you meant to, since the crate passes what your implementation answers.
- An environment variable that you set on the guest, because the guest reads every one of them.
- A guest that opens a queue of another VM by its name, which the ABI allows.
- A guest that traps or spends its own limits, because a trap is a normal end for a call and your embedder builds a new guest.
- A denial of service that comes from the traffic you send to the plugin rather than from the plugin.

## Supported versions

| Version | Receives fixes |
|---|---|
| 0.1.x | yes |

## What this crate defends

The crate treats the guest as untrusted and your embedder as trusted.
Every value a guest sends arrives as bytes of guest memory, and the crate reads them through bounds checks before any service of yours sees them.
Every identifier a guest passes to the host is checked against the identifiers that guest obtained, so a small number that another VM could guess reaches nothing.
The crate uses no `unsafe` code of its own.

## What your embedder must answer

Some protections depend on how you set up the crate, so they are your responsibility rather than the crate's.

### Separate your tenants

If one process runs plugins for several tenants, give each tenant its own VM id with `VmServices::with_vm_id`.
Shared data and metrics are kept apart by VM id, so one tenant's plugin cannot read another tenant's keys or metrics.

Queues work differently.
Any guest that knows a queue's VM id and name can open it with `proxy_resolve_shared_queue`, and can then add items to it and take items from it.
If you run plugins you do not trust, give them a `SharedServices` store of their own.

### Bound what your services keep

The crate limits what each guest sends, and `InMemoryStore` limits what it holds across all guests.
If you write your own `SharedServices`, the crate cannot see inside it, so you will need to put limits on your store yourself.

The header maps and the buffers you lend to a guest grow with what the guest writes into them.
`VecHeaderMap` and `Vec<u8>` accept every write, so a guest can add a large header many times in one callback and the host holds every copy.
If you serve plugins you do not trust, lend a map and a buffer of your own that refuse a write past the size you accept, with the `NotAllowed` error.

### Recover a guest that traps

When a guest traps, its instance is poisoned and the crate refuses every later callback on it.
Any request waiting on one of that guest's callouts will wait forever unless you end it.
Use `Guest::open_callouts` to find those requests, end them, and then build a replacement with `GuestSpec::build`.

### Watch the rate of rebuilds

A plugin that traps on every request costs you a rebuild on every request.
`GuestSpec::poisoned_guests` counts the poisoned guests you have dropped, so you can compare it after each rebuild and stop serving the plugin if it climbs faster than you are comfortable with.

## The limits you can change

The module documentation of `proxy_wasm_host::abi::v0_2_1` has a table of every limit, its default, and the method that changes it.
`Limits` carries what one guest may spend and how many callouts it may hold open, `EngineConfig` carries the fuel switch and the stack size of the engine, and `InMemoryStoreLimits` carries what `InMemoryStore` keeps.
You can remove most `Limits` values by passing `None`, and a guest then meets no bound of the crate on that call.
The CPU time, the open callouts, and the store limits always have a value.
