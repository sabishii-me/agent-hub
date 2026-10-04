# Misalignment list — what is claimed vs what the real product does (2026-10-04)

Each row: the claim (authority) vs the OBSERVED reality (a real run), and the evidence.
This is the list of things that are NOT aligned. Every item has a real test or probe.

## A. Product defects found by real tests (real red, not closed)

| # | claim (authority) | observed reality | evidence | owner |
|---|---|---|---|---|
| A1 | an `approve:true` preset "asks before every tool call" (heavy-review preset; prts-web AGENTS §8; PROTOCOL §4) | selecting it, the model's `read` tool runs with NO approval raised; the harness never sends `extension_ui_request` | tests/approvals/an-approval-preset-asks-before-every-tool.py (RED) | ADAPTER/extension |
| A2 | a denied approval must stop the side effect (PROTOCOL §4) | cannot even reach a deny - no approval is raised (blocked by A1) | tests/approvals/denying-an-approval-blocks-the-side-effect.py (RED) | ADAPTER/extension |
| A3 | turn terminal is the `turn_end` event (adapter-v1) | was read from the prompt RPC result -> every pi turn `failed`; FIXED | 20261003-120000 (fixed) | HUB |
| A4 | cancel is idempotent, 200 (v1) | idle cancel was 400; FIXED | 20261003-140000 (fixed) | HUB |
| A5 | a turn while one runs is 409 session_busy (v1) | was 400 validation_failed; FIXED | 20261003-160000 (fixed) | HUB |
| A6 | a cancel is settled by PROCESS FACT, not a timer | a timer swept a normal cancel -> healthy session `needs-repair`; FIXED (liveness) | 20261004-030000 (fixed) | HUB |
| A7 | a terminal is bound to ITS turn (adapter-v1 ID rule) | was keyed by session -> the next turn took it; FIXED (clientMessageId + per-turn waiter) | 20261004-040000 (fixed) | HUB |

## B. The approval/plan/preset family - the UNALIGNED core the owner named

- **preset APPLIED vs preset ENFORCED are different facts, and only the first is real today.**
  `appliedPreset: heavy-review` is reported, the config is written, the extension is installed,
  the adapter can map `approval_need` - yet no approval is raised. "Applied" is reported while
  "enforced" is silent-false. This is the same defect family as F1/F5 ("looks applied, is not").
- `plan` toggle: OK (tests/presets/plan-mode-is-applied-and-reports.py, 4/4).
- unknown preset: correctly ends `starting_failed` (tests/presets/a-preset-is-listed-and-applied.py, 8/8).

## C. Coverage gaps still open (no test; some need real interaction)

- `GET/POST /v1/sessions/{id}/approvals|questions` - now tested (approval) and RED; questions not.
- `POST /v1/model-providers/{id}/models/refresh` - untested.
- `POST /v1/harnesses/{id}/connections/validate` - untested.
- `POST /v1/plugins/registry/refresh` - untested.
- `GET /v1/harnesses/{id}/presets` - now tested; `auth*`, `extensions PATCH` - untested.
- skills: tested on the EXCLUDED hub-authored model (review A1) - not the decided model.

## D. Doc/authority misalignment

- `contract/adapter-v1.json` `proseAuthority` pointed at `plugins/PROTOCOL.md`, which did not
  exist in this repo (deleted in d93a919). RESTORED, and `turn_end` gained `clientMessageId`.
- The adapter contract's `turn_end` had no turn identity; fixed (now `requires clientMessageId`).
