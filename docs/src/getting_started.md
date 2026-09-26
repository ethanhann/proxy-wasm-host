# Getting Started

## Introduction

`proxy-wasm-host` is a crate that allows WebAssembly modules to run as plugins in various proxy servers.
It is a Rust implementation of the [Proxy-Wasm](https://github.com/proxy-wasm/spec) specification.

The crate is a port of [proxy-wasm-go-host](https://github.com/mosn/proxy-wasm-go-host).
It hosts guests built for ABI v0.2.1, and it also accepts guests built for v0.2.0.

You do not need to use this crate unless you are building or maintaining a proxy written in Rust.
See [areweproxyyet.github.io](https://areweproxyyet.github.io/) for a list of proxies that might benefit from this crate.

## Installation

```shell
cargo add proxy-wasm-host
```

## Usage

Usage of this crate assumes that you want to load and run a plugin in the host runtime this crate provides.

For example, given a pre-compiled plugin file called "foo.wasm", you would load it like this:

```rust
use std::sync::Arc;

use proxy_wasm_host::abi::v0_2_1::types::LogLevel;
use proxy_wasm_host::abi::v0_2_1::{Guest, Host, LogContext, LogSink, PluginConfig, VmServices};
use proxy_wasm_host::{Engine, Limits, Module};

const FOO_PLUGIN: &[u8] = include_bytes!("plugins/foo.wasm");

// Where the log lines of the plugin go.
struct Stderr;

impl LogSink for Stderr {
    fn log(&self, _: LogContext<'_>, _: LogLevel, message: &[u8]) {
        eprintln!("{}", String::from_utf8_lossy(message));
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Set up the runtime and compile the plugin.
    let engine = Engine::new()?;
    let module = Module::new(&engine, FOO_PLUGIN)?;

    // Create the services the plugin reads, such as its log sink and its VM configuration.
    let services = VmServices::new(Arc::new(Stderr)).with_vm_configuration(*b"{}");

    // Link the host functions once, then bind the guest to them with its limits.
    let host = Host::new(&engine)?;
    let mut guest = Guest::new(&host, &module, services, &Limits::default())?;

    // Start the plugin, which creates its root context and runs its VM start and its configuration.
    let started = guest.start(PluginConfig::new().with_name(*b"foo"))?;
    println!("{started:?}");
    Ok(())
}
```

