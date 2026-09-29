# agent-hub-transport

The hub's transport (`ARCHITECTURE` §9, task T6): an axum/hyper server whose request
paths never block the runtime. It provides:

- `Accepted` - `202 Accepted` + `Location` (RFC 9110 §15.3.3, §10.2.2): a long
  operation returns immediately, its result is a resource;
- `Admission` - a bounded semaphore; over capacity is `503` + `Retry-After`, never an
  unbounded queue;
- `routes()` / `finish()` - the transport-owned surface (`/v1/hub/status`,
  `/v1/hub/events`) that domains merge onto;
- the SSE stream with `id` / `Last-Event-ID` catch-up (replay or resync).

## Measured concurrency (task T6)

`tests/concurrency.rs` runs against a real listener:

| scenario | result |
|---|---|
| 200 concurrent `/v1/hub/status` | ~40 ms, all 200 |
| 500 concurrent `/v1/hub/status` | ~80 ms, all 500 |
| two heavy operations + a third | both progress; third `503` |
| status polls while heavy work runs | all `200` |

The old Node hub (one event loop, `fs.rmSync`) froze at **8** concurrent polls during a
plugin delete.
