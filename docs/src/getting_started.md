# Getting Started

## Introduction

`proxy-wasm-host` is a crate that allows WebAssembly modules to run as plugins in various proxy servers.
It is a Rust implementation of the [proxy-wasm](https://github.com/proxy-wasm/spec) specification.

The crate is a port of [proxy-wasm-go-host](https://github.com/mosn/proxy-wasm-go-host).
Unlike the Golang implementation, `proxy-wasm-host` only supports version 2.0+ of the spec and has far more tests.

You do not need to use this crate unless you are building or maintaining a proxy written in Rust.
See [areweproxyyet.github.io](https://areweproxyyet.github.io/) for a list of proxies that might benefit from this crate.

## Installation

```shell
cargo add proxy-wasm-host
```

## Usage

Usage of this crate assumes that you want to load and run a plugin in the host runtime this crate provides.
Assume you have a file called "foo.wasm", a precompiled WebAssembly module.

```rust
const FOO_PLUGIN: &[u8] = include_bytes!("plugins/foo.wasm");

fn main() {
  // Our "foo.wasm" plugin.
  let module = Module::new(&engine, FOO_PLUGIN).unwrap();

  // Set up the underlying host runtime.
  let engine = Engine::new().unwrap();

  // Create services, e.g., logging, gRPC callouts, etc.
  let sink = Arc::new(Sink::default());
  let services = VmServices::new(sink.clone()).with_vm_configuration(vm_configuration);

  // Create the host from the runtime engine.
  let host = Host::new(&engine).unwrap();

  // Create the guest from the host, module, services, and limits.
  // The guest is a wrapper around the plugin that allows it to run on the host.
  let guest = Guest::new(&host, &module, services, &Limits::default()).unwrap();

  // Now start the proxy-wasm lifecycle of the guest/plugin to actually run it.
  // ...
}
```

