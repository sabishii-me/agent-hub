# 20261004-050000 — an approval-bearing preset raises NO approval; the tool runs ungated

Recorded: 2026-10-04. Found by: driving a real approval preset (the approval layer had no
test). Owner: **ADAPTER (prts-harness-pi) or the agent-presets extension** - to be pinpointed.
Status: recorded, NOT fixed.

## Symptom

- combo: hub `ab2f1fa`, pi plugin 0.1.9, pi runtime `1.0.0`, provider HOME-JP-prod.
- a session is created with `presetId: "heavy-review"` (its definition:
  `{"approve": true, "tools": [], "description": "every tool call asks before it runs"}`).
- `appliedPreset` is `heavy-review` (the preset applied).
- the prompt makes the model call a tool. The model calls `read`; the tool runs to completion.
- `GET /v1/sessions/{id}/approvals` is EMPTY the whole time; no approval is ever raised.
- the turn ends `completed`, the file was read - i.e. the tool ran WITHOUT asking.

## Why this matters

The whole point of the preset is "ask before every tool call". A user who selects it believes
nothing runs without their consent; in fact everything runs. This is a safety-relevant
silence, not a cosmetic bug.

The pieces are in place and were observed:

- the config IS written to the session cwd: `.pi/agent-presets.json` =
  `{"active":"heavy-review","presets":{"heavy-review":{"tools":[],"approve":true}}}`.
- the extension IS installed: `.pi/extensions/agent-presets/index.ts`.
- the extension's code DOES ask when `review.on` (which is `preset.approve === true`) via
  `ctx.ui.input("tool-review/v2", ...)`.
- the adapter DOES map `extension_ui_request` -> `approval_need` (pi-adapter.cjs ~597).
- the adapter writes the config path into `AGENT_PRESETS_CONFIG` only inside `startPi` when
  `activePresetId !== null` (pi-adapter.cjs ~477-481).

So the ask never reached the adapter: `ctx.ui.input` was never called, i.e. the extension's
`activePreset()` returned null or `review.on` was false at the time the tool ran.

## Suspected root (to CONFIRM, not assumed)

A timing/order defect: the extension reads `AGENT_PRESETS_CONFIG` at LOAD; if pi starts before
the config env/file is present, `activePreset()` is null and review is off for the whole
session - the preset looks applied (`appliedPreset` set) but is not ENFORCED.

## Fix direction

Not chosen yet - first CONFIRM the order (does the pi that runs the turn see the config?).
Then fix at the owning boundary (adapter start ordering, or the extension), never with a test
workaround. A test-only switch is forbidden.

## Verify (when fixed)

a session on `heavy-review` that calls a tool raises a real approval:
`GET /v1/sessions/{id}/approvals` lists it; `POST .../approvals/{aid}` with the SAME id allows
(resumes the tool) or denies (blocks it, no side effect). And with `review off` it does not ask.
