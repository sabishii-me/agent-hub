# agent-hub

A Rust agent-hub: one non-blocking `axum` server over the shared `Transport`, with the
domains mounted. **This repository is the implementation**; `prts-web` is the orchestration
repository and holds the reviewer tasks.

## The continuous main task (read this first)

**Implement the Rust agent-hub per the approved architecture, delivering real, usable `/v1`
capabilities.** Not: completing a reviewer's checklist, making tests green, or stopping after
one item until someone says "continue".

- The approved architecture is `docs/ARCHITECTURE.md`; the **order of work is §17**. The
  cross-review decisions (ADRs) live in the orchestration repo under `docs/decisions/`.
- A reviewer handoff or a review report (`PROVIDER-TURN-REVIEW-*.md`) is **correction input**,
  not the task. Do not role-switch into reviewer mode, and do not copy its list as the whole task.

### The current vertical chain (where the work is)

```
hub-managed provider relationship state
-> persistent instance identity + secret reference
-> recoverable provider / credential lifecycle
-> resolver + credentials/grant
-> config/set target + actual applied identity confirmation
-> prompt / abort / terminal
-> restart & session recovery
```

Every link has real code and live verification; the work records below hold the detail.
What is NOT fully closed, honestly: a real model turn needs a provider credential this
environment does not have (a credential authorization - it blocks only that call); and
several fault/recovery branches are reasoned and unit-tested but only partly exercised
on a real process (an unconfirmed cancel, a stop that fails, a config response that
fails after the adapter applied it). These are tracked in `docs/tasks/lifecycle-chain.md`
and `docs/tasks/review-*.md`, not claimed as done.

### Current focus

**The slice is IN PROGRESS, not complete.** The provider/secret/turn chain and the
session preset capability are the current work; several genuinely HUB-OWNED domains
remain unimplemented (§17 order), and each remaining route is classified in
`docs/tasks/remaining-surface.md` as ONE of: (a) hub work still to do, (b) a concrete
external dependency, or (c) an unavailable real call (credentials/paid). A missing
plugin or credential does NOT make a hub-side implementation complete.

Delivered so far on this chain: preset switching WITHOUT wedging a session (restart +
`session/start(resume)`, HTTP-verified across repeated switches), the confirmed
`applied_preset` persisted, the provider pending barrier on every mutator/consumer, and
the turn/abort delivery bound to the dispatched process generation. Still to do (hub
side): the provider-type descriptor loading + auth execution (`/v1/model-providers/types`
and `auth/*` are unconditional `501` today), the new plugin-sourced skills model
(T2b), and the remaining §17 domains/surface.

## Status

**Not a product yet** (honest). What is real now:

- Sessions own a real adapter process (`session/start` + `config/set`; `active` only when both
  succeed); a session may name a hub-managed `modelProviderId`/`modelId` (resolved through an
  injected resolver -> `credentials/grant` -> `config/set`, with the adapter's `applied` identity
  **confirmed and persisted**), a `presetId` (confirmed against `applied.preset`), and
  `plan`/`review` (confirmed against `applied.plan`/`applied.review`).
- Turns: `202 + Location`, a durable turn identity, the adapter's own result is the terminal,
  cancel is coordinated with dispatch, and a restart reconciles orphans (`needs-repair`).
- Providers: relationship state in the database (rows with an `incarnation`/`revision` guard),
  credential in the **OS keychain** as a per-instance reference, never plaintext.
- Plugins: `POST /v1/plugins` accepts BOTH declared sources — a git clone (`{url, ref?}`)
  and a release artifact (`{artifact:{url,sha256,id,pluginType,version,size?}}`), verified
  size-first then sha256 before unpacking; a recorded artifact is reported by
  `GET /v1/sessions/{id}/artifacts`.
- Sessions: `POST /v1/sessions/{id}/repair` recovers a session whose cancelled turn never
  confirmed its end (hub-side re-abort, process replace, re-attach; `proven` only when the
  re-attach succeeded); `preview:true` changes nothing.
- Everything unbuilt answers `501`; the mounted-vs-contract surface is in `§24`.

## Build and run

```
cargo build --workspace                 # zero warnings
cargo test  --workspace                 # component tests
AGENT_HUB_ADDR=127.0.0.1:8080 cargo run -p agent-hub
```

On start it writes `endpoint.json` (URL + bearer token) under the data dir and prints the bound
address. Real-hub integration tests are gated on the environment (e.g.
`AGENT_HUB_TEST_PLUGIN_DIR`); a test that cannot run prints SKIP and returns - it never fakes a
pass.

## Work records (the implementation log)

- `docs/ARCHITECTURE.md` - the architecture and, in §17-§24, the order of work, the current
  status and the remaining surface. **§17 is the plan of record.**
- `docs/tasks/minimal-secret-store.md` - the OS secret store (instance identity, recovery).
- `docs/tasks/turn-lifecycle.md` - the turn lifecycle, terminal semantics, restart reconciliation.
- `docs/tasks/provider-grant-chain.md` - the provider->session grant chain, presets and model
  selection; `docs/tasks/connections.md`, `docs/tasks/plugin-lifecycle.md`,
  `docs/tasks/metadata-routes.md`, `docs/tasks/remaining-surface.md`.

## Authorization boundary

Do **not**, without explicit authorization: read a person's real credentials
(`~/.pi/agent/models.json`), import an apiKey, touch the real OS keychain, or make a real
vendor/paid model call. A missing credential authorization **blocks only that real call**; it
does **not** block the system implementation. Never substitute a fake provider, a native private
auth path, or a fake green for managed-provider acceptance. Merge/push/publish follow their own
authorization. Verify against the real `/v1` path, never by echoing or by asserting `active` alone.
