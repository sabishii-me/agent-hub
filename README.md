# agent-hub

A Rust agent-hub: one non-blocking `axum` server over the shared `Transport`, with the
domains mounted, speaking the `/v1` contract. **This repository is the implementation.**

## Build and run

```
cargo build --workspace                 # zero warnings
cargo test  --workspace                 # component + real-hub integration tests
AGENT_HUB_ADDR=127.0.0.1:8080 cargo run -p agent-hub
```

On start the hub writes `endpoint.json` (the URL and bearer token) under its data dir,
prints the bound address, and shuts down on Ctrl-C. Every `/v1` route requires the bearer
token. A route for a capability that is not built answers `501`.

Real-hub integration tests are gated on the environment (a plugin directory, a real
provider). A test that cannot run prints SKIP and returns; it never fakes a pass.

## Layout

```
crates/            the domains (one crate each)
  adapter/         the agent-bus client and the running harness processes
  contract/        the single normalization authority for `contract/*.json`
  db/              the SQLite store: sessions, turns, plugins, providers, connections
  events/          the event bus (monotonic ids, replay, resync)
  extensions/      extension placement for a harness
  harnesses/       the thin harness projection
  humans/          approvals and questions
  plugins/         install / prepare / remove / enable / disable + the registry
  providers/       model providers as data + the OS-keyring credential
  sessions/        the session + turn lifecycle over a real adapter process
  skills/          the skills surface
  transport/       the axum transport, auth, error rendering, the contract surface
hub/               the served binary + the boot self-check
contract/          the wire contracts: v1.json, adapter-v1.json, errors.json, openapi.json
plugins/           the adapter protocol prose
tests/             the Python acceptance suite (env-driven, against a real hub)
```

## The contract is the interface

`contract/v1.json` is the `/v1` surface; `contract/adapter-v1.json` is the hub↔adapter
protocol; `contract/errors.json` is the error-code table; `contract/openapi.json` is the
generated projection. The served surface must equal the contract (`/v1/surface`,
`/v1/openapi.json`). A change to a contract is expressed in `contract/` and nowhere else.

## Concurrency

The hub is a concurrent service: long work is accepted (`202 + Location`) and reports on the
event stream (`GET /v1/events`); request handling never blocks the runtime.

## Tests

There are two layers:

- **Rust component tests** (`cargo test --workspace`) - each crate's behaviour, plus the
  gated `hub/tests/*.rs` that spawn the real binary.
- **The Python acceptance suite** (`tests/`), env-driven against a real hub and REAL adapters:
  a plugin via `PI_PLUGIN_DIR`, a harness id via `PI_HARNESS_ID`, a model via `PI_MODEL`.

**Harness acceptance is a merge gate.** A harness that does not pass the required tests may
not be merged: for a harness `H`, the whole-chain test and every `tests/interrupt/*.py` must
be green against its REAL adapter (`PI_PLUGIN_DIR=<H> PI_HARNESS_ID=<H>`) or `H` does not
merge. A case `H` genuinely cannot express is a recorded limit, never a pass and never a
skip. Never make a gate green by weakening a test, skipping a case, or faking an adapter.

## Authorization boundary

Do **not**, without explicit authorization: read a person's real credentials
(`~/.pi/agent/models.json`), import an apiKey, touch the real OS keychain, or make a real
vendor/paid model call. A missing credential authorization blocks only that real call; it does
not block the implementation. Never substitute a fake provider or a fake green. Merge/push/
publish follow their own authorization.
