# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- A host for Proxy-Wasm ABI v0.2.1 guests on wasmtime, which also accepts v0.2.0 guests.
- Every callback of the ABI, covering the lifecycle of HTTP and TCP streams, HTTP and gRPC callouts, ticks, queue ready callbacks, and foreign function calls.
- Every host function of the ABI, with the embedder supplying header maps, buffers, stream operations, callouts, properties, and foreign functions through traits.
- An in-memory store for shared data, shared queues, and metrics, with limits that hold across every guest that uses it.
- Limits on the CPU time and fuel of each guest call, on the memory of each guest, and on the size of the maps, names, and log messages a guest sends.
- Recovery after a guest traps, with `GuestSpec` to build a replacement and a count of the poisoned guests you drop.
- Two example proxies in the repository, one with a single guest and one with a pool of workers.

[Unreleased]: https://github.com/ethanhann/proxy-wasm-host/commits/main
