# Services

Services allow VMs to do stuff outside the VM itself.

## Logging

### The LogSink trait

The `LogSink` trait implementation determines where a guest's log messages are sent.

For example, if you wanted to discard all messages you would implement it like this:

```rust
struct Discard;

impl LogSink for Discard {
    fn log(&self, _log_level: LogLevel, _message: &[u8]) {}
}
```

However, this would not be done except for perhaps during local development.
Realistically, the log level and message would be sent somewhere useful.

This connects guest log output to whatever tracing subscriber (e.g., stdout, JSON, OpenTelemetry, etc.) is configured:

```rust
struct TracingSink;

impl LogSink for TracingSink {
    fn log(&self, level: LogLevel, message: &[u8]) {
        let text = String::from_utf8_lossy(message);
        match level {
            LogLevel::Trace => tracing::trace!("{text}"),
            LogLevel::Debug => tracing::debug!("{text}"),
            LogLevel::Info => tracing::info!("{text}"),
            LogLevel::Warn => tracing::warn!("{text}"),
            LogLevel::Error | LogLevel::Critical => tracing::error!("{text}"),
        }
    }
}
```

### Using with VmServices

todo: Setting an initial log level on VmServices and how the guest can change it at runtime.

### Internals

todo: How the guest calls proxy_log and how that reaches your sink.

## Callouts

todo topics to cover:

- How a guest initiates an HTTP or gRPC callout.
- How the embedder dispatches it externally and delivers the response through on_http_call_response or the gRPC callbacks.
- Max open callouts and what happens when the limit is reached.

## Shared Services

todo topics to cover:

- SharedServices trait: shared data, queues, and metrics across guests.
- InMemoryStore as the shipped implementation.
- InMemoryStoreLimits: queue capacity, queue count, metric count, shared data count, value size.
- Queue registration and resolution across guests.
- How multiple Guest instances share state through a common `Arc<SharedServices>`.
