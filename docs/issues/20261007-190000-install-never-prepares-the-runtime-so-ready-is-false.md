# 20261007-190000 - install never prepares the runtime, so `ready` is false and the time is fake

**State:** OPEN (fixing)  
**Severity:** T0 - installing a harness adapter leaves it UNUSABLE, reports `ready`, and takes ~0.1s
instead of the real tens of seconds.

## Measured (dev)

```
POST /v1/plugins {artifact pi}  -> 202          (0.00s)
GET  /v1/plugins/pi             -> ready        (0.11s)
  runtime.package: @earendil-works/pi-coding-agent@1.0.0
  runtimeReady: false            <-- the runtime is NOT there
  runtime.target: null           <-- the declared command does not exist
  plugin dir: extensions/ presets/ ...   (the ADAPTER only; no runtime/)
```

The manifest declares `runtime.command = ["node","runtime/dist/cli.js"]`. After install that file
does not exist and `runtimeReady` is false, yet the state is `ready`.

## Cause

`crates/plugins/src/routes.rs` POST install: the detached task calls only `finish_install`, which
lands the adapter tree and commits the row. It NEVER calls `runtime/prepare`, so the adapter's
runtime is never materialised. The runtime is produced ONLY by the separate
`POST /v1/plugins/{id}/prepare` route, which the owner says is wrong: an install of an adapter MUST
prepare its runtime.

## Fix

The install command must be one command: land the adapter, then materialise the runtime, then verify
the declared command exists - state `installing` -> `preparing` -> `ready`. Install therefore takes
the real time (tens of seconds), and `ready` means the harness is actually usable.
