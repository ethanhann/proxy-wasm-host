# proxy-wasm-host

[![CI](https://github.com/ethanhann/proxy-wasm-host/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/ethanhann/proxy-wasm-host/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache%202.0-blue)](LICENSE)


A [Proxy-Wasm](https://github.com/proxy-wasm/spec) ABI v0.2.1 host library for Rust, built on [wasmtime](https://wasmtime.dev/).
If you write a proxy in Rust and want to run Proxy-Wasm guests in it, this crate provides the host side of the ABI.
It is a port of [proxy-wasm-go-host](https://github.com/mosn/proxy-wasm-go-host) from Go to Rust.

## Status

The crate is not published yet.
It runs every callback of ABI v0.2.1, which covers the lifecycle of an HTTP stream and of a TCP stream, the HTTP and gRPC callouts, the ticks, the queue ready callbacks, and a foreign function call.
It serves logging, the log level, the clock, the tick period, the header maps, the buffers, the stream operations, the HTTP and gRPC callouts, the local response, the shared data, the shared queues, the metrics, the properties, and the foreign function call.
The embedder drives each callback and supplies the state a guest reads.
When it is published, the facts below decide whether you can use it.

- ABI: Proxy-Wasm v0.2.1, with v0.2.0 guests accepted.
- Runtime: wasmtime 49.
- Minimum supported Rust version: 1.96.
  The minimum follows the `rust-version` that wasmtime declares, so it moves when the wasmtime dependency moves.

## Build and test

The project uses [just](https://github.com/casey/just).

```sh
just check
```

`check` runs the format check, clippy with warnings denied, the build, the tests, and rustdoc with warnings denied.
CI runs the same recipe.
Run `just` with no arguments to list every recipe.
`just build-guests` rebuilds the test guests under `crates/test-guests`.
It needs rustup, and it installs the pinned toolchain and the `wasm32-wasip1` target on first use.
`just bench` runs the benchmarks.

## Examples

```sh
cargo run --example http_server
curl http://127.0.0.1:2045/
```

```text
INFO http_server: listening on 127.0.0.1:2045 with the plugin .../add-request-header.wasm
INFO request{path="/"}: http_server: GET /
INFO request{path="/"}: guest: adding header
```

The answer lists the headers, and it carries the header the plugin added.
`cargo run --example http_workers` runs the same request through a pool of workers, where each worker has its own guest.

## License

Apache License, Version 2.0.
See `LICENSE` for the text and `NOTICE` for the attribution to the original Go project.
