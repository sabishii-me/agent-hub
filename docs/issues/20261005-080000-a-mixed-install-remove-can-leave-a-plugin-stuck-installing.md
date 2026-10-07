# 20261005-080000 — a mixed concurrent install/remove can leave a plugin stuck at `installing`

Recorded 2026-10-05. Found by `tests/plugins/concurrent-mixed-install-remove.py` (>=3 mixed
install/remove ops on two plugins at once). Owner: **HUB**. Status: recorded, NOT fixed
(root cause NOT located - do not guess).

## Observed (real, intermittent)

One hub, two plugins (`pi`, `deepseek`), 6 mixed ops issued concurrently (install pi, install
deepseek, remove pi, install pi, remove deepseek, install deepseek). After settling, a plugin was
left with `state: "installing"` instead of converging to `ready`/`absent`/`failed`:

- run 1: `{pi: installing, deepseek: absent}`  (seen twice)
- other runs: converged (`{pi: absent, deepseek: ready}` etc.)

Roughly 1 in 3 under load; 8/8 clean when run alone. So it is a RACE, not a deterministic path.

## Why it is a defect

ARCHITECTURE §10: an operation ends in `ready`/`absent`/`failed` + detail; a crash/replace must
never leave a silent half-state. A plugin stuck at `installing` is exactly that - the resource
lies (it is not installing; the operation is over or was superseded).

## Suspected area (NOT diagnosed - do not treat as cause)

`begin_install` does `enter(id, Installing)`; the detached `finish_install` (install route) or
`fail_remove` (remove route) is what `leave`s it. Under concurrent mixed ops on the SAME id
(install pi ... remove pi ... install pi), an older completion may run after a newer `enter`,
or a superseded op's `leave`/`enter` pairing may be unbalanced, leaving `Installing` in `ops`
with no running task to clear it. The `ops` map is `HashMap<id, Ops>` and `Ops` is a set with a
ranked verdict; a missing paired `leave` would pin the verdict. VERIFY with a targeted trace
before changing anything - this note is a HYPOTHESIS, not a located cause.

## Fix direction (once located)

Make the op's terminal transition OWNED by the operation that entered it (an op id, not just the
plugin id), so a superseded/older completion cannot clear or pin a newer op's state - and so
EVERY entered op has exactly one matching terminal. Do not add a timeout that merely hides a
stuck op.

## Status

recorded, NOT fixed. The concurrency test is the record (it goes red intermittently).
