# Harness capability probes: a one-shot adapter must be reusable

The pi adapter is **one-shot** for capability probes: after answering `models/list`,
`presets/list` or `tools/list` with no session open it calls `process.exit(0)` ("a
probe never lingers"). The hub cached ONE adapter process per harness and never
noticed the exit, so a second capability call failed with `adapter_unreachable` /
"pipe is being closed".

## The fix

- `RequestHandle` carries an `alive` flag (an atomic) that the reader clears when the
  adapter's stdout reaches EOF. `Adapters::ensure_started` drops a DEAD cached handle
  and respawns, so any number of one-shot capability calls work.
- `Harnesses::gated` adds the envelope the contract requires (`harnessId`, `known`, and
  the list key when the adapter omitted it); previously it forwarded the adapter's
  reply verbatim, so `GET /v1/harnesses/{id}/models` was missing `harnessId`/`known`.

Verified live: `models` -> `presets` -> `models` -> `extensions` in sequence all answer
(each one-shot cycle respawns); `models` carries `harnessId:"pi"`, `known:true`. Adapter
test: a handle whose child exited reports dead.

## A dead session adapter is not "running"

`Runtime::is_running` now consults the cached handle's liveness (not just the map
membership), so a session whose adapter process EXITED is not reported running. This
makes the lifecycle truthful: `reopen` after the adapter dies RESTARTS it (instead of
returning "already running"), and the read-through views respawn. Verified live: kill
the pi-adapter process, then `reopen` -> the session restarts to `active`; a
read-through before that also respawns.
