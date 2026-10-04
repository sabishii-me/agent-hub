# 20261004-050000 — a selected preset is reported APPLIED but never reaches the process that runs the turn

Recorded: 2026-10-04. Found by: driving a real approval preset (`heavy-review`,
`approve:true`). Owner: **the PRESET apply path** - the ADAPTER (`prts-harness-pi`) owns
preset as its baseline; the hub's `session/start` -> `config/set` order is the inducement.
Status: recorded, NOT fixed.

## Symptom

A session created with `presetId: "heavy-review"` reports `appliedPreset: "heavy-review"`
and is `active`. But the preset is NOT in force: the model's `read` tool runs with no approval
(the whole point of that preset), and, in isolation, the same preset would ask. So the preset
is reported applied while it is not enforced.

## What is proven (real runs + an isolation experiment)

1. **The approval mechanism works when the preset actually reaches pi.** In isolation - a temp
   project with the `agent-presets` extension and `{"approve":true}`, pi spawned
   `--mode rpc --approve` - the `/review status` command answers **"review is on"**
   (`review.on === true`) and pi emits `extension_ui_request`. So extension + preset + trust is
   a working path.
2. **In the REAL session the pieces are present**: `<cwd>/.pi/extensions/agent-presets/index.ts`,
   `<cwd>/.pi/agent-presets.json` = `{"active":"heavy-review","presets":{"heavy-review":{"approve":true}}}`,
   `appliedPreset: heavy-review`. Yet during a tool call NO `extension_ui_request` reaches the
   adapter.
3. Therefore the pi process that runs the turn was spawned WITHOUT `AGENT_PRESETS_CONFIG`, so
   the extension loaded with `review.on === false`. "Applied" is recorded; "in force" is false.

## Root cause (adapter start order)

`pi-adapter.cjs` starts pi twice on create:

- `session/start` -> `startPi` spawns pi#1 with NO preset (the preset is not known yet to the
  adapter at this point).
- the hub then sends `config/set` with `presetId`. The adapter's preset branch restarts pi to
  carry the env, GUARDED by `if (!(pi && currentRef && !turnActive))` ->
  "cannot apply preset: no live harness to carry it". If pi#1 is not yet live when `config/set`
  arrives, the restart is SKIPPED - and `applied.preset` is still recorded, so the hub reports
  `appliedPreset` and marks the session `active`.

So the preset takes effect only when the timing happens to allow the restart; otherwise the
session lies about the preset.

## Why it is a preset bug (not an approval bug)

Approval is a FIELD of a preset (`approve`). The failure is that the PRESET is not applied to
the process that runs. The approval route, the hub's reverse `approval_need` handler, and the
extension's gating are all correct; they never run because the preset never reached pi.

## The two owner-side fix options (the fix must make the preset reach the RUNNING process)

- **Adapter (preferred):** the process that runs the turn must be spawned WITH the preset. Do
  not rely on a post-`config/set` restart: either start pi#1 with the session's preset, or make
  the restart unconditional and awaited, and never record `applied.preset` unless the running
  process carries it. "Applied" must mean the running process has it.
- **Hub (inducement):** the hub could convey the preset in `session/start` (or start pi only
  after the preset is decided), removing the two-phase window. This is a hub<->adapter ordering
  question; the adapter fix is sufficient on its own.

## Verify (when fixed)

- `tests/approvals/an-approval-preset-asks-before-every-tool.py` -> a tool raises a real
  approval; `GET /v1/sessions/{id}/approvals` lists it; allow resumes, deny blocks.
- `tests/approvals/denying-an-approval-blocks-the-side-effect.py` -> deny prevents the write.
- `tests/presets/a-preset-is-listed-and-applied.py` stays green (enforced, not only applied).
