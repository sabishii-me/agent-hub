# Convergence report — what the Python suite genuinely verifies (registry unpublished)

Written 2026-10-05, revised after review. Owner order: do not fake-pass, do not fabricate a
registry, do not let the test stand in for the product. The one missing precondition behind every
BLOCKED row: the official registry release is UNPUBLISHED.

## This revision fixes the review's blockers
1. Fabricated-registry tests removed from the acceptance path: registry-refresh.py no longer has a
   fake 'success' (loopback server whose id becomes the catalog); it tests ONLY the error surface.
   the-release-hub-ignores-the-registry-override.py now decides by the SERVER HIT COUNT, not by an
   empty catalog list.
2. run.py accounting hardened: a FAIL or CRASH can no longer be hidden by a BLOCKED line or an
   exit-0; 0/0 is not a pass; all-BLOCKED / no-PASS runs exit non-zero.
3. install_plugins is CONDITIONAL on publication with a REAL path (it reads the HUB'S OWN catalog,
   not a file); it is not permanently blocked.
4. Falsified test designs corrected or explicitly retracted (see Four).
5. Refusals with no published registry are no longer called "sha256 verification".

## Method
`python tests/run.py` classifies each file by EXIT CODE and its summary into SEPARATE buckets —
PASS / PARTIAL / BLOCKED / FAIL / CRASH / EMPTY. There is no "N/N passed".

Full-suite result for commit 0fc8568 (raw output: docs/tasks/20261005-run-output-raw.txt):
```
files: 45
  PASS    : 7      (independent, no registry)
  PARTIAL : 4      (some checks pass, some BLOCKED)
  BLOCKED : 33     (a real precondition is missing -> UNVERIFIED)
  FAIL    : 1      (the >=100-concurrent-connections claim: measured peak 4-6, NOT 100)
  CRASH   : 0
```
7 + 4 + 33 + 1 = 45. Buckets are DISJOINT. The FAIL is real: one-hundred-connections MEASURES the
peak simultaneous connections and the hub did NOT hold 100 at once; the old PASS was about
completion, not concurrency, and is retracted.

PASS files: contract/status-surface, contract/served-surface, plugins/a-caller-cannot-install-from-an-arbitrary-url,
plugins/the-release-hub-ignores-the-registry-override, harnesses/an-uninstalled-harness-cannot-be-used,
skills/crud, connections/crud.

PARTIAL files: plugins/install-reconciles, plugins/install-refusals, plugins/registry-refresh,
plugins/the-release-hub-error-surface.

---

## One. What is genuinely verified now (real evidence)

| capability | precondition | /v1 trigger | actual result |
|---|---|---|---|
| served surface equals contract | real hub + contract/ | boot self-check | every declared route mounted; no parameterless GET is 501 |
| status + models answer from contract | real hub, no plugins | GET /v1/status, /v1/models | shapes match |
| a caller CANNOT install from an arbitrary url | real hub, NO registry | POST /v1/plugins {url|artifact} | 403 plugin_not_in_registry; no dir lands |
| release hub ignores the registry override | RELEASE binary + canary server | refresh then catalog | debug hits the canary server (1), RELEASE hits it ZERO times |
| registry refresh ERROR surface | real hub | refresh at dead/500/non-JSON/no-array | 502 registry_unavailable naming the case |
| release build error codes | RELEASE binary | bad body / missing / unlisted source | 400 validation_failed; 404 not_found; 403 |
| an uninstalled harness cannot be used | real hub, no plugins | GET /v1/harnesses, POST /v1/sessions | 404 harness_not_found |
| an uninstalled harness cannot be used | real hub, no plugins | GET /v1/harnesses, POST /v1/sessions | 404 harness_not_found |
| skills + connections CRUD | real hub | /v1/skills*, /v1/connections* | 9/9, 10/10 (PATCH read back; delete proven by an authoritative read) |
| SSE stream opens, does not block a GET | real hub | GET /v1/events held; GET /v1/harnesses | SSE 200 + first byte; GET < 5s |
| >=100 connections AT ONCE | real hub | 120 requests | **FAIL**: measured peak 4-6; the claim is retracted |

## Two. UNVERIFIED / BLOCKED — the official registry is unpublished

Missing precondition: the 'registry' release of sabishii-me/agent-hub must be published
(currently HTTP 404). BLOCKED, not passed: plugin discovery; registry refresh from the real
address; AUTHORIZED install; the whole chain from an empty hub; everything needing an installed
plugin (session CRUD, all interrupt/*, approvals/*, concurrency/*, provider/*, presets/*, tools/*,
model/*, harnesses/discovery).

An "installation was refused" is NOT reported as "the safety mechanism passed": with no published
registry there is no reachable authorized install to contrast against, so a refusal is consistent
with BOTH a correct gate and a hub with no registry. UNVERIFIED.

### Second, independent blocker
The host's real provider (~/.pi/agent/models.json -> 192.168.31.29:8990) answers 400 to its own
/models with the host token (verified OUTSIDE the hub — docs/issues/20261005-150000). The
session/turn chain is blocked by the provider too.

### Already-installed-environment note
No test claims a from-empty-hub full chain. $PI_PLUGIN_DIR is a REAL SOURCE used only for refusal
tests and kind_of() labelling; it is not an installed hub plugin and no session runs on it. This run
did NOT verify plugin discovery, install, or a cold-start chain.

## Three. Retracted over-claims
- any "all green" / "N/N passed": the runner counted exit codes; five files lost a FAIL into exit 0.
- "whole-chain 13/13 proves the system": it installed from a caller url with the test as registry.
- "registry-refresh 17/17 proves the registry works": it fabricated a registry and read error codes.
- "installed by registry id": no id path exists; the test supplied url+sha256.
- "refusals are 403, so install is safe": with no registry that proves only 'no registry -> refuse'.
- "release hub installs nothing (secure)": measured on a STALE binary.
- "the suite is isolated": a test wrote the registry into the repo's registry.json; fixed, restored.

## Four. Distortions fixed or retracted

### Round two (review of a592ec3)
- release-ignores-override: run() did not check the subprocess returncode, so a crashed/missing
  binary (0 hits) read as 'correctly ignored'. Now both hubs must START and answer the refresh
  before canary hits are judged. 5/5.
- denying-an-approval: a NameError (`cwd` undefined) crashed the test on any run once it got past
  the registry block; restored `cwd = s.get("cwd")`, and now checks the deny POST result and the
  wait_turn result (a failed deny / stuck turn no longer passes on 'file absent').
- review-remains: the tool evidence was the WHOLE session, so an earlier turn's tool call satisfied
  it. Now the message count is frozen before the OFF turn and only the OFF turn's own messages are
  read, plus the read tool's RESULT must reach the answer.
- plan-mode: 'no file' could mean provider-error/hang/broken tool. Now the plan write turn must
  reach a terminal, AND a second write with plan OFF must SUCCEED - proving plan, not a broken
  tool, was the difference.
- model identity: cross-turn recall is conversation CONTINUITY, not model IDENTITY. Split: the
  fields are 'reports a claim'; the actual identity is BLOCKED (no independent evidence).
- install_plugins: a refresh failure with the registry PUBLISHED is now a RuntimeError (product
  failure); only an unpublished 404 is BLOCKED. start() clears an inherited registry override.
- many-sessions-cancel: the FACT line claimed 'N turns run at once', which is NOT verified. Claim
  corrected to 'N admitted turns, batch-cancelled, all settle'; the concurrency question is an
  explicit BLOCKED; the cancels are issued in a loop and this is stated.
- report numbers corrected to the real run; raw output archived at docs/tasks/20261005-run-output-raw.txt.

### Round one (earlier)
- tcpfwd.die(): closed only the listener; now closes established connections with SO_LINGER=0 (a
  real mid-turn RST). (Static: not yet re-measured for RST.)
- session-crud compact: `!= 501` let a 500 pass; now requires 2xx.
- concurrent-mixed-install-remove: an EMPTY event set satisfied `ids <= {..}`; now requires
  `ids == {pi, deepseek}` (both seen).
- nine files discarded a FAIL into exit 0; runner classification hardened (FAIL/CRASH win over
  BLOCKED; exit-0 + failed>0 is FAIL; 0/0 is EMPTY; all-BLOCKED exits non-zero).

## Five. Readiness
Verified without the registry: the contract surface, the refusal gate, the refresh error surface,
the release binary's fixed address and ignored override, and the CRUD surfaces. To unblock: publish
the 'registry' release, then fix the host provider. Then the BLOCKED files can run and the
authorized-install + full-chain path can be VERIFIED for the first time — against the real registry,
not a fabricated one. Publication does NOT bless any earlier green.
