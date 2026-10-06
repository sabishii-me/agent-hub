# 20261005-090000 — a plugin's `runtimeReady` stays false after a successful `prepare`

Recorded 2026-10-05. Found while moving the tests to REAL artifact installs. Owner: **HUB**.

## Observed (real)

Install the published `pi` artifact (registry.json, url+sha256), then:

```
POST /v1/plugins/pi/prepare
-> 200 {"harnessId":"pi","runtimeReady":true,"ready":true,
        "package":"@earendil-works/pi-coding-agent","version":"0.85.1",
        "target":".../plugins/pi/runtime/dist/cli.js","detail":"installed ...@0.85.1"}
GET /v1/plugins/pi
-> "runtime": {"package":"@earendil-works/pi-coding-agent","version":"0.85.1","target":null},
   "runtimeReady": false
```

The CLI **is on disk** (`.../plugins/pi/runtime/dist/cli.js` exists). So the prepare really
happened, but the RESOURCE reports `runtimeReady: false` and `runtime.target: null`.

## Why it is a defect

The contract (GET /v1/plugins): "`runtimeReady` is a fact about the filesystem (the manifest's
command exists), not a claim about the harness working." The command EXISTS, so `runtimeReady`
must be `true`. The resource is the truth (ARCHITECTURE §13.3) - and it is wrong here. It also
makes `runtime.target` null though the target was resolved.

## Impact

A client that shows "runtime ready" (and any test/automation that waits on it) sees `false`
forever after a successful prepare - it looks un-ready / stuck. This blocked the real-install
test flow (it waited on `runtimeReady`).

## Fix direction

`view()` must compute `runtimeReady` from the filesystem (the manifest's command present under
the plugin dir) and fill `runtime.target`, exactly as the contract says - not whatever it does
now. Trace `crates/plugins` `view`/`row_for_dir` and `runtimeReady` before changing.

## Status

recorded, NOT fixed.
