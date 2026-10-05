# 20261005-050000 — a plugin installed through /v1 is not usable until the hub restarts

Recorded 2026-10-05. Found by the END-TO-END test `tests/lifecycle/the-whole-chain-plugin-to-session-to-tools-over-v1.py`
(empty plugins root -> install -> session -> tools, all over /v1). Owner: **HUB**.

## Observed (real, reproducible)

On a hub whose plugins root is EMPTY:

```
POST /v1/plugins {source:{url:<local pi plugin path>, ref}} -> 202 installing
GET  /v1/plugins/pi                                       -> ready   (the install DID land)
GET  /v1/harnesses                                        -> []      <-- the installed harness is MISSING
POST /v1/sessions {harnessId:"pi"}                        -> 404 harness_not_found
```

Then RESTART the hub on the SAME data dir:

```
GET  /v1/harnesses -> ["pi"]
```

So the install lands the directory and the plugin row reaches `ready`, but the harness is
**not registered with the running hub**: `/v1/harnesses` stays empty and no session can use
it, until a restart re-scans the plugins root.

## Why it matters

The contract's `POST /v1/plugins` is how a plugin is INSTALLED ("install a plugin ... what
lands is a directory with a manifest ... in the hub's OWN plugins root"). An install that
does not make the plugin usable until a restart is not an install - the operator has to bounce
the hub. The harness registry (`crates/harnesses`) is populated by scanning `AGENT_HUB_PLUGINS_DIR`
at START; `finish_install` (crates/plugins/src/service.rs) does not (re)register or tell the
adapters registry about the new plugin.

## Why every test missed it

Every test used `Hub(plugins_src=...)`, which COPIED the plugin into the plugins root BEFORE
the hub started - so the harness was always present at boot and the install path was never
exercised. That backdoor is now removed: `tests/lib/hub.py` installs through `/v1` after start,
which is how this surfaced. This is also why "one harness, N sessions" passed while the real
install->use chain was broken.

## Fix direction (HUB - decide, do not guess the mechanism)

After `finish_install` (and `remove`) the hub must (re)compute the harness registry so a just
installed plugin is immediately usable - i.e. the adapters registry must be able to rescan/
register without a restart. Trace how `HarnessRegistry`/adapter list is built at boot and provide
the in-process update; do NOT paper over it with a hidden restart.

## Snapshot

`docs/review/snapshots/20261005-050000-<sha>.txt`.

## Status

**FIXED**. The install/remove routes now re-scan the adapters registry (and scan prunes a
removed harness), so an installed plugin is usable immediately. The end-to-end test
`tests/lifecycle/the-whole-chain-plugin-to-session-to-tools-over-v1.py` is 13/13.
