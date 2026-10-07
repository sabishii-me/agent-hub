# 20261003-130000 — removing a deployment plugin dir returns 500, contract says 409

Recorded: 2026-10-03. Found by:
`tests/plugins/a-plugin-is-installed-listed-enabled-and-removed.py`. Owner: **HUB**. Status: FIXED (crates/plugins maps the deployment-dir refusal to 409 conflict, not 500).
recorded, NOT fixed.

## Repro (real)

The hub serves a plugin from its own plugins root that the DEPLOYMENT placed there (copied
in, not installed by the hub). `DELETE /v1/plugins/pi`:

```
500 {"error":"plugin_remove_failed","detail":"plugin `pi` is a deployment directory, not installed by this hub"}
```

## Why it is a defect

The contract's `DELETE /v1/plugins/{id}`: "A directory a deployment put on the search path is
read-only to the hub (**409**, naming the directory)". 409 conflict = the right status;
500 `plugin_remove_failed` claims the *filesystem* failed, which is false - nothing was
attempted. A consumer retrying a 500 (it is marked retryable) would retry forever.

## Fix direction

The "deployment directory, not installed by this hub" refusal is a CONFLICT, not a remove
failure: return 409 with a code that names the conflict (the contract's message already
says 409). Map the `NotInstalledByHub` error to a 409 code, not `plugin_remove_failed`.

## Verify

`python tests/plugins/a-plugin-is-installed-listed-enabled-and-removed.py` ->
"the hub refuses to remove a deployment dir" with status 409 must pass.

## Status: FIXED (2026-10-05)

`crates/plugins/src/service.rs`: `PluginError::NotInstalledByHub` now maps to the contract code
`conflict` (**409**), not `plugin_remove_failed` (500). The detail still names the directory.
Verified: `tests/plugins/a-plugin-is-installed-listed-enabled-and-removed.py` -> 9/9
("the hub refuses to remove a deployment dir" now sees 409).

Also (`tests/run.py`): the default layer list was missing `plugins`, `harnesses` and `presets`,
so this failure never ran in a default `run.py`. Completed the list; the suite is now **32/32**.
