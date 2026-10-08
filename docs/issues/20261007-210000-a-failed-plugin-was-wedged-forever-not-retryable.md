# 20261007-210000 - a `failed` plugin was wedged forever (not retryable): every retry answered 409

**State:** FIXED (product change, verified).  
**Severity:** T0 - a failure was permanent. The contract's `failed` is "retryable"; here it could
never be retried.

## Observed

With install now really preparing the runtime (docs/issues/20261007-200000), the `deepseek` harness
fails its runtime prepare and reports `failed` with the honest reason
(`deepseek-adapter: dsh runtime not found: .../<id>/runtime/lib/bin.js`). A retry then answered:

```
POST /v1/plugins {artifact deepseek} -> 202 -> ready                    (1st)
POST /v1/plugins {artifact deepseek} -> 202 -> failed                   (2nd)
POST /v1/plugins {artifact deepseek} -> 409                             (3rd)  <-- wedged
```

## Cause

`Plugins::busy(id)` returned true for ANY op present, and `Op::Failed` is `enter`ed on every failure
with no `leave`. So a failed plugin stayed "busy" forever and every later install/remove was refused
`InProgress` (409). `Failed` is a persistent VERDICT (`state = "failed"`), not an operation in flight.

## Fix

- `state::Ops::in_flight()` = installing | removing | preparing; `busy()` uses it, so `Failed` no
  longer counts as busy.
- `Ops::clear_failed()`; `Plugins::enter` clears a previous `Failed` when a NEW operation starts, so a
  retry supersedes the old verdict (a reinstalled-and-succeeded plugin is not shown `failed`).

## Verified

`deepseek` in 3 consecutive installs: `['failed', 'failed', 'failed']` - each retry ACCEPTED (202,
never 409), each failure honest. The `failed` state is terminal AND retryable, as the contract says.
