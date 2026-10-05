# Test review — every fake assertion, and the real bug it hid

Recorded 2026-10-05. Rule: the deliverable is the PRODUCT, not the tests. A test that passes
whatever happens is `print green`; it is reviewed here, the REAL behaviour it should assert is
stated, and where the product is wrong the bug goes to `docs/issues/` with a code snapshot so
each fix can be reconciled.

## How a test is judged

- A `t.check(ok, ...)` is only a REAL test if `ok` is a SPECIFIC fact the contract/design
  requires. `x is not None`, `x in (<many>)`, and truthiness of a variable that is always set are
  NOT tests: they accept the failure the test exists to catch.
- For each such assertion: state the contract/design requirement, what the product ACTUALLY does
  (measured), and the bug id if it is wrong.
- The fix updates the PRODUCT; the test then asserts the now-true fact. Never the reverse.

## Ledger

| # | test:line | fake assertion | required (source) | measured | verdict |
|---|---|---|---|---|---|
| 1 | interrupt/killing-the-adapter-leaves-an-honest-state.py:43 | `st is not None` ("hub still answers") | after the adapter child dies the session must NOT claim `active` (the file's own docstring) | status stays `active` | **BUG 20261005-020000** (pre-existing, control at `ab2f1fa^`) |
| 2 | interrupt/killing-the-adapter-leaves-an-honest-state.py:57 | `settled is not None` (any terminal) | a turn with a dead adapter must not be `completed` | `failed` (so passes, but the assertion allowed `completed`) | assertion tightened |
| 3 | interrupt/a-provider-that-hangs-does-not-hang-the-turn.py:49 | `final is not None` + `!= active` | TBD (measure) | TBD | pending |
| 4 | interrupt/a-killed-hub-recovers-its-sessions.py:46 | `st in ("active","needs-repair","readonly","starting_failed")` — accepts `active` | ARCHITECTURE §21 N2: after restart an `active` session with no process -> `needs-repair` | product CORRECT: `needs-repair` | FIXED: assertion now `== "needs-repair"`; passes |
| 5 | interrupt/a-cancelled-turn-releases-the-session.py:73 | `ended in ("cancelled","interrupted")` | a CONFIRMED cancel -> `cancelled` (contract/v1.json turn.ended; `interrupted` is the unconfirmed/restart case, §21 N2) | pi yields `cancelled` | FIXED: assertion now `== "cancelled"`; passes |
| 6 | interrupt/killing-the-adapter-during-a-cancel-settles-the-turn.py:43 | `ended in ("cancelled","interrupted","failed")` | TBD | TBD | pending |
| 7 | interrupt/killing-the-adapter-mid-turn-does-not-hang-the-turn.py:47 | `ended in ("failed","interrupted","cancelled")` | TBD | TBD | pending |
| 8 | interrupt/killing-the-adapter-mid-turn-does-not-hang-the-turn.py:54 | `st in ("needs-repair","starting_failed","readonly","active")` — accepts `active` | TBD | TBD | pending |
| 9 | interrupt/the-network-drops-mid-turn-and-the-turn-settles.py:63 | `ended in ("failed","interrupted","cancelled")` | TBD | TBD | pending |
| 10 | interrupt/send-immediately-after-cancel.py:47 | `tr2["status"] in (202,409,422,503)` | TBD | TBD | pending |
| 11 | interrupt/cancel-is-idempotent-and-bounded.py:39 | `tr["status"] in (202,409,503)` | TBD | TBD | pending |
| 12 | concurrency/many-sessions-cancel-at-once.py:43 | `started >= 1` (of N) | TBD | TBD | pending |
| 13 | presets/plan-mode-is-applied-and-reports.py:40 | `reported is True or warning` (a warning passes) | TBD | TBD | pending |
| 14 | lifecycle/...forks...:56 | `src in ("active","closed")` | TBD | TBD | pending |
| 15 | model/a-real-model-answers-with-a-confirmed-identity.py:48,67 | `a1 is not None` | TBD | TBD | pending |
| 16 | plugins/...:42,51 | `in (409,403)` / `in (200,404)` | TBD | TBD | pending |
| 17 | presets/a-preset-is-listed-and-applied.py:43 | `r2["status"] in (202,400,422)` | TBD | TBD | pending |
| 18 | provider/...:27,52 | `in (400,422)` / `in (200,202,204)` | TBD | TBD | pending |
| 19 | lifecycle/the-session-crud...:63,67 | `in (200,409)` / `in (200,202,204)` | TBD | TBD | pending |
| 20 | connections/...:39 | `in (200,204,404)` | TBD | TBD | pending |
| 21 | tools/the-model-really-runs-a-tool.py:64 | `any(n in (...6 names...))` | TBD | TBD | pending |

## Rules for this review

- The snapshot of the code under test is taken at the moment the bug is recorded
  (`docs/review/snapshots/<issue-id>-<sha>.txt`), so the fix can be reconciled against it.
- A test is not "fixed" by relaxing it. If a required fact is not yet produced by the product,
  the bug is recorded and the test asserts the required fact (it goes RED until the product is
  fixed).
