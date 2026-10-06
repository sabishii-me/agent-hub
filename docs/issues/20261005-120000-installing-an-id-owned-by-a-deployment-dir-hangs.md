# 20261005-120000 — POST /v1/plugins hangs when the id already lives in a DEPLOYMENT directory

## Status
OPEN — located to a single request; exact blocking call NOT yet located (needs a controlled trace).

## Reproduced (real hub, this session)
```
AGENT_HUB_PLUGINS_DIR=<dep>,  <dep>/pi/manifest.json present (a deployment dir)
POST /v1/plugins {source:{url:<local pi path>, ref:"fix/runtime-placement"}}
-> the request NEVER returns (client TimeoutError after 15s)
   WHILE the hub still answers GET /v1/plugins -> 200 (the hub is alive)
```
So it is a hang on ONE request, not a hub-wide deadlock. The staging directory fills with the
cloned tree (`.git`, `manifest.json`, ...), so the clone completed; the stuck step is later.

## What IS located
- `begin_install` (crates/plugins/src/service.rs:336-430) does the entire clone/copy synchronously
  and, on success, returns `InstallIntent::Proceed`; only THEN does the handler send `202` + start
  the detached `finish_install`. Since no `202` arrived, the hang is inside `begin_install` OR the
  `spawn_blocking` that runs it.
- `begin_install` has **no deployment-directory check** (the `NotInstalledByHub` guard exists only
  in `begin_remove`, service.rs:494). So a deployment-owned id is NOT refused up front — the code
  proceeds to stage and land it.
- `finish_install` calls `agent_hub_db::install(&db, id, &layout, &source_dir, None)` with
  `layout = <dep>/pi` — a directory that ALREADY exists as a deployment dir. This is the prime
  suspect (replacing an existing, possibly-in-use directory on Windows, or a DB lock), but it was
  not instrumented, so this is a HYPOTHESIS, not a located cause.

## Correct behaviour (per the contract)
`POST /v1/plugins` for an id that lives in a directory the hub does not own must be REFUSED with
**409 `conflict`**, naming the directory (the same rule `begin_remove` enforces; v1 POST: "refused
with 409 ... An id living in a directory this hub does not own"). It must NOT clone, must NOT land,
must NOT hang.

## Test
`tests/plugins/install-refusals-have-honest-codes.py` case B1 — asserts 409 `conflict` naming the
directory. RED today (hangs).

## Next (controlled)
Trace the blocking task for this exact input: log entry/exit around `begin_install`'s staging,
manifest read, and around `agent_hub_db::install`, to see which one does not return. Do not pick a
cause before that trace.
