# 20261009-110000 - a dead adapter is noticed only ~6.6s later (the session keeps claiming active)

**State:** OPEN (measured; latency to investigate). Owner: hub (`crates/sessions` death detection).
**Severity:** for ~6.6s after its adapter process dies, a session still reports `active` with no
process behind it — the exact dishonesty documented as `20261005-020000`. It DOES reconcile, but far
too slowly to be "the hub notices".

## Measured (dev, real pi adapter, Windows)

    session active -> kill the adapter child -> poll GET /v1/sessions/{id}
    flipped to `needs-repair` after 6.65s

`tests/interrupt/killing-the-adapter-leaves-an-honest-state.py` waits 1.5s and so fails
intermittently (5/6, 6/6 depending on timing). The property holds; the LATENCY is the defect.

## Where it is

`crates/sessions/src/runtime.rs`: the pump task signals a death only when the adapter's **stdout
closes**. On Windows a killed child's pipe does not necessarily surface EOF to the tokio reader
promptly, so the reader may wait on its own read/buffer before it observes the close. The death
watcher (`spawn_death_watcher`) then quarantines the session on `spawn_blocking`.

## Fix direction (to decide, not guessed)

- the runtime can OBSERVE the child directly (`try_wait`/`wait` on the `Child`) instead of inferring
  death from pipe EOF, so a dead child is seen at once; and/or
- the session read path checks `is_running(sid)` (the handle's `is_alive`) and reconciles on the spot,
  so a caller never reads `active` for a process that is gone.

Both are hub-side; neither touches the adapter. The test is being made to POLL (assert the property,
not a fixed sleep) so it stops being timing-dependent while this latency is fixed.
