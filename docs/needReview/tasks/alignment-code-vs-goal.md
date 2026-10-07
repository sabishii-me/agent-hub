# Alignment: the current code vs the goal (2026-10-04)

The goal (`README.md`): implement the Rust agent-hub per the approved architecture, delivering
REAL, usable `/v1` capabilities against the REAL adapters. §17 is the order of work. This file
states, capability by capability, what the CURRENT CODE does, with the REAL test result as the
context (no document or review is treated as a target here).

Version combo of the evidence: hub `f729a56`, pi plugin (branch fix/runtime-placement) `7b84cb0` (0.1.10),
pi runtime `@earendil-works/pi-coding-agent@1.0.0`, provider HOME-JP-prod, Windows.
Also verified against jouzu plugin `89e17d4` (runtime jouzu 0.1.18 / pi 0.87.1).

## How to read this

- A row is REAL only if a test drives it through `/v1` against the real adapter and passes.
- A row is RED if a test drives it and fails - that is an open defect.
- A row is OPEN if there is no test yet (not "done", not "broken" - not proven either way).

## Capability status (current code)

| capability | status | evidence |
|---|---|---|
| contract surface == the contract (no stub/extra route) | REAL | contract/surface 4/4 |
| status / surface / openapi / models metadata | REAL | contract/metadata 8/8 |
| session create / close / reopen / fork (source untouched) | REAL | lifecycle/session 10/10 |
| session CRUD + read-through (list/get/patch/turns/messages/stats/skills/artifacts/compact/repair-preview/delete) | REAL | lifecycle/crud 19/19 |
| turn: real model answer + confirmed identity + multi-turn recall | REAL | model/identity 8/8 |
| turn: the model REALLY runs a tool | REAL | tools/real-call 6/6 |
| cancel: confirmed -> terminal, session reusable | REAL | interrupt/cancel 7/7 |
| cancel: idle / repeated -> idempotent 200 | REAL | interrupt/cancel-idempotent 5/5 |
| cancel then immediate re-send (no wedge, terminal bound to its turn) | REAL | interrupt/send-after-cancel 8/8 (x8 runs) |
| interrupt: hub killed (power cut) -> recover | REAL | interrupt/kill-recover 6/6 |
| interrupt: adapter killed mid-turn -> honest, no hang | REAL | interrupt/kill-adapter-mid-turn 8/8 |
| interrupt: adapter killed during a cancel -> honest | REAL | interrupt/kill-adapter-during-cancel 8/8 |
| interrupt: the pi RUNTIME killed (adapter alive) -> honest | REAL | interrupt/kill-runtime 8/8 |
| interrupt: the WHOLE tree killed + restart on same data dir -> needs-repair -> reopen -> usable | REAL | interrupt/kill-tree-restart 8/8 |
| interrupt: provider unreachable -> start fails honestly, names the error | REAL | interrupt/provider-unreachable 5/5 |
| interrupt: provider hangs (black-hole route) -> hub does not freeze | REAL | interrupt/provider-hangs 5/5 |
| interrupt: network DROPS mid-turn (real TCP forwarder killed) -> honest terminal | REAL | interrupt/network-drop 8/8 |
| concurrency: >=100 connections, none times out | REAL | concurrency 3/3 |
| concurrency: a second turn while one runs -> 409 session_busy, not queued | REAL | concurrency/busy 5/5 |
| concurrency: cancelling one session does not disturb another | REAL | concurrency/cross-talk 9/9 |
| concurrency: N sessions cancelled together all settle | REAL | concurrency/many-cancel 15/15 |
| provider CRUD / types / unknown refused before write / token never echoed | REAL | provider/crud 13/13 |
| plugin install/prepare/enable/disable/icon/remove (deployment-dir refusal is 409, not 500) | REAL | plugins/lifecycle 9/9 |
| connections CRUD; row gone after delete; token never echoed | REAL | connections/crud 8/8 |
| harness list; capability-gated routes answer honestly | REAL | harnesses 7/7 |
| **preset: enumerated, applied, unknown preset -> starting_failed** | REAL | presets/apply 8/8 |
| **plan toggle applied (or honestly warned)** | REAL | presets/plan 4/4 |
| **approval: an `approve:true` preset raises a REAL approval before a tool** | REAL | approvals/preset-gates 8/8 |
| **approval: denying blocks the side effect** | REAL | approvals/deny-blocks 2/2 |
| **review: switchable after a preset (on -> off -> on)** | REAL | approvals/review-toggle 9/9 |
| questions (harness asks the user) | OPEN | no test |
| model-providers/{id}/models/refresh | OPEN | no test |
| harnesses/{id}/connections/validate | OPEN | no test |
| plugins/registry/refresh | OPEN | no test |
| harnesses/{id}/auth*, extensions PATCH | OPEN | no test |
| skills (the plugin-sourced model) | OPEN | only the directory model is tested |

Suite total: **32/32 files pass** (`python tests/run.py`). The layers list now covers every
real layer (plugins/harnesses/presets were previously missing, so a real plugins failure hid there).

## Capabilities implemented but NOT real

None in the approval/preset area anymore: the approval path is REAL (a preset's `approve:true`
raises a real approval; deny blocks the side effect; review stays switchable). Two defects were
closed to get here: `20261004-050000` (the pre-preset spawn's stale review record overrode the
preset) and the hub-side `list_approvals` returning resolved rows + a bare `deny` read as allow.

Still NOT real / NOT proven: the OPEN rows above (questions; model list refresh; connection
validate; registry refresh; harness auth/extensions PATCH; the plugin-sourced skills model).
And one KNOWN open defect: `20261004-070000` (PATCH review:true can race the next turn, ~4/20).

## Verdict

- The core the product is used for - sessions, turns, cancel, interrupt/restart honesty,
  concurrency, providers, plugins, connections, plan, presets - is REAL and proven by running it.
- The approval/preset/review capability is now REAL, proven end-to-end through `/v1` against the
  real pi adapter (and re-checked on jouzu 0.1.18).
- The OPEN rows above are unproven, not broken. `20261004-070000` is a known open defect (review
  re-enable race).
