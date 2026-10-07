# Implementation gaps found by real tests (2026-10-04)

This file lists what the real product DOES, observed by running it through `/v1` against the
real adapters - nothing more. It does not judge documents or reviews, and it does not treat
any review result as a target or precondition.

## Open gaps (real tests, RED where stated)

| # | observed reality | test | owner |
|---|---|---|---|
| G-1 | **a selected preset is reported APPLIED but never reaches the process that runs the turn** (`heavy-review`, `approve:true`): the tool runs with no approval; the harness never sends `extension_ui_request`. Root cause: the adapter restarts pi to carry the preset only if a live child exists at `config/set`; if the first pi is not up, the restart is skipped and `applied.preset` is still recorded. | tests/approvals/an-approval-preset-asks-before-every-tool.py (RED); docs/issues/20261004-050000 | the preset apply path (adapter owns preset; the hub start order is the inducement) |
| G-2 | a denied approval cannot be exercised: no approval is raised to deny (blocked by G-1) | tests/approvals/denying-an-approval-blocks-the-side-effect.py (RED) | same |

## Not covered yet (no test)

- `GET/POST /v1/sessions/{id}/questions` - no test.
- `POST /v1/model-providers/{id}/models/refresh` - no test.
- `POST /v1/harnesses/{id}/connections/validate` - no test.
- `POST /v1/plugins/registry/refresh` - no test.
- `GET /v1/harnesses/{id}/auth*`, `extensions PATCH` - no test.
- skills: only the hub-authored directory model is exercised; the plugin-sourced model has no
  test.

## Verified real (for contrast, so the gaps are not mistaken for everything)

session lifecycle; turn + cancel terminal; provider CRUD/probe; plugin lifecycle; connections
CRUD; concurrency (a second turn is refused, cancelling one session does not disturb another,
N sessions cancel together); the process-death matrix (runtime killed, whole tree killed +
restart, adapter killed during a turn, adapter killed during a cancel); a real network drop
mid-turn; the plan toggle; an unknown preset ending `starting_failed`.
