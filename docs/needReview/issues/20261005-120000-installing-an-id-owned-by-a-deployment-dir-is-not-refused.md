# 20261005-120000 — installing an id that lives in a DEPLOYMENT directory is NOT refused: it is silently OVERWRITTEN (and sometimes hangs)

## Status
OPEN — located.

## Reproduced (real hub, this session, two runs)
Setup: `AGENT_HUB_PLUGINS_DIR=<dep>`, `<dep>/pi/manifest.json` present (a DEPLOYMENT directory the
hub does not own). Action: `POST /v1/plugins {source:{url:<local pi path>, ref}}`.

Run 1 (observed twice):
```
POST /v1/plugins  ->  202 {"pluginId":"pi","state":"installing"}
plugin state: installing (origin=deployment) -> ready (origin=hub)
<dep> after: ['.pi.outgoing', '.stage', 'pi']     <-- the deployment dir was REPLACED
```
Run 2: the SAME request sometimes HANGS (client timeout after 15-20s), while `GET /v1/plugins`
still answers 200 (the hub is alive; only that request is stuck). The `.pi.outgoing` rename on a
deployment dir that a process holds open on Windows is the likely cause of the intermittent hang —
a hypothesis, not located.

## Root cause (LOCATED)
`crates/plugins/src/service.rs:336 begin_install` has **no check** that the target id is one the hub
installed. It stages the source and returns `Proceed` for ANY id. Then
`crates/db/src/recovery.rs:177 install` UNCONDITIONALLY:
1. `rename(layout.target -> layout.outgoing)` if the target exists, then
2. `rename(staging -> layout.target)`.
It never consults `installed_at`/origin. So a DEPLOYMENT directory is treated as "the old copy",
moved to `.pi.outgoing`, and overwritten by the hub's own copy — and the row flips
`origin: deployment -> hub`.

By contrast `begin_remove` (service.rs:484-500) DOES enforce this (`NotInstalledByHub` -> 409
`conflict`). The install path is simply missing the twin guard.

## Contract
`POST /v1/plugins` (openapi.json): "An id living in a directory this hub does not own
(AGENT_HUB_PLUGINS_DIR, a deployment's ...) ... refused with 409". `plugin_remove_failed`/`conflict`
codes exist. The hub must REFUSE (409 `conflict`, naming the directory), clone nothing, land
nothing, and NEVER touch the deployment tree.

## Test
`tests/plugins/install-refusals-have-honest-codes.py` case B1 — expects 409 `conflict` naming the
directory. RED today (202 + overwrite, or a hang).

## Fix (unblocked — no contract change)
In `begin_install`, before staging, refuse when the id resolves to a directory the hub does not own:
if a directory for `id` exists under a searched root but has no hub-installed row (origin != "hub"),
return `PluginError::NotInstalledByHub(id)` (409 `conflict`). This is the exact twin of the
`begin_remove` guard. The id is not known until the manifest is read, so the guard belongs right
after the manifest id is extracted (before `enter`/`announce`/`Proceed`).
