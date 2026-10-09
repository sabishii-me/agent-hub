# 20261009-030000 - a DETACHED dsh server outlives every adapter, so remove cannot delete the plugin

**State:** OPEN. Owner: adapter (server lifetime) + hub (which processes `remove` stops).
**Severity:** the plugin can never be removed on Windows once a deepseek session has run; the remove
fails with `io: Access is denied. (os error 5)` and (before that) hangs at `removing`.

## Measured (dev, real dsh 0.2.0-rc.2, Windows)

- install `deepseek` -> ready; open a session (the adapter spawns `dsh web`).
- close the session; `DELETE /v1/plugins/deepseek` -> 202 `removing`.
- the hub log then says: `detached remove failed error=io: Access is denied. (os error 5)`.
- a `node .../runtime/lib/bin.js web --no-open --port 0` process is STILL RUNNING after the adapter
  was stopped, holding files under `plugins/deepseek/runtime/`, so `remove_dir_all` cannot delete them.

## Cause

The dsh adapter spawns ONE `dsh web` per DSH_HOME and `detached:true` + `unref()`s it, so it is shared
by every session of that home and SURVIVES the adapter that spawned it (by design - a sibling's
session must not kill the shared server). The hub's remove calls `adapters.stop_harness(id)`, which
stops the cached adapter process; the adapter's own shutdown kills the server it owns. Two holes:

1. `Bus::kill` did a HARD kill, so the adapter never ran its stdin-EOF shutdown (fixed: the hub now
   closes stdin, waits ~2s, then force-kills - the protocol's "die with the core").
2. Even so, the OWNER adapter (the one that spawned dsh) may not be the process the hub stops: the hub
   tracks one adapter per harness, but a session-per-adapter model leaves the owner alive (or already
   gone) while the server keeps running, and no remaining adapter owns it.

## Fix direction (needs a decision)

- the adapter must kill the WHOLE process tree of the server it owns (done: taskkill /T on Windows,
  process group on POSIX) AND `remove` must reach the OWNER; or
- the hub must stop EVERY adapter process for a harness before removing it (not just the cached one);
  and/or
- the server must not be detached across the OWNER's death - its lifetime must end when the last
  session adapter ends, or be adopted so any remaining adapter can stop it.
