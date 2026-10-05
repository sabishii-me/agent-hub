# 20261005-040000 — N sessions of one harness started at once race on the SHARED home's files

Recorded 2026-10-05. Found by `tests/concurrency/many-sessions-of-one-harness.py` (N sessions of
ONE harness started at the same instant). Owner: **PLUGIN** (pi/jouzu adapter) + possibly HUB.

## Observed (real)

- pi, N=32: 30/32 reach `active`; the 2 failures are
  `adapter refused: cannot apply model: deepseek-flash` and
  `adapter protocol: the adapter closed its stdout`.
- pi, N=4: passed 3x, but one earlier run failed 1/4 - so it is a RACE, load-dependent.
- jouzu, N=4 and N=8: all `active` (after the 20261005-030000 profile fix).

## Why (hypothesis - verify with a controlled trace before fixing)

Every session of a harness shares the ONE home (`<DATA_DIR>/agents/<harness>/<internal>`), which
is correct (a harness is not one-home-per-session). But the adapter **writes the injected
provider into that shared home's files** on EVERY start (`applyInjectedProvider` ->
`models.json`), and the harness itself reads/writes its state there. Many concurrent starts
therefore read a file mid-write, or collide on it. `cannot apply model` / `adapter closed its
stdout` are consistent with that.

## Correct direction (do not invent the mechanism - trace first)

The shared home is right; the CONCURRENT MUTATION of it is the problem. Likely fixes:
- serialize the home's file mutation (the hub's per-harness placement lock already serializes
  placement; the adapter's provider write happens per session AFTER that - it may need its own
  per-harness serialization, or the provider write should be idempotent/atomic);
- write `models.json` atomically (temp + rename) so a concurrent reader never sees a half file.
Trace WHICH file races (adapter write vs harness read) before choosing.

## Snapshot

`docs/review/snapshots/20261005-040000-<sha>.txt`.

## Status

recorded, NOT fixed. The test is the record; do not lower N to hide it.
