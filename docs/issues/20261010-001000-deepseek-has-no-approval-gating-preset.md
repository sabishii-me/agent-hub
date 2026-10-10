# 20261010-001000 - deepseek has no preset that gates tool calls (no approval can be raised)

**State:** OPEN (recorded capability gap). Owner: the deepseek adapter/plugin (its presets).
**Severity:** on deepseek, NO tool call ever raises an approval, whatever the preset: the
approval/question surface (`GET /v1/sessions/{id}/approvals`) is never exercised, and any hub
behaviour that depends on a pending approval (e.g. `20261010-000000`) cannot be driven on deepseek.

## Measured (dev, real dsh 0.2.0-rc.2, Windows)

A session with each shipped preset (`standard`, `code`, `minimal`; `cordis` is an authoring preset),
prompted to run a write tool, raised NO approval:

    standard: active=active   approval_raised=False
    minimal:  active=active   approval_raised=False
    code:     session start failed (needs its own inputs)

dsh ships `config/agent-presets/{code,cordis,minimal,standard}` and none gates every tool the way pi's
`heavy-review` does (`{"approve": true}`).

## Consequence

`tests/interrupt/an-approval-pending-when-the-adapter-dies-does-not-hang.py` passes on pi and jouzu
(they ship an approval-gating preset) and FAILS honestly on deepseek (no gating preset -> no approval
raised). The test takes `PI_APPROVAL_PRESET` (default `heavy-review`) so a harness names its own.

## Fix direction

Ship (or document) an approval-gating preset for deepseek, so the approval surface is real on it too
(the hub already forwards dsh's `approval/requested` waterfall to `/v1/approvals`; only the harness
side lacks a preset that raises one). Until then, the deepseek approval path is unverified by a real
round-trip.
