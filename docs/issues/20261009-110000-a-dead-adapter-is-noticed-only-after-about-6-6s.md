# 20261009-110000 - a dead adapter is noticed late and unstably (1s .. 20s), leaving the session `active`

**State:** OPEN (diagnosed; fix designed, not yet written). Owner: hub
(`crates/adapter/src/bus.rs` reader + `crates/sessions` reconciliation).
**Severity:** after its adapter process dies, a session keeps reporting `active` with no process
behind it for a variable time — measured **1.26s to 20.7s** across runs, same binary. That is the
exact dishonesty documented as `20261005-020000`, and it makes
`tests/interrupt/killing-the-adapter-leaves-an-honest-state.py` timing-dependent.

## Measured (dev, real pi adapter, Windows) — instrumented with monotonic traces

The path from "adapter killed" to "session reconciled off active" has three stages. Traces on a fast
run (`FLIPPED +1.26s`):

    reader first EOF            @0
    reader ending (task done)   @1027     <- ~1000ms, DETERMINISTIC
    reader task about to end    @1027
    pump recv->None             @1508     <- ~481ms scheduler delay
    watcher got death / quarantine ...    <- ~20-900ms more

Across runs the SAME code flipped at **1.26s, 2.24s, 2.53s, 6.65s, 12.82s, 20.71s**. Two parts:

### (a) ~1s deterministic: the reader's EOF debounce
`crates/adapter/src/bus.rs` reader: on `Ok(None)` (EOF) it does `EOF_RETRIES = 20` × `sleep(50ms)`
before concluding the adapter is gone. That debounce exists to avoid mistaking a **transient EOF** (a
pipe read racing the adapter's own flush) for a dead adapter (TASK-048 rework). It is the floor:
**every** death costs ~1s.

### (b) seconds, variable: the death is only noticed asynchronously
After the reader ends and drops the channel sender, the pump task's `recv()` returns `None` and sends
the death to `spawn_death_watcher`, which calls `quarantine_session` (a DB write). Under load this
handoff is delayed by hundreds of ms to many seconds. **Nothing else observes the death**: a `GET
/v1/sessions/{id}` does not check whether the process is gone, it reads the stored status.

## Fix (designed)

Two independent, additive changes — neither touches an adapter:

1. **Confirm death by the child, not by a timer.** On stdout EOF the reader should ask the `Child`
   directly (a non-blocking `try_wait`): exited -> real death, break at once; still running ->
   transient EOF, keep reading. This removes the ~1s floor and is MORE correct than a fixed count
   (a slow-yet-alive adapter can no longer be detached by 20 lucky EOFs).

2. **Reconcile on read.** A management read of a session whose process is gone (`is_running` false)
   must reconcile it then and there (off `active`), instead of relying solely on the async death
   watcher. A caller must never read `active` for a process that is not there, no matter how busy the
   runtime is.

The test (`killing-the-adapter-leaves-an-honest-state.py`) is already made to POLL (assert the
property, bounded) so it stops depending on this latency; the latency is fixed here, at the source.
