# proxy-wasm-host

[![CI](https://github.com/ethanhann/proxy-wasm-host/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/ethanhann/proxy-wasm-host/actions/workflows/ci.yml)
[![Tests](https://img.shields.io/endpoint?url=https://raw.githubusercontent.com/ethanhann/proxy-wasm-host/gh-pages/tests.json)](https://github.com/ethanhann/proxy-wasm-host/actions/workflows/ci.yml)
[![Coverage](https://img.shields.io/endpoint?url=https://raw.githubusercontent.com/ethanhann/proxy-wasm-host/gh-pages/coverage.json)](https://github.com/ethanhann/proxy-wasm-host/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache%202.0-blue)](https://github.com/ethanhann/proxy-wasm-host/blob/main/LICENSE)

A [Proxy-Wasm](https://github.com/proxy-wasm/spec) ABI v0.2.1 host library for Rust, built on [wasmtime](https://wasmtime.dev/).
If you are writing a proxy in Rust and want it to run Proxy-Wasm plugins, this crate implements the host side of the ABI for you.
It is a port of [proxy-wasm-cpp-host](https://github.com/proxy-wasm/proxy-wasm-cpp-host) and [proxy-wasm-go-host](https://github.com/mosn/proxy-wasm-go-host) to Rust.

- ABI: Proxy-Wasm v0.2.1, and guests built for v0.2.0 also load.
- Runtime: wasmtime 49.
- Minimum supported Rust version: 1.96.

## Getting started

```sh
cargo add proxy-wasm-host
```

The [API documentation](https://docs.rs/proxy-wasm-host) opens with a complete example that compiles a plugin, starts it, and runs a request through it.

## Examples

```sh
cargo run --example http_server
curl http://127.0.0.1:2045/
```

The response lists the request headers, including the one the plugin added.
`cargo run --example http_workers` runs the same kind of proxy with a pool of worker threads and one guest per worker.

## Contributing

[CONTRIBUTING.md](https://github.com/ethanhann/proxy-wasm-host/blob/main/CONTRIBUTING.md) covers the build, the checks, and the conventions.
[SECURITY.md](https://github.com/ethanhann/proxy-wasm-host/blob/main/SECURITY.md) covers how to report a vulnerability.

## License

Apache License, Version 2.0.
See `LICENSE` for the text and `NOTICE` for the attribution of the original Go project.
