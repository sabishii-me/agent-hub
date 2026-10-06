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

Full-suite composition (45 files): 12 PASS, 33 BLOCKED, 0 FAIL, 0 CRASH, plus the PARTIAL files
(some checks pass, some blocked). Re-run after this revision to confirm.

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
| 100 loopback connections | real hub | 100 idle conns | no timeout |
| skills + connections CRUD | real hub | /v1/skills*, /v1/connections* | 7/7, 8/8 |

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

## Four. Distortions fixed or retracted this round
- tcpfwd.die(): closed only the listener; now closes established connections with SO_LINGER=0 (a
  real mid-turn RST). Previously mislabelled "cut the route".
- denying-an-approval: `target is None` passed trivially; now a missing cwd is a FAILURE and the
  file's absence is asserted against a real path.
- review-remains: proved only "no approval", not that the tool ran; now asserts a tool call is in
  the transcript.
- session-crud compact: `!= 501` let a 500 pass; now requires 2xx.
- concurrent-mixed-install-remove: an EMPTY event set satisfied `ids <= {..}`; now requires
  `ids == {pi, deepseek}` (both seen).
- plan-mode: a bool field is not execution; now ALSO asserts a write did NOT happen under plan.
- model identity: appliedModel is a claim; the proof remains the cross-turn recall (4242), noted.
- many-sessions-cancel: already refuses to assert "running at once" (written reason) — kept.

## Five. Readiness
Verified without the registry: the contract surface, the refusal gate, the refresh error surface,
the release binary's fixed address and ignored override, and the CRUD surfaces. To unblock: publish
the 'registry' release, then fix the host provider. Then the BLOCKED files can run and the
authorized-install + full-chain path can be VERIFIED for the first time — against the real registry,
not a fabricated one. Publication does NOT bless any earlier green.
