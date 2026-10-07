# The concurrency acceptance (ADR-0009), measured

ADR-0009 requires the hub to be a concurrent, non-blocking service: **>= 100 concurrent
connections**, no in-flight work timing a connection out, long work returns a handle
and reports via the event stream. `TASK-036` reproduced the original defect on the
Node hub (a plugin delete froze the event loop; 8 concurrent `/status` polls timed out
with `os error 10054`).

## Measured on the Rust hub

`tests/concurrency/measure.mjs <base> <token> [N]` fires `2*N` plain concurrent
requests (`/v1/status` + `/v1/surface`) and fails if any errors.

Result at `c03e4b1`: **400 requests (200 status + 200 surface), 400 ok, 0 errors,
344 ms.** The acceptance holds with a wide margin; long operations are already
`202 + Location` (session create/turn, plugin install/remove) and report through the
event stream.

This is a MEASUREMENT (a real run), not an assertion - per ADR-0009's own wording.

## The original defect: a long operation must not freeze the loop

TASK-036's reproduction was a plugin delete (363MB / 28,582 files) blocking the event
loop so 8 concurrent `/status` polls timed out (`os error 10054`).

Measured on the Rust hub: `POST /v1/plugins` for the 411MB pi plugin answers
`202 Accepted` immediately (the install runs detached), and **15 of 15 `/status` polls
during the install answered 200** (a 2s cap each; zero timeouts). The loop never
freezes: long work is accepted as a handle and reported through the event stream.
