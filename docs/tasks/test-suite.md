# Test suite — real, adversarial, layered (design of record)

Status: design of record, 2026-10-03. Language: **Python 3.13 (stdlib only)** — deliberately
NOT the hub's language (Rust), so a test cannot import hub internals or hand-build a fake.
Runner: `python tests/run.py [layer]` — a layer selector lets us run one band, not the world.

## The one rule

**No fake, anywhere, ever.** A step is either driven for REAL (real hub binary, real plugin,
real runtime, real provider, real MCP server) or the test prints **SKIP + what is missing**.
A fake provider, a stub adapter, a mocked response, or an assertion that only checks
`active` / `200` / "some assistant text" is a FAILURE of the suite, not a pass. A red is the
product's fault until proven otherwise. Every case prints its version combo (hub SHA + plugin
SHA + runtime version + provider + platform + isolated data dir) so a result is attributable.

Why Python, not Rust: the hub is Rust; a Rust test can construct hub types directly and
"prove" a behaviour the wire never shows. A different language can only speak the real
surfaces (HTTP + SSE + child processes + filesystem), so it cannot hide a fake.

## Layout

```
tests/
  run.py                 # selector: all | a layer name; non-zero exit on any failure
  lib/hub.py             # spawn the real binary, read endpoint.json, HTTP, kill (power-cut)
  lib/tally.py           # check(name, ok, detail); prints the version combo
  contract/  lifecycle/  model/  tools/  approvals/  interrupt/
  concurrency/  provider/  skills/  connections/
```

`lib/hub.py` only: spawns `target/debug/agent-hub.exe` (or release), waits for
`endpoint.json`, speaks HTTP/SSE with the bearer, and `taskkill /T /F` (no cleanup, like a
power cut). It reads NOTHING of the hub's DB and imports NOTHING of the hub.

## Layers (each a band, each real)

| layer | property under test | real dependency |
|---|---|---|
| **L1 contract** | `/v1/surface` == `contract/v1.json`; no stub route; every declared error code is reachable-shaped | none |
| **L2 lifecycle** | create / close / reopen-on-ref / fork-does-not-touch-source / compact keeps history / patch (title, plan, review, thinking, preset) | real plugin |
| **L3 model** | a real model answers AND the identity is confirmed; a second turn continues the conversation; stop reason is reported | real provider (AUTH) |
| **L4 tools** | the model REALLY runs a tool: tool_started/tool_end, and the answer reflects the tool result | real provider (AUTH) |
| **L5 approvals** | a real `approval_need`: allow permits the action, deny prevents it, adapter gets the SAME id; a FAST /v1 answer is never lost (raise-before-register race) | real plugin |
| **L6 interrupt** | a cancelled turn never reaches a new process; a KILLED hub recovers open turns; an unconfirmed cancel is bounded; close/reopen across a stop | real plugin |
| **L7 concurrency** | >=100 concurrent connections, none times out; a long op does not block a short one | none |
| **L8 provider/auth** | hub-owned device-code sign-in; real catalog; the credential reaches a real harness; cancel/delete-recreate/incarnation (A3-A5) | real provider (AUTH) |
| **L9 skills/connections** | a plugin-provided skill loads in a real harness; a real MCP connection reaches the harness | skills plan + pi MCP |

Named after the PROPERTY, not a number (`a-cancelled-turn-never-reaches-a-new-process.py`).

## Classification for layered running

- **AUTH-gated** (L3, L4, L8): need a real provider call + keychain side effect. Without an
  explicit authorization they SKIP with a reason; they are never faked.
- **local** (L1, L2, L5, L6, L7): a real plugin + real runtime, no vendor call.
- **pending** (L9): blocked on the skills decision (below) and the pi MCP translation.

## Order of execution (this round)

1. L1 (contract) + L7 (concurrency) — no auth, immediate.
2. L2 (lifecycle) + L6 (interrupt) — real plugin, no auth.
3. L5 (approvals) — real plugin.
4. L3/L4/L8 — when AUTH is authorized.
5. L9 — after the skills decision.

## DEFERRED: skills (timestamped)

**skills is deferred. Timestamp: 2026-10-03.** Resume when: (a) the skill-content decision is
made — the proposed direction is a `skill` plugin kind (`manifest.kind:"skill"`, an artifact,
managed id `skill-<id>`), so content is a versioned artifact and lifecycle is the SAME plugin
lifecycle (install/enable/disable/upgrade/minHubVersion); (b) workspace+session layered
selection field names are fixed; (c) the PUT/DELETE `/v1/skills` routes' fate is decided
(hub-authored store is excluded; content is plugin-provided). Until then L9-skills does not
run. Recorded in docs/tasks/review-bb23f7d-findings.md A1.

## What this replaces

`hub/tests/*.rs` (Rust integration tests with a FAKE HTTP provider in session_slice.rs) and
the 3 ping-pong `tests/e2e/*.mjs` runners are NOT the acceptance bar. The fake is removed; the
Rust unit tests stay (they test functions, not the product) but are not a capability claim.
The old `tests/*.mjs` suite drives the OLD Node hub (`server.mjs`, `/v1/hub/...`) and is
archived, not deleted, with a note.
