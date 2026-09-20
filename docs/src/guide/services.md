# Services

Services allow VMs to do stuff outside the VM itself.

## Logging

todo

## Callouts

todo topics to cover:

- How a guest initiates an HTTP or gRPC callout.
- How the embedder dispatches it externally and delivers the response through on_http_call_response or the gRPC callbacks.
- Max open callouts and what happens when the limit is reached.
