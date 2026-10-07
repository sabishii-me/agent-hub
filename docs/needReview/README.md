# needReview — everything below is PENDING REVIEW

The hub's documentation is inconsistent: decisions recorded at different times contradict each
other, and the contract/architecture/status docs have drifted. This directory holds **every**
document that must be reviewed before it is trusted again. Nothing here is authoritative until it
passes review.

## What this review must cover (three passes)

### Pass 1 — every document
For EACH file under this directory: what it claims, what it is based on, whether that basis still
holds, and whether it contradicts another document. The reviewer records the contradiction, not a
summary.

### Pass 2 — every endpoint
`contract/v1.json` is the endpoint source of truth; `contract/openapi.json` is its generated
projection. Review all endpoints against:
- the architecture (`ARCHITECTURE.md`, esp. §11/§13/§24) and the ADRs;
- what the hub actually serves (the mounted routes vs `§24`);
- the route docs (`tasks/metadata-routes.md`, `plugin-lifecycle.md`, `connections.md`, …).
Record: missing endpoint, undocumented behaviour, a status/code that disagrees, a route the code
serves but the contract does not declare (and the reverse).

### Pass 3 — the architecture and the design decisions
Review `ARCHITECTURE.md` end to end against the ADRs and against the code. Record every place two
design statements conflict, and every place the architecture claims something the code does not do
(or the reverse).

## The known contradictions (to CONFIRM during the review, not assumed true)
These are candidates, listed so the review checks them — each must be CONFIRMED or DISMISSED with
evidence, not repeated on trust:
- the direct-address policy: `review/20261005-contract-proposal-install-by-registry-id.md` says
  "do NOT forbid a direct url" and "default refuse, a deployment may later allow unlisted"; the
  settled direction is "not forbidden, must reconcile, default refuse, and the USER may force an
  install in a known-unsafe state (`approve-unsafe`)". Confirm which documents still contradict.
- the registry model: `registry.json` is BOTH a checked-in local file AND the published release
  asset; the docs describe it both as git-tracked and as a build/publish artifact.
- `ARCHITECTURE.md` §18/§24 vs `system-alignment.md` vs `CURRENT-STATE.md`: three status tables
  from different times.
- `plugin-lifecycle.md` says the catalog/refresh/icon are "not built"; §18 says they are real.

## Rules for this review
- No document is edited by moving it here; moving is only to mark it PENDING.
- A contradiction is recorded with BOTH sides and their file:line, then it is resolved by an owner
  decision — not by picking one silently.
- Nothing in `contract/` changes until Pass 2 is reviewed.
