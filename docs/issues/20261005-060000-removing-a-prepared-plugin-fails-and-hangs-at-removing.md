# 20261005-060000 — removing a PREPARED plugin fails (a live process holds its files) and the state hangs at `removing`

Recorded 2026-10-05. Found by `tests/plugins/a-plugin-is-installed-listed-enabled-and-removed.py`
(install -> prepare -> disable/enable -> REMOVE, over /v1). Owner: **HUB**.

## Observed (real)

```
POST /v1/plugins/pi/prepare   -> ready (materialises the runtime)
POST /v1/plugins/pi/disable ; enable
DELETE /v1/plugins/pi         -> 202 removing
GET  /v1/plugins/pi           -> removing FOREVER (never 404)
```

Hub log:
```
ERROR agent_hub_plugins::routes: detached remove failed error=io: The process cannot access the file because it is being used by another process. (os error 32)
```

## Two defects

1. **A prepared plugin cannot be removed**: `prepare` materialised the runtime and something
   (the adapter / a node child) still holds files under the plugin dir, so `finish_remove`'s
   `remove_dir_all` hits Windows `os error 32` and fails.
2. **A failed remove leaves the state `removing` FOREVER**: the detached task logs the error but
   the plugin row stays `removing` - the caller can never learn it failed, and the resource
   lies. The contract's `state` includes `failed`; a remove that failed must land there (or
   revert), never hang.

(Without a prior `prepare`, remove worked in a probe - so it is the prepared/running state that
triggers it.)

## Fix direction (HUB - trace before coding)

- Before removing, STOP anything the hub started for that plugin (its adapter process), and
  confirm it exited, then delete. A removal that cannot stop the process must report `failed`,
  not `removing`.
- A failed remove must set a terminal state (`failed`) with a reason, never leave `removing`.

## Status

recorded, NOT fixed.

## UPDATE (2026-10-05): defect 2 FIXED; defect 1 needs a manager change

- **Defect 2 FIXED**: a failed remove now records `failed` (a terminal state) instead of
  hanging at `removing` forever (`Plugins::fail_remove`, wired in the remove route). Verified:
  remove of a prepared plugin now reads `failed`, not `removing`.
- **Defect 1 NOT fixed**: `Adapters` caches ONE adapter process per harness (a `RequestHandle`
  in a map), but the `AgentBus` (which owns the `Child`) is moved into the pump task, so
  `stop_harness` (remove the map entry) does NOT kill the child - it stays alive (observed: a
  `node` process survives) and still holds the plugin's files, so `remove_dir_all` keeps failing
  with os error 32. The manager needs a real way to STOP a cached adapter (keep a kill handle /
  the bus, or a shutdown signal) - a design change, recorded rather than guessed.

Net: the resource is now HONEST (`failed`) but the remove still cannot complete when the plugin
was prepared. Both are the SAME issue; the test stays RED on the 'really removed' assertions.
