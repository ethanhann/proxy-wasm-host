# Services

Services allow VMs to do stuff outside the VM itself.

## Logging

todo topics to cover:

- The LogSink trait and what it receives (a log level and a message).
- How the guest calls proxy_log and how that reaches your sink.
- Setting an initial log level on VmServices and how the guest can change it at runtime.
- Connecting LogSink to a proxy's logger.

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
- How multiple Guest instances share state through a common Arc<SharedServices>.
