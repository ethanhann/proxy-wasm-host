# Security policy

## Reporting a vulnerability

Report a vulnerability through the private security advisory form of this repository, under the Security tab on GitHub.
Do not open a public issue.

Include the version you tested, the guest module or its source, and what the guest was able to access.
A guest that reproduces the problem helps more than a description.
This is a small project, so there is no promised response time.

## Scope

The crate treats the guest as untrusted and your embedder as trusted.
A vulnerability is any way for a guest to access something that the host did not give it, such as:

- The state of another guest, or of another request of the same guest.
- A metric or a shared data key of another VM.
- Host memory outside the guest's own memory.
- Memory or time without limit, through a call that the documented limits say is bounded.

Out of scope:

- A defect in wasmtime, which you report to that project.
- A service of yours that returns more than you intended.
- A guest that opens a queue of another VM by name, which the ABI allows.
- A guest that traps or exhausts its own limits.
- A denial of service caused by the traffic you send to the plugin.

## Supported versions

| Version | Receives fixes |
|---|---|
| 0.1.x | yes |

## What your embedder is responsible for

Give each tenant its own VM id with `VmServices::with_vm_id`.
Shared data and metrics are partitioned by VM id, but any guest that knows the VM id and the name of a queue can open it, so give plugins you do not trust a `SharedServices` store of their own.

The crate limits what a guest sends, and `InMemoryStore` limits what it stores.
The header maps and buffers you provide accept every write, so provide your own implementations that return `NotAllowed` past the size you accept, and enforce limits in any `SharedServices` you implement yourself.

When a guest traps, the crate refuses every later callback on it.
Use `Guest::open_callouts` to end the requests that are waiting on it, and build a replacement with `GuestSpec::build`.
`GuestSpec::poisoned_guests` counts the guests you have replaced, so you can stop serving a plugin that traps too often.

The module documentation of `proxy_wasm_host::abi::v0_2_1` has a table of every limit, its default, and the method that changes it.
