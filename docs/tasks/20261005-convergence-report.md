# Convergence report — what the Python suite genuinely verifies (registry unpublished)

Written 2026-10-05. Owner order: do not fake-pass, do not fabricate a registry, do not let the test
stand in for the product. This states, honestly, what is verified, what is not, and why. The one
missing precondition behind every BLOCKED row is: the official registry release is UNPUBLISHED.

## Method
`python tests/run.py` classifies each file by EXIT CODE and its summary line into five SEPARATE
buckets - PASS / FAIL / BLOCKED / PARTIAL / CRASH. There is no "N/N passed". A BLOCKED result is a
missing real precondition, counted as UNVERIFIED, never a pass.

Full-suite result (45 files):
```
PASS    : 9
PARTIAL : 3    (some checks passed, some BLOCKED -> not fully verified)
BLOCKED : 33   (a real precondition is missing -> UNVERIFIED)
FAIL    : 0
CRASH   : 0
```

---

## One. What is genuinely verified now (real evidence)

Each row: the user capability, the real precondition, the /v1 operation, and the ACTUAL result -
not a filename, not a comment, not a 202.

### PASS (9) - no registry needed; independently reproducible

| capability | precondition | /v1 trigger | actual result that proves it |
|---|---|---|---|
| the served surface equals the contract | real hub + contract/ | boot self-check + GET /v1/openapi.json | 4/4: every declared route mounted; no parameterless GET answers 501 |
| status surface + models answer from the contract | real hub, no plugins | GET /v1/status, GET /v1/models | 8/8: shapes match the contract |
| a caller CANNOT install from an arbitrary url | real hub, NO registry | POST /v1/plugins {url|artifact} | 4/4: a source in no registry -> 403 plugin_not_in_registry; no dir lands |
| a release hub ignores the registry override | RELEASE binary + canary registry on loopback | POST /v1/plugins/registry/refresh then GET /v1/plugins/catalog | debug catalog = [OVERRIDE-WAS-USED], release catalog = [] -> opposite outcomes prove the release binary ignores AGENT_HUB_REGISTRY_URL |
| registry refresh error surface | real hub | POST /v1/plugins/registry/refresh at dead/500/non-JSON/no-array/ok addresses | 15/15: each failure answers contract code registry_unavailable (502) naming the case; a good source succeeds and the catalog restates it |
| release build error codes | RELEASE binary | POST /v1/plugins {} , {source:{}} , GET/DELETE/prepare missing, artifact variants | 8/8: 400 validation_failed for a bad body; 404 not_found for missing; 403 for an unlisted source (1 BLOCKED: the built-in refresh) |
| an uninstalled harness cannot be used | real hub, no plugins | GET /v1/harnesses, POST /v1/sessions | 6/6: /v1/harnesses empty with a note; session create -> 404 harness_not_found |
| one hundred loopback connections do not time out | real hub | 100 idle connections | 3/3 |
| skills CRUD + connections CRUD | real hub | /v1/skills* , /v1/connections* | 7/7 and 8/8 (token never echoed; delete idempotent) |

### PARTIAL (3) - refusal paths pass; authorization paths are BLOCKED
| file | passed | BLOCKED |
|---|---|---|
| install-refusals | 10/10: B6 (400), B7 (git source -> 403 at the gate), B8 (listed url + wrong sha256 -> 403), all leave no half tree | B1 (deployment-dir 409) - needs an authorized source first |
| install-reconciles | 3/3: no registry -> a real release, an unlisted url, and a git source are ALL refused 403 | the AUTHORIZED path (registry lists a release -> install) and the proxied-url case |
| the-release-hub-error-surface | 8/8 | the built-in refresh (registry asset 404) |

---

## Two. Registry-dependent capabilities: UNVERIFIED / BLOCKED

BLOCKED - the official registry release is UNPUBLISHED. These have NO verified result. An
"installation was refused" is NOT reported as "the install safety mechanism passed": with no
published registry there is no REACHABLE authorized install to contrast a refusal against, so a
refusal is consistent with BOTH a correct gate and a hub that simply has no registry. UNVERIFIED.

- plugin discovery (catalog from a real registry): BLOCKED.
- registry refresh from the real address: BLOCKED (asset is 404).
- authorized install (a registry-listed release -> installed): BLOCKED.
- the whole chain from an empty hub (install -> session -> real tool turn): BLOCKED.
- everything downstream needing an installed plugin: session CRUD/read-through, all interrupt/*
  (cancel, kill, recovery), approvals/*, concurrency/* , provider/* , presets/* , tools/* , model/* ,
  harnesses/discovery: BLOCKED (33 files).

Missing precondition for all of the above: the 'registry' release of sabishii-me/agent-hub must be
published (https://github.com/sabishii-me/agent-hub/releases/download/registry/registry.json,
currently HTTP 404).

### A second, independent blocker
Even with a registry, the session tests need a working model provider. The host's real provider
(~/.pi/agent/models.json -> 192.168.31.29:8990) answers 400 to its own /models with the host token
(verified OUTSIDE the hub - docs/issues/20261005-150000). The session/turn chain is blocked by the
provider too, independent of the registry.

### Already-installed-environment note (owner item three)
No test claims a from-empty-hub full chain. The local plugin at $PI_PLUGIN_DIR is used only as a REAL
SOURCE for refusal tests and for kind_of() labelling; it is NOT an installed hub plugin, no session
runs on it, and no test presents it as a prepared install. This run did NOT verify plugin discovery,
install, or a cold-start chain - those are BLOCKED.

---

## Three. Retracted earlier over-claims

| earlier claim | why retracted |
|---|---|
| "35/35 files passed" / any "all green" | the runner counted exit codes only; five files discarded a FAIL into exit 0 (20261005-160000) |
| "whole-chain 13/13 proves the system works" | it installed from a caller url with the test as the registry, and one assertion accepted a 409 as success; now BLOCKED |
| "registry-refresh 17/17 proves the registry works" | it configured a loopback registry URL and tested the refresh FUNCTION's error codes, not the trust boundary |
| "installed a plugin by registry id" | the hub resolves no id yet; the test supplied url+sha256. The authorized install is BLOCKED |
| "install is safe because refusals are 403" | refusals with no published registry prove only "no registry -> refuse", not "the right release was authorized" |
| "release hub installs nothing" measured once | the release binary was STALE (pre-reconcile); after rebuild it refuses, but the earlier number was an artifact |
| the suite is isolated | a test refreshed the registry INTO the repo's registry.json (the working tree was modified); fixed by removing the injection, and registry.json was restored |

---

## Four. Distortions fixed (owner item four)

- a failing assertion exited 0 - fixed: five files tie their result to sys.exit(0/1); verified
  install-reconciles returns exit 1 on a real fail.
- a precondition that did not take effect - fixed: "no registry" now explicitly clears the override;
  it previously ran with a registry the harness had supplied.
- a stale binary measured - fixed: the release binary was rebuilt (0 -> 1 occurrence of
  plugin_not_in_registry); no test may measure a binary older than the source.
- a state field used as the result - under repair: tests assert HTTP status + contract code + the
  on-disk fact, not a summary phrase.
- "all refused / empty events / empty result" treated as success - removed: BLOCKED is a distinct
  outcome; a refusal with no usable registry is UNVERIFIED, not a pass.
- claimed concurrency/recovery/isolation without verifying it - those files are BLOCKED (they could
  not run), not passed.
- separate accounting - PASS / PARTIAL / BLOCKED / FAIL / CRASH are printed separately; no aggregate
  green.

---

## Five. Readiness for the next phase

Verified without the registry: the contract surface, the refusal gate (no published registry ->
nothing installs), the registry/refresh error codes, the release binary's fixed address and ignored
override, and the CRUD surfaces (skills, connections, harness-n
