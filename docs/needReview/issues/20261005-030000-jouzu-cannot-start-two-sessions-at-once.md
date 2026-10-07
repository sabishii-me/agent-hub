# 20261005-030000 — jouzu: two sessions of one harness cannot start at once (the profile apply races)

Recorded 2026-10-05. Found by the REAL concurrency test `tests/concurrency/many-sessions-of-one-harness.py`
(4 sessions of ONE harness, started at once). Owner: **PLUGIN** (`prts-harness-jouzu`) + jouzu.

## Observed (real, reproducible)

4 sessions of the `jouzu` harness created at the same instant, ONE harness home
(`<DATA_DIR>/agents/jouzu/jouzu-home`, correct shape). Result: 1 reaches `active`, 3 fail with
`starting_failed`; the adapter log carries:

```
[jouzu-harness] Jouzu profile state is unreadable: another profile operation is in progress
[jouzu-adapter] harness exited code=1
```

pi with the same test: **all 4 active** (4/4). So it is jouzu-specific.

## Why

Every jouzu launch runs the profile apply at startup (`runtime/dist/main-cli.js`, the
`applyProfile` call on the non-doctor path). `applyProfile` takes `profile.lock` under the
harness home; a lock held by a LIVE process is REFUSED (not waited on), so the losing launches
throw `ProfileStateError` and exit 1. Two+ concurrent launches of one home therefore collide.

## Why it matters

A harness MUST serve many sessions at once (ADR-0009 concurrency; a real user opens several).
jouzu itself supports unlimited sessions on one home - but the profile apply on EVERY launch,
racing on the shared lock, breaks concurrent starts. This is not "how jouzu is used"; it is a
defect in how the harness is started.

## Fix (to decide - do not invent)

The profile is hub-owned and converges after the first apply. Options, in order of preference:
1. **PLUGIN/adapter**: apply the jouzu profile ONCE before any session of this harness starts
   (serialized by the hub's per-harness placement lock), so every later launch finds it
   converged and does not take the lock. This keeps one home and does not touch jouzu.
2. jouzu runtime: make the startup apply converge without a hard failure on a live lock (wait,
   or treat "already applied" as success).
Until fixed, `many-sessions-of-one-harness` is RED on jouzu - the test is the record, do not
relax it.

## Snapshot

`docs/review/snapshots/20261005-030000-<sha>.txt` (the adapter's start path + the jouzu profile apply).
