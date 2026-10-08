# 20261008-063000 - removing a plugin with OPEN SESSIONS is not refused (409 `plugin_in_use`)

**State:** OPEN (fixing). Owner: HUB.

## What the contract says

`contract/openapi.json` DELETE /v1/plugins/{id}: "...a harness with open sessions is refused rather
than yanked out from under them (**409 `plugin_in_use`**, naming the sessions)." The same rule is on
POST install ("A harness with open sessions is refused (409 `plugin_in_use`) rather than replaced
under a running process").

## What the hub does

`crates/plugins/src/service.rs`: `PluginError::InUse` and its code `plugin_in_use` EXIST, but
NOTHING ever constructs `InUse` - `begin_remove` (and `begin_install`) never consult the sessions
domain. So removing a plugin that has a live session proceeds instead of being refused.

## Fix

Before a remove (and a replace-install) of a harness, consult the sessions domain for OPEN sessions
on that harness. If any, refuse 409 `plugin_in_use`, naming them. This needs the plugins domain to
reach the session store (a dependency that must be wired deliberately, not by importing sessions
into plugins blindly).

Status: recorded; the harness functional test must pin the REAL behaviour until this is fixed.
