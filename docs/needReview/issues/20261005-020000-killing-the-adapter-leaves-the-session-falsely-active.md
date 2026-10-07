# 20261005-020000 — killing the adapter child leaves the hub reporting the session `active`

Recorded: 2026-10-05. Found by: making the test assertion SPECIFIC (it was `st is not None`,
which accepted anything). Owner: TBD (hub side; do NOT attribute before a control).

## Observed (real, no inference)

Real hub, real pi adapter. The session is `active`. The adapter's node child process
(`child_processes(hub.child.pid)`, one `node`) is killed; the HUB survives. Then:

- `GET /v1/sessions/{id}` still reports **`status: active`** - the hub did not notice the child
  whose stdout pipe carried its protocol died. It claims a live session with no process.
- A new turn is **accepted (202)**, then settles `ended: failed` (it does not hang).

## Why this was hidden

`tests/interrupt/killing-the-adapter-leaves-an-honest-state.py:43` asserted
`t.check(st is not None, "the hub still answers about the session")` - ANY status passes, so
"still says active" passed. Same file `:57` asserted `settled is not None` - any terminal. The
test's own docstring says "the session must not keep claiming `active` with no process" and yet
it never asserts that. It is a `print green` assertion, not a test.

## The contract/design

ARCHITECTURE §21 (N2) covers the RESTART case (at boot no process runs): `active` (process gone)
-> `needs-repair`. It does NOT state the live case (hub alive, the adapter child dies). So the
specific required status here is NOT contract-dictated; what IS required by the test's own claim
and by "no fake" is: the hub must NOT keep reporting `active` when the process is gone, and a
turn must not hang. Those two are assertable exactly:
  - `status != "active"` after the child dies;
  - a turn is either refused (non-2xx) or reaches `ended` within a bound.

## Open question (needs a control, not a guess)

Is "stays active" a pre-existing hub defect, or did `ab2f1fa` (removing the bounded control
waits / making control requests bounded only by process liveness) remove the very death-detection
that used to flip the status? MUST be answered with a controlled comparison (run this probe at the
pre-`ab2f1fa` commit), not by assumption. Recorded here BEFORE any fix.

## CONTROL (2026-10-05): it is PRE-EXISTING, not from my change

Built `ab2f1fa^` (1a2efbc, BEFORE my timeout/liveness edits) in a git worktree and ran the
SAME probe against THAT binary (same pi adapter, same kill of the node child):

```
before kill: active
adapter children: [56020]
AFTER kill, session status = 'active'
```

Identical. So the 'still claims active after the adapter child dies' behaviour is PRE-EXISTING
(not introduced by `ab2f1fa`), and my change did not cause it. What I DID do wrong is hide it:
the test asserted `st is not None`, so it passed on the very symptom its docstring says must not
happen. Fix direction: the hub must not keep `active` when the process behind it is gone; and
the turn must be refused or settle (it does settle `failed`, so that half is fine). The specific
status when a live hub loses its adapter child is NOT contract-dictated -> assert the RE
QUIRED property (`!= active`, no hang), not an invented exact value.

## Code under test (snapshot)

`docs/review/snapshots/20261005-020000-c8a3f36.txt` (hub @ `c8a3f36`, pi adapter @ `7b84cb0`).

The site: `crates/sessions/src/runtime.rs`, the per-session notification pump task
(`tokio::spawn(async move { while let Some(n) = notifications.recv().await { ... } })`). When the
adapter child's stdout closes, `notifications` ends and the loop EXITS; on that path nothing
updates the session's status, so the session row keeps `active` though no process serves it.
`is_running(sid)` still holds a handle in the `running` map, so callers also still believe it.

Reconciliation: the fix must, on pump end (child gone), settle the session honestly - the
design's `needs-repair` (an orphaned tail; ARCHITECTURE §21 N2) or an explicit failed state -
and it must be provable by the two asserted properties (never `active`; a turn does not hang).

## DESIGN GAP (must be decided, not invented)

ARCHITECTURE §21 N2 covers ONLY the restart case (at boot no process runs -> `active` becomes
`needs-repair`). §13 (invariants) does not state that a LIVE hub must notice its adapter child
dying and flip the session. So the required status on adapter death is NOT pinned by the design.
Per the rule (docs first; when silent, say so - do not invent), the fix is BLOCKED on a design
decision: when a live hub's adapter child dies, does the session go `needs-repair` immediately
(and a turn is refused until `reopen`), or stay readable-but-refusing, or something else?
The test asserts the two properties the file itself claims (`!= active`, no hang); the exact
status is left to the owner.

## Sites in the test suite that HID this

- `tests/interrupt/killing-the-adapter-leaves-an-honest-state.py:43` - `st is not None` (fixed: now `!= active`).
- `tests/interrupt/killing-the-adapter-mid-turn-does-not-hang-the-turn.py:54` - `st in
  ("needs-repair","starting_failed","readonly","active")` (fixed: now `!= active`).

Both files' docstrings SAY the session must not keep claiming `active`; both assertions
permitted exactly that. Confirmed by running the real system: kill the adapter child mid-turn ->
turn `failed`, session status still `active`.

## RESOLVED (2026-10-05)

Product fix (not the test):
- `crates/sessions/src/runtime.rs`: the per-session pump task now signals an adapter
  DEATH when its notification stream ends (the adapter's stdout closed): it sends
  `(sid, generation)` on a death channel. The generation distinguishes an unintended
  death from a deliberate stop/reopen/repair (which bumps it).
- `crates/sessions/src/service.rs`: `spawn_death_watcher()` consumes the channel; when the
  generation still matches and the row is `active`, it runs the existing `quarantine_session`
  (the §21 N2 rule, applied live: settle to `needs-repair`, settle open turns, block turns).
- `hub/src/main.rs`: wires the watcher at composition time.

Verified (real pi adapter): kill the adapter child on an IDLE session -> status becomes
`needs-repair` (was `active`); kill it MID-TURN -> the turn settles `failed` and the session
becomes `needs-repair`. `killing-the-adapter-leaves-an-honest-state` 5/5 (its 'st is not None'
and 'settled is not None' fake assertions replaced with the real properties),
`killing-the-adapter-mid-turn-does-not-hang` 8/8.
