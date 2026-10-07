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

## UPDATE (2026-10-05): measured further, NOT root-caused

- The dominant pi error at N=32 is `cannot inject provider <url>: EPE...` (an `EPERM`/`EEXIST`
  class OS error on the injected-provider write), plus `the adapter closed its stdout`.
- It is INTERMITTENT: a later N=32 run passed entirely; N=4 passes 3x.
- An ATOMIC write attempt (temp + `renameSync` over `models.json`) made it WORSE on Windows
  (rename over a file another process holds -> `EPERM`), so it was REVERTED. Recorded so the
  next attempt does not repeat it.
- NOT root-caused. Do NOT guess a fix; trace the exact syscall + which process holds the file
  (the adapter's provider write vs the pi process's own config read/write) with a controlled
  run before changing anything.

## ROOT CAUSE (2026-10-05): the hub's EXTENSION SNAPSHOT SWEEP deletes a snapshot a live session is using

The failure is NOT in the adapter. The adapter log shows:

```
[pi-harness] Error: Failed to load extension "...extensions.snapshots/.snap-<id>/agent-presets": Extension path does not exist
[pi-adapter] pi exited code=1
```

The hub hands each session an IMMUTABLE extension snapshot path (`AGENT_HUB_INSTALLED_EXTENSIONS_DIR`,
`crates/extensions/src/service.rs::install_for_harness`) and the adapter loads it with `-e`. But
`sweep_old_snapshots` keeps only the newest `KEEP_SNAPSHOTS = 8` and **deletes every older one -
with NO check that a live session is still using it**. At N=32 concurrent starts, session #1's
snapshot is deleted by session #9's sweep before #1's pi runtime loads it -> pi exits 1 ->
`the adapter closed its stdout`. pi N=32: 28-30/32; pi N=4 never hits it (only 4 < 8).

The design (TASK-048 S5) states the sweep is "by age, NEVER while current" - the implementation
keeps only 8 and deletes current ones. Owner: **HUB**.

Fix direction: a snapshot in use by a live session must not be swept. Sweep by AGE with a bound
longer than any session start, or track which snapshot each live process holds and never delete
one in use. Do NOT simply raise KEEP_SNAPSHOTS (a count can never cover an unbounded N).
