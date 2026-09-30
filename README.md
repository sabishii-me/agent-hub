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

Make the already-wired chain a **coherent, recoverable slice** before adding surface: one
consistent story for provider version + secret ownership, resolver consistency, the applied
confirmation on start/reopen/switch, turn admission/delivery/cancel/terminal, fault/restart
reclamation, and one fact across GET/event/error. The whole-chain analysis is
`docs/tasks/lifecycle-chain.md`; `docs/ARCHITECTURE.md` §24 lists the remaining surface
(unbuilt routes stay `501` and are not this round's goal).

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
