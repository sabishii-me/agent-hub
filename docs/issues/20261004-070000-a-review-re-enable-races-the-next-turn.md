# 20261004-070000 — re-enabling review (`PATCH review:true`) is reported applied, but the next turn can run ungated

Recorded: 2026-10-04. Found by `tests/approvals/review-remains-switchable-after-a-preset.py`
(~4/20 standalone). Owner: **PLUGIN** (`prts-harness-pi`; the same code is in
`prts-harness-jouzu`). Status: recorded, NOT fixed.

## Symptom

A preset session with review ON, then `PATCH review:false` (tool runs ungated - correct), then
`PATCH review:true`: the PATCH answers 200 and the adapter reports `applied.review=true`, but
the NEXT turn's tool call runs WITHOUT an approval. Intermittent (~20%). The model DOES call
the tool and the tool DOES run (the turn transcript shows `tools:[{name:"read", state:"done"}]`)
- it is not a model that skipped the tool.

## Evidence (real runs, hub 83779eb + pi plugin 7a23419 + pi 1.0.0)

- Extension-side trace of a FAILING attempt: the pre-preset spawn loads `review.on=false`; the
  preset spawn loads `review.on=true`; `/review off` lands (`CMD arg=off`); **`/review on` never
  reaches the extension (`no CMD arg=on`)** - yet the hub received `applied.review=true` and the
  tool ran ungated.
- Adapter-side trace of a PASSING attempt: `SEND /review on` -> `READBACK state=true` ->
  `SETTLE-SNAP reviewEntry={"asking":true}`. So the normal path is correct.
- Timing: inserting a 1.5s pause between `PATCH review:true` and the next turn drops the
  failure rate from ~4/20 to ~1/20. **It is a race between the review switch becoming effective
  and the next turn.**

## Root cause (mechanism)

`PATCH review:true` -> hub `apply_policy_knob` -> adapter `config/set {review:true}` ->
`reviewCommand(true)`:

1. `piRequest({type:'prompt', message:'/review on'})` - pi accepts the prompt (`success:true`)
   but the `/review on` COMMAND is executed by the extension on pi's own schedule; acceptance is
   not the same as the switch having taken effect.
2. `readReviewState` then reads the newest `hub-review/state` entry and reports it. When the
   command's own `appendEntry` has NOT landed yet, the readback reflects an EARLIER entry, so
   `applied.review` can be reported `true` from a stale entry while the extension's live
   `review.on` is still `false`.
3. The hub treats the confirmed `config/set` as authoritative and admits the next turn; the turn
   reaches the tool call while the extension still has `review.on=false`, so no approval is
   raised and the tool runs.

So "applied" is read from a LOG entry, not from the state the running extension actually has -
the exact class of bug as 20261004-050000 (a log record mistaken for the live state).

## Fix direction (adapter-owned)

- `reviewCommand` must not report a state until the change is EFFECTIVE, not merely accepted:
  confirm the extension's `review.on` after pi has processed the command (e.g. wait for the
  command's `entry_appended`, or have the command's effect observable before readback), so
  `applied.review` reflects the live gate.
- The `/review on|off` command should be synchronous with respect to its log append, so a
  readback taken right after acceptance cannot see a pre-command entry.
- Do NOT patch the hub to delay turns, and do NOT add a sleep. The switch must be honest.

## Verify

`python tests/approvals/review-remains-switchable-after-a-preset.py` run 20x -> 0 failures
(step 3: after `PATCH review:true`, the tool MUST raise an approval, every time). And
`tests/approvals/*` green on pi and jouzu.

## WIP fix (NOT committed — unverified)

`prts-harness-pi` `stash@{0}` ("WIP 20261004-070000 reviewCommand waits for the observed
review state (unverified: keychain down)"): `reviewCommand` now waits (bounded, 5s) until the
newest `hub-review/state` entry actually records the requested value before reporting it, so
`applied.review` is read from the observed state rather than the acceptance of pi's prompt.

Measured effect (before the keychain became unreachable, so partial): the failure rate in a
20x loop dropped from ~4/20 to ~2/20. It is an improvement but NOT a complete fix — the residual
is not yet explained, which is why it is stashed, not committed. Resume: apply
`stash@{0}`, reproduce the residual with the extension's own trace (the extension has an
`fs` import; a probe must not break its load), then decide the real fix. Do NOT commit a fix
whose residual is unexplained.

Blocked on `docs/issues/20261004-080000` (the OS secret store is unreachable now), so the
20x verification cannot be re-run until the store is back.

## UPDATE (2026-10-05): the stale-log half is FIXED; a SECOND, jouzu(pi 0.87.1)-specific defect remains

Fix committed (pi `096759f`, mirrored jouzu `63d57d7`):

1. `reviewCommand` counts the `hub-review/state` entries BEFORE sending `/review on` and
   resolves only when a **NEWER** entry records the requested value (was: read the newest
   entry right after pi accepted the prompt, which could be an earlier stale `true`).
2. `config/set`'s `settle()` no longer overwrites `applied.review` from the log's newest entry
   when **this** request carried `review` (that clobber re-read the log as if it were the live
   gate). `settle()` still snapshots review when the request did not change it.

Result:
- **pi 1.0.0: fixed.** `tests/approvals/review-remains-switchable-after-a-preset.py` 9/9;
  a 20x loop -> **0/20**; idempotent `PATCH review:true` stays `appliedReview:true`; full suite
  32/32.
- **jouzu (pi 0.87.1): improved, NOT fixed.** The toggle test still fails ~3/12.

### The remaining jouzu defect (evidence)

On a failing run the extension's own `hub-review/decision` entry records
`{source:"user", approved:true}` - i.e. the extension DID gate, called `ctx.ui.input`, and got
an **approval** - yet the test's `GET /v1/sessions/{id}/approvals` poll (every 50ms) saw NO
approval to answer. So the approval was **raised and allowed without a visible /v1 decision**.
Adapter-side trace of the failing turn: `tool_execution_start` -> `extension_ui_request:input`
(title `tool-review/v2`) -> `entry_appended` -> `tool_execution_end`; the adapter reached its
`APPROVAL-BRANCH structured=true` and sent `approval_need`. pi 0.87.1's `emitToolCall` DOES
honour `{block:true}` (checked in the bundle), so the block was lost because the gate was
handed an approved decision, not because the block was ignored.

Hypothesis to test next (NOT yet proven): on pi 0.87.1 the `extension_ui_response` the
extension receives is not the hub's `/v1` decision but something that resolves the dialog as
approved (a turn-settle abort that yields `undefined` would DENY, so it is not that). Trace the
exact `extension_ui_response` value the adapter writes for the failing turn, and whether the
hub raised an approval at all for it. Do NOT claim this fixed until the toggle test is 0/20 on
jouzu too.
