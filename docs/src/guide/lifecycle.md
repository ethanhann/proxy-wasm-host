# Lifecycle

todo topics to cover:

- Root context: create, vm_start, configure.
- Stream context: create, request headers, request body, response headers, response body, done, log, delete.
- Tick timer and how set_tick_period drives on_tick.
- Poisoning: what causes it (trap, limit, panic), what it means (instance is dead), how to recover (rebuild from the same Module).
