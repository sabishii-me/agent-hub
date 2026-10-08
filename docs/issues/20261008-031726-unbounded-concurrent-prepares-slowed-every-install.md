# 20261008-031726 - unbounded concurrent `runtime/prepare` slowed EVERY install (hub had no scheduling)

**State:** FIXED (hub scheduling; verified).  
**Severity:** the hub is a concurrent service, but its heavy work ran unbounded, so firing several
installs made them ALL slower than doing them one at a time - the opposite of what a concurrent
service should do.

## Measured (the three OFFICIAL released artifacts, same machine)

| arrangement | total for 3 installs |
|---|---|
| serial (one prepare at a time) | 381s |
| **unbounded concurrent (old)** | **634s** |
| **bounded prepare (new, gate = 2)** | **313s** |

`npm install` of a runtime costs ~50s alone but ~600s when three run at once: they contend for
CPU/disk. So "start everything immediately" was the slowest policy.

## Cause

`crates/plugins/src/routes.rs` spawned every install's detached task with no bound; the adapters'
heavy `runtime/prepare` (an npm install) therefore all ran at once. There was no hub-level
scheduling: the only choices were "one op at a time" (a previous T0) or "all at once" (this).

## Fix

A hub-internal scheduling gate, NOT a user choice. `PluginsState.prepare_gate` (a semaphore) bounds
how many plugins run `runtime/prepare` at once (default 2, `AGENT_HUB_PREPARE_CONCURRENCY` to tune).
Downloads and landing stay concurrent; only the heavy prepare is bounded. The user still fires as
many installs as they like - each is accepted 202 at once and shows `installing`/`preparing`, never
"stuck". Measured with the gate: 313s for three (fastest of the three policies), `overlap=True`, and
all three reach `ready`.
