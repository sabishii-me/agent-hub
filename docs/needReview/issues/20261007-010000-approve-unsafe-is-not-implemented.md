# 20261007-010000 — `approve-unsafe` (force-install a source that does not reconcile) is NOT implemented

## Status
OPEN — the settled policy has no code and no contract field.

## The settled policy (owner, 2026-10-07)
A direct address is NOT forbidden. The hub RECONCILES the source against the official registry:
- a source the registry lists (artifact `url` AND `sha256`) -> authorized;
- anything else -> REFUSED by default (`plugin_not_in_registry`, 403);
- **the user MAY force the install in a known-unsafe state**, by giving an explicit
  `approve-unsafe` (the international "explicit approval of an unsafe operation" convention, in the
  spirit of `curl --insecure` / a "yolo" flag).

## Observed (real hub, fixture registry, 2026-10-07)
`POST /v1/plugins` with a source NOT in the registry -> `403 plugin_not_in_registry`; there is NO
request field or header that overrides it. `crates/plugins/src/service.rs::authorized` returns
`Err(NotInRegistry)` for an unlisted artifact and (categorically) for a git source, and nothing in
`crates/plugins/src/routes.rs` reads an approval.
Measured in `tests/plugins/install-from-the-registry-fixture.py`: 6/6 — listed->202 `ready`;
unlisted->403; git->403 (with and without a registry). NO case exercises a forced install.

## Why it matters
The policy has three parts (reconcile / default-refuse / user-force). Two exist; the third does
not. So a user who KNOWS a plugin is not in the registry cannot install it at all - the escape
hatch the policy names is unreachable. (The inverse hole - an UNFORCED unlisted install - is closed
by 20261005-130000.)

## What must be decided BEFORE it is written (contract change)
1. WHERE the approval lives: an HTTP header (`Approve-Unsafe: true`) or a body field
   (`{source:{...}, approveUnsafe:true}`). Not chosen.
2. Its exact name/spelling (the settled word is "approve-unsafe").
3. The semantics: does forcing SKIP reconciliation only, or also the download/digest checks?
   (Proposed: skip RECONCILIATION only; still download and verify sha256+size against the DECLARED
   values, and record that the install was forced, so the result is auditable.)
4. Whether a forced install is recorded (a flag on the plugin row / an event) so "this plugin was
   installed unsafe" is visible afterwards.

## Fix (after the decisions)
- `contract/v1.json`: add the approval to the `POST /v1/plugins` source (or the operation);
  `contract/errors.json` unchanged unless a new code is chosen.
- `crates/plugins`: when approved, authorize the source WITHOUT a registry match; still verify the
  bytes; record the forced install.
- A test: the SAME unlisted source that is 403 by default becomes installed when forced.

## NOT yet done
No contract written; no code; no test. The install chain is otherwise measured (listed/git/
unlisted refusals).
