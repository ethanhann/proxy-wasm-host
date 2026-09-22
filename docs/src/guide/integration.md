# Integration

todo topics to cover:

- Create an Engine, compile a Module, link a Host, instantiate a Guest.
- Implement StreamState to connect the guest to your request and response data.
- StreamState method reference: what each method serves (header maps, buffers, properties, stream control, local responses, foreign functions).
- Access vs NoStream and how they gate what the guest can reach.
- HeaderMap and Buffer traits: use VecHeaderMap and Vec<u8>, or bring your own.
- Implement LogSink to route guest log output.
- Walk through a complete request from on_context_create to on_log and on_delete.
- Multi-tenancy and thread safety: Guest is Send but not Sync, sharing state via Arc.
- Moving a Guest between threads.
