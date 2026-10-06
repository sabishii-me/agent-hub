# 20261005-140000 — DELETE /v1/plugins/{id} for a nonexistent id answers 409 conflict with a FALSE detail ("is a deployment directory")

## Status
OPEN — located. The message LIES; the code/status is a separate (contract-unpinned) question.

## Observed (RELEASE build, real hub, no override)
```
DELETE /v1/plugins/nope
-> 409 {"error":"conflict","detail":"plugin `nope` is a deployment directory, not installed by this hub"}
```
`nope` exists nowhere. It is not a deployment directory; it is not a plugin at all. The detail is
false.

## Root cause (LOCATED)
`crates/plugins/src/service.rs:518-525 begin_remove`:
```rust
let row = self.db.plugin(id)?.ok_or_else(|| PluginError::NotInstalledByHub(id.into()))?;
if row.installed_at.is_none() {
    return Err(PluginError::NotInstalledByHub(id.into()));
}
```
`NotInstalledByHub` (service.rs:17) displays as "plugin `{0}` is a deployment directory, not installed
by this hub". So TWO different situations are collapsed into ONE error:
- the id is NOTHING (no row, no directory)  -> reported as "is a deployment directory" (FALSE);
- the id is a DEPLOYMENT directory (a row-with-no-installed_at is not how a deployment is found; a
  deployment is a directory with a manifest and no row at all) -> arguably correct.

A caller cannot tell "it was never here" from "it is somebody else's directory", and one of the two
messages is a lie for the other case.

## What is definitely wrong (no contract change needed)
The DETAIL must be true. A nonexistent id must not be described as a deployment directory.

## What needs a decision (contract unpinned — matrix row D3)
The contract's `DELETE /v1/plugins/{id}` lists only `200 success`; it does NOT pin the nonexistent
case. Options: 404 `not_found` (it does not exist) vs 409 (it exists but is not the hub's). This is
the owner's call. Until then the fix at minimum must not LIE: separate the two cases so each says what
it is.

## Test
A release-build test asserting the exact code + that the detail does not claim "deployment directory"
for an id that exists nowhere. RED today.
