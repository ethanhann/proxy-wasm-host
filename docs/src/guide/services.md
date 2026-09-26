# Services

Services allow VMs to do stuff outside the VM itself.

## Logging

### The LogSink trait

The `LogSink` trait implementation determines where a guest's log messages are sent.

For example, if you wanted to discard all messages you would implement it like this:

```rust
struct Discard;

impl LogSink for Discard {
    fn log(&self, _context: LogContext<'_>, _log_level: LogLevel, _message: &[u8]) {}
}
```

However, this would not be done except for perhaps during local development.
Realistically, the log level and message would be sent somewhere useful.

This connects guest log output to whatever tracing subscriber (e.g., stdout, JSON, OpenTelemetry, etc.) is configured:

```rust
use std::borrow::Cow;

struct TracingSink;

impl LogSink for TracingSink {
    fn log(&self, context: LogContext<'_>, level: LogLevel, message: &[u8]) {
        let text = String::from_utf8_lossy(message);
        let plugin = match &context.plugin_name {
            Some(name) => String::from_utf8_lossy(name),
            None => Cow::Borrowed("<unconfigured>"),
        };
        match level {
            LogLevel::Trace => tracing::trace!(%plugin, "{text}"),
            LogLevel::Debug => tracing::debug!(%plugin, "{text}"),
            LogLevel::Info => tracing::info!(%plugin, "{text}"),
            LogLevel::Warn => tracing::warn!(%plugin, "{text}"),
            LogLevel::Error | LogLevel::Critical => tracing::error!(%plugin, "{text}"),
        }
    }
}
```

The `LogContext` says which guest, which plugin, and which callback wrote the line, so you can add those as fields of the event.

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
