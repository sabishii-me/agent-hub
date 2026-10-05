# 20261004-060000 — jouzu adapter: three real gaps (pre-existing, NOT from the 0.1.18 upgrade)

Recorded: 2026-10-04. Found by the real suite run against the jouzu plugin
(`PI_PLUGIN_DIR=prts-harness-jouzu PI_HARNESS_ID=jouzu`) while upgrading jouzu 0.1.13 -> 0.1.18.
Owner: **PLUGIN** (`prts-harness-jouzu`) - but see UPDATE: NOT yet located to jouzu code.
Status: recorded; the label 'jouzu gaps' is UNPROVEN.

## Why this is recorded

During the jouzu 0.1.18 upgrade the suite ran 24/28 against jouzu. Four files failed. Each was
re-checked against the OLD 0.1.13 runtime and **fails there too** — so these are pre-existing
jouzu adapter gaps, NOT regressions from the upgrade. (The upgrade itself is clean: see the
upgrade record; approvals 8/8 + 2/2 + 9/9 pass on 0.1.18/pi 0.87.1.)

## The three gaps (real, reproducible; each also fails at 0.1.13)

1. **`lifecycle/a-session-closes-reopens-and-forks-without-touching-its-source.py`** — after
   close -> reopen -> `POST /v1/sessions/{id}/fork {}`, the adapter closes its stdout and the
   fork answers `502 adapter_crash` ("the session could not be started: adapter protocol: the
   adapter closed its stdout"). It is timing-sensitive: a fork from a plain `active` session
   succeeds, and a close->reopen->fork sometimes succeeds; the test's ordering hits it. The
   same test passes 10/10 against pi. Owner: the jouzu adapter's fork/re-attach path.

2. **`interrupt/send-immediately-after-cancel.py`** — the post-cancel turn settles
   `ended=failed` instead of `completed` (7/8; the turn is accepted and the session is not
   wedged, but a later clean turn still ends `failed`). Owner: the jouzu adapter's terminal
   for a turn after a cancel.

3. **`concurrency/cancelling-one-session-does-not-disturb-another.py`** — session A's turn is
   not admitted/`session B is active` fails while cancelling A (the test then trips on a
   missing `turn`). Owner: the jouzu adapter under concurrent cancel + start.

(A fourth, `model/a-real-model-answers-with-a-confirmed-identity.py`, failed once mid-suite but
passes standalone on 0.1.18 — treat as flake until it reproduces with the adapter stderr; not
recorded as a gap.)

## What is NOT the cause

- **Not the 0.1.18 upgrade**: all three fail identically at 0.1.13.
- **Not the hub**: the same tests pass 10/10 (fork) and 8/8 (model) against pi; the hub code is
  unchanged between the two runs.

## Fix direction

Adapter-owned: correct fork/re-attach after a reopen, the post-cancel turn terminal, and the
concurrent cancel+start path. Then re-run the three files against jouzu (must be green), and
the full suite against jouzu (target 28/28). Do NOT special-case them in the hub.

## UPDATE (2026-10-05): the failing set is now measured, and it is pre-existing

Ran the HARNESS-PARAMETERIZED tests (the ones that read `PI_HARNESS_ID`) against jouzu
(`PI_PLUGIN_DIR=.../prts-harness-jouzu PI_HARNESS_ID=jouzu`). The pi-hardcoded tests
(plugins/lifecycle, harnesses/routes) are NOT parameterized - running them with `PI_HARNESS_ID`
gives FALSE failures (they still expect the harness `pi`). Do not count those as jouzu gaps.

The parameterized set has **4 real failures** on jouzu:

| test | result | first failing assertion |
|---|---|---|
| lifecycle/...forks... | 6/7 | `fork is accepted` -> 502 adapter_crash (adapter closed stdout) |
| model/...identity | 6/7 | (standalone now) |
| interrupt/send-immediately-after-cancel | 7/8 | the post-cancel turn ends `failed` |
| concurrency/cross-talk | 1/3 | `session A is active` -> start does not reach active |

CONTROL (required before any cause claim): stashing the 20261005-010000 extension-delivery
change (jouzu back to `63d57d7`) and re-running gives the SAME results; cross-talk is 1/3 on
three runs both with and without the change. So these are NOT caused by that change - but
they are ALSO **not yet located to a jouzu code path**. They are four symptoms. Do NOT leave
this worded as 'three jouzu adapter gaps' until each is traced to a line of jouzu code (or
shown hub-side). The cross-talk and fork symptoms in particular need a trace of the jouzu
spawn/fork path, not an assumption.

## LOCATED (2026-10-05): TWO distinct root causes, both in jouzu (not the hub)

Traced each symptom to code (controlled: same hub, same test; pi passes the identical sequence).

### Cause A - jouzu's `turn_end` event omits `clientMessageId` (and `state`)

- The hub binds a turn's terminal by the `clientMessageId` on the adapter's `turn_end` event
  (`crates/sessions/src/runtime.rs`, the pump: `pump_terminals.remove(cmid)`; waiter registered by
  `watch_terminal(clientMessageId)` in `service.rs`). The `clientMessageId` is the TURN id.
- **pi** sends `{type:'turn_end', clientMessageId, state, status}` (`pi-adapter.cjs:797`).
- **jouzu** sends `{type:'turn_end', status}` ONLY, at `jouzu-adapter.cjs:913` and `:1112` - no
  `clientMessageId`, no `state`. So the hub's `if let (Some(cmid), Some(state))` never matches;
  the waiter is never fired; after the 5s wait the hub settles the turn with the honest fallback
  (`service.rs`: cancelled -> `interrupted`, else -> `failed` with 'the adapter did not report a
  turn state').
- Evidence: a temporary turn trace (reverted) showed the adapter itself settling
  `status=completed` for the third turn, while the hub recorded `ended: failed`. Direct probe:
  after an 8s wait, turn 1 reads `state=ended ended=failed` though the model answered.
- Explains: **interrupt/send-after-cancel** (post-cancel turn ends `failed`), **model/identity**
  (turn 1 ends `failed`; turn 2 then has no clean occupancy).
- Fix (jouzu-adapter, jouzu's own code): include `clientMessageId` (the turn's id) and a contract
  `state` on every `turn_end` exactly as pi does. This is the same shape pi already emits.

### Cause B - jouzu's profile lock is non-blocking, so two harnesses starting at once collide

- Every jouzu start calls `applyProfile` (`runtime/dist/main-cli.js:244`), which acquires a
  `profile.lock` via atomic `openSync(path,'wx')` (`runtime/dist/profile-manager.js:303` ->
  `runtime/dist/state-lock.js::acquireStateLock`). When the lock is held by a LIVE pid it THROWS
  immediately (`onBusy` -> `ProfileStateError 'another profile operation is in progress'`) -
  there is no wait/retry. The process then exits code=1.
- Because `JOUZU_HOME` is shared per harness (`<harness_dir>/jouzu-home`, jouzu-adapter:41), TWO
  sessions of the same jouzu harness that start together both apply the profile and one loses.
- Evidence: concurrent-create probe - session A -> `starting_failed`, `startError: adapter
  protocol: config/set failed: the adapter closed its stdout`; the adapter log carried
  `Jouzu profile state is unreadable: another profile operation is in progress` and
  `[jouzu-adapter] harness exited code=1`.
- Explains: **concurrency/cross-talk** (`session A is active` fails), **lifecycle fork**
  (`502 adapter_crash` - a fork starts a second harness while the first's profile op is in
  flight; it is timing-dependent, a plain fork succeeds).
- Fix (jouzu runtime/adapter, jouzu's own code): the profile apply on a session start must be
  idempotent/converged-once, or the lock must wait-and-retry (bounded) instead of throwing, or
  the hub must serialize jouzu harness starts. The correct owner of the fix is the jouzu side
  (the lock is jouzu's); do NOT paper over it in the hub.

### Status

Both located. NOT yet fixed (jouzu adapter + jouzu runtime changes; to be applied on jouzu's own
code and verified on jouzu's own suite). The earlier 'three gaps' wording is superseded: it is
two mechanisms across four symptoms.
