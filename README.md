# proxy-wasm-host

A [Proxy-Wasm](https://github.com/proxy-wasm/spec) ABI v0.2.1 host library for Rust, built on [wasmtime](https://wasmtime.dev/).
If you write a proxy in Rust and want to run Proxy-Wasm guests in it, this crate provides the host side of the ABI.
It is a port of [proxy-wasm-go-host](https://github.com/mosn/proxy-wasm-go-host) from Go to Rust.

## Status

The crate is not published yet.
It runs the request header lifecycle of ABI v0.2.1 and does not yet serve every host function.
It serves logging, the log level, the clock, the tick period, the header maps, the buffers, the stream operations, the callout status, the local response, the shared data, the shared queues, the metrics, the properties, and the foreign function call.
The HTTP and gRPC callouts answer `UNIMPLEMENTED`, and the callbacks beyond the request header lifecycle are not delivered yet.
When it is published, the facts below decide whether you can use it.

- ABI: Proxy-Wasm v0.2.1, with v0.2.0 guests accepted.
- Runtime: wasmtime 48.
- Minimum supported Rust version: 1.95.
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

## License

Apache License, Version 2.0.
See `LICENSE` for the text and `NOTICE` for the attribution to the original Go project.
