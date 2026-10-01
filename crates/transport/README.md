# agent-hub-transport

The hub's transport primitives, on axum/hyper/tokio. **Mechanisms, not product capability.**

- `Accepted` - `202 Accepted` + `Location` (RFC 9110 §15.3.3, §10.2.2). `Accepted::detached`
  runs the work on a task; a synchronous body must use `spawn_blocking` itself.
- `Admission` - a bounded semaphore; over capacity is `503` + `Retry-After`.
- `ErrorRenderer` - maps a contract error **code** to a status (the one place a status is written).
- `BearerToken` / `require_bearer` - the inbound bearer guard (o5: every route requires it).
- `routes()` / `finish()` - the transport-owned surface a domain merges onto; the SSE stream.

## Component tests

`cargo test -p agent-hub-transport` exercises these primitives against a real listener. The
`concurrency` file asserts only that N concurrent requests are answered, that two gated requests
complete while a third is refused, and that nothing deadlocks. It does **not** demonstrate heavy
operations or an idle-connection count (TASK-048 F05); the ADR-0009 numbers remain an open gate.
