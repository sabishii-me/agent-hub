# agent-hub

A Rust agent-hub: one non-blocking `axum` server over the shared `Transport`, with the
domains mounted. **This repository is the implementation**; `prts-web` is the orchestration
repository and holds the reviewer tasks.

## The continuous main task (read this first)

> **RESUME HERE**: `docs/tasks/CURRENT-STATE.md` is the durable handoff (goal, the exact
> open/closed links, the ONE authorization that blocks a real run, and the docs map). A
> MOCK proves only the hub's half — it is NOT acceptance. The three REAL adapters are
> present locally (pi `23ae330`, jouzu `453eff2`, deepseek `580aca8`); a capability is
> DONE only when driven through `/v1` against one of them.


**Implement the Rust agent-hub per the approved architecture, delivering real, usable `/v1`
capabilities.** Not: completing a reviewer's checklist, making tests green, or stopping after
one item until someone says "continue".

- The approved architecture is `docs/ARCHITECTURE.md`; the **order of work is §17**. The
  cross-review decisions (ADRs) live in the orchestration repo under `docs/decisions/`.
- A reviewer handoff or a review report (`PROVIDER-TURN-REVIEW-*.md`) is **correction input**,
  not the task. Do not role-switch into reviewer mode, and do not copy its list as the whole task.

### The system-alignment table (the real status)

The target is NOT "routes mounted" or "tests green": it is the UPGRADED system working
with the REAL adapters/plugins through the formal `/v1`. The authoritative per-capability
table is `docs/tasks/system-alignment.md`: for each capability it records the approved
target, the hub entry + responsibility, the adapter/plugin responsibility, the CURRENT
real implementation, the **gap owner** (HUB / PLUGIN / CONTRACT / UNAUTH), the required
upgrade, and the real acceptance. Every remaining route is also classified in
`docs/tasks/remaining-surface.md`.

Three real adapters exist and must be reconnected (their committed SHAs are recorded in
the review): pi, jouzu, dsh/deepseek. A capability is DONE only when driven through
`/v1` against a real adapter; a capable mock proves the hub's half, not the capability.

### Open links (at the recorded SHA), by owner

- **G3 (HUB+CONTRACT+PLUGIN)** provider TYPE authority: `GET /v1/model-providers/types`
  reads descriptors, but `create` accepts any type, availability = "field present", and
  grant ignores the type. Publish the descriptor artifact contract; use ONE type
  authority for enumerate/create/availability/execution; upgrade the provider plugin.
- **G4 (HUB)** delete the auth placeholder: `POST .../auth` creates a pending operation
  with NO executor and returns 501. Replace with a no-side-effect refusal until the real
  declarative flow exists.
- **G5 (HUB)** preset restart failure state: a restart whose re-attach fails must leave the
  session `needs-repair`, not `active`; a composite PATCH (preset + thinkingLevel) must run
  every field, not early-return.
- **G6 (HUB+CONTRACT+PLUGIN)** `minHubVersion` (ADR-0008) is declared nowhere and enforced
  nowhere; add it to the manifest contract and enforce at ONE plugin accept/activate edge.
- **§17 remainder** the new plugin-sourced skills/resources model (T2b) and the shared
  adapter layer; C2 forbids restoring the OLD skills model, it does NOT cancel the new one.

### Code DONE this session — but NOT acceptance-tested against a real adapter

Everything below was exercised with a MOCK adapter (or static comparison), which is NOT
acceptance. The hub's code is in place; the REAL pi/jouzu/dsh run is BLOCKED by the
unauthorized OS-keychain startup probe (see `docs/tasks/CURRENT-STATE.md`). Do not read
these as capability passes.

- **G1** the adapter bidirectional control protocol: a reverse request (`approval_need`)
  keeps its JSON-RPC id and is answered; the reply vocabulary is aligned to the REAL
  adapters' `{approved, reason:'allowed'|'denied'}` (static read of pi `23ae330`, jouzu
  `453eff2`, deepseek `580aca8`).
- **G2** the harness `connections/delete` wire mapping (`{id}` not `{connectionId}`), which
  the REAL dsh `deepseek-adapter.cjs:1308` confirms (`rows.find(r=>r.id===p.id)`).
- Preset switch without wedging (restart+resume) + persisted `applied_preset`; provider
  pending barrier on every mutator/consumer; turn/abort bound to the dispatched process
  generation; plugin install from git or a verified artifact; harness extension snapshot
  per start; provider `types` loader + auth operation store (start/status/cancel).

### Authorization boundary (blocks only the test action)

A real-auth/paid model call and touching the real OS keychain are NOT authorized in
general: the hub writes/reads/deletes a random keychain probe at startup. A real
pi/jouzu/dsh acceptance therefore needs an EXPLICIT isolated-side-effect authorization
and, for a real turn, a credential authorization. Missing authorization blocks that
TEST, not the credential-free implementation. Tests not run are NOT accepted and are
never reported green.

## Status

**The system is IN PROGRESS, not complete.** What is real now:

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
