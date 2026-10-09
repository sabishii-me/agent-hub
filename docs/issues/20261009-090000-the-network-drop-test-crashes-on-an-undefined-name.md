# 20261009-090000 - the network-drop interrupt test crashes on an undefined name (it never ran)

**State:** FIXED (this commit). Owner: tests.
**Severity:** `tests/interrupt/the-network-drops-mid-turn-and-the-turn-settles.py` raised
`NameError: name 'c' is not defined` at import time, so the test NEVER executed its assertions — a
green-looking file that proved nothing.

## Cause

Introduced editing `69968a4` (`fix(providers): the id is assigned by the TYPE...`): the provider POST
was renamed to bind `PROV`, and a line was added to read the id from the response but referenced `c`
(the response variable is `r`), and it overwrote `PROV` with the extract. Result:

    PROV = hub.post("/v1/model-providers", {...})
    PROV = ((c.get("json") or {}).get("provider") or {}).get("id")   # NameError: c

The test's own header already says what it proves (a network drop mid-turn settles the turn, never a
clean `completed`; the hub survives); it simply could not reach its assertions.

## Fix

Bind the POST response to `r` and read the provider id from it:

    r = hub.post("/v1/model-providers", {...})
    PROV = ((r.get("json") or {}).get("provider") or {}).get("id")

Then run it for real (pi; and the other harnesses) and confirm it passes.
