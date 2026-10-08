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

## Where the fix BELONGS (correction)

A first attempt put a semaphore in the HUB (`PluginsState.prepare_gate` +
`AGENT_HUB_PREPARE_CONCURRENCY`). That was REVERTED: it leaked harness-adapter knowledge into the
hub - the hub must not know that "a prepare is heavy" or that it is a harness. The hub knows no
package names and installs no harness; it must not schedule by plugin type either.

The scheduler belongs to the PLUGIN (the adapter owns its own `runtime/prepare` and may bound it),
or is a GENERIC hub capability that treats every long operation alike (never a per-type rule, never
a plugin-specific variable). The measurement stands (unbounded concurrent prepares are the slowest
policy); WHERE the bound lives is the plugin's / a generic hub's decision, not a harness special
case in the hub.

STATUS: OPEN - the hub-side special case is reverted; the proper home is to be decided (plugin's
adapter, or a generic hub-side bound).
