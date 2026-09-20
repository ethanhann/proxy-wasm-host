# Getting Started

`proxy-wasm-host` is a crate that allows WebAssembly modules to run as plugins in various proxy servers.
It is a Rust implementation of the [proxy-wasm](https://github.com/proxy-wasm/spec) specification.

The crate is a port of [proxy-wasm-go-host](https://github.com/mosn/proxy-wasm-go-host).
Unlike the Golang implementation, `proxy-wasm-host` only supports version 2.0+ of the spec and has far more tests.

## Installation

```shell
cargo add proxy-wasm-host
```
