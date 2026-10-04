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
