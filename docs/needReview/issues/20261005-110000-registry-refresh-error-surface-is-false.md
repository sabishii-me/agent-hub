# 20261005-110000 — the registry surface returns a false code, and every registry failure prints the SAME detail (its payload is discarded)

## Status
Defect A FIXED (crates/plugins/src/service.rs:31 — `#[error("{0}")]`, verified: each of the six
cases now names itself). Defect B OPEN — needs a `contract/errors.json` code (OWNER review required).

## Owner's direction
The registry IS the env var `AGENT_HUB_REGISTRY_URL`. EVERY "no registry / bad registry" case must
have a test: a BAD address FAILS, a CORRECT address SUCCEEDS. There is no registry to point at yet.

## Defect A (plain bug — no contract change): the error detail is hardcoded, not the payload
`crates/plugins/src/service.rs:31`:
```rust
#[error("no registry URL is configured")]
RegistryUrlMissing(String),
```
The format string has **no `{0}`**, so the `String` payload is discarded. All six distinct registry
failures — not-set, transport failure, non-2xx, read failure, not-JSON, no-plugins-array — render the
IDENTICAL detail. A caller cannot tell which happened, and a test cannot assert which case ran.

Evidence this fooled me: I set `AGENT_HUB_REGISTRY_URL` and got "no registry URL is configured" and
concluded the process did not see the env var. It DID (proved: in the same process `GET /v1/plugins`
returns `roots.given` from `std::env::var("AGENT_HUB_PLUGINS_DIR")` at request time). The message was
simply the hardcoded string. Diagnosis must not come from a message that erases its own payload.

Fix: `#[error("no registry URL is configured: {0}")]` or `#[error("{0}")]`, so the detail names the case.

## Defect B (contract change — OWNER review required): the code is false
All six cases map to `plugin_install_failed` (service.rs:55), whose `contract/errors.json:226` message
is "the plugin could not be **installed** (git failed, or the repository is not a plugin)", 502,
`retryable:true`. Nothing was installed; retrying never helps for the input-driven cases.

Observed: `POST /v1/plugins/registry/refresh` -> `502 {"error":"plugin_install_failed","detail":"no registry URL is configured"}`.

Proposed contract addition (needs owner approval):
```json
"registry_unavailable": {
  "http": 502,
  "retryable": false,
  "message": "the plugin registry could not be read: no AGENT_HUB_REGISTRY_URL is set, the address did not answer, the answer was not 2xx, or the body is not a registry (not JSON, or no plugins array). The detail names which."
}
```
Map `PluginError::RegistryUrlMissing(_) -> "registry_unavailable"`.

## The six cases (each needs a test, per owner)
| # | case | detail must contain |
|---|------|---------------------|
| 1 | no `AGENT_HUB_REGISTRY_URL` | "not set" |
| 2 | unreachable address | "registry fetch failed" |
| 3 | non-2xx answer | "HTTP 500" |
| 4 | body not JSON | "not JSON" |
| 5 | JSON without a plugins array | "plugins array" |
| 6 | a correct registry | (success; catalog restates it) |

Test: `tests/plugins/registry-refresh-updates-the-local-registry.py` — all six, real HTTP sources on
loopback. RED today on Defect A (identical details) and Defect B (wrong code).

## Also
The suite has been running a possibly-stale `target/debug/agent-hub.exe`: a running hub holds the
binary (Windows "Access is denied (os error 5)" on rebuild). THREE orphaned agent-hub.exe processes
were found this session. A stale binary silently makes every result meaningless. Record whether the
suite should assert the binary is freshly built / kill leftovers before running.
