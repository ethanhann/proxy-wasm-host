# Host Functions

todo topics to cover:

- The 39 ABI v0.2.1 host functions grouped by category.
- Headers: get, set, add, remove, replace pairs.
- Buffers: get, set.
- Logging: proxy_log.
- Clock: proxy_get_current_time_nanoseconds.
- Properties: get, set.
- Stream control: continue, close, send local response.
- HTTP callouts: proxy_http_call.
- gRPC: call, stream, send, close, cancel.
- Shared data: get, set.
- Shared queues: register, resolve, enqueue, dequeue.
- Metrics: define, get, increment, record.
- Foreign function: proxy_call_foreign_function.
- Context: set effective context.
- Timer: set tick period.
- What StreamState method each host function calls.
- What errors each host function can return to the guest.
