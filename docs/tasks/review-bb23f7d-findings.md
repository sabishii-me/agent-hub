# Review RECONSTRUCTION-ALIGNMENT-bb23f7d — findings and where each is fixed

The reviewer (TASK-048) fixed Hub `bb23f7d4b49d887dcbecd9eecfb2754826ad8998`. Verdict:
direction aligned, delivery NOT complete; do not open the architecture, keep developing the
main task. The findings below are the driving list. "Accepted" = the complaint is verified
true against the code, not argued. This file is the ledger; each fix records its commit.

## A1 — P1: skills fell back to the EXCLUDED hub-authored store; resources ignore the session
ACCEPTED. `crates/skills/src/routes.rs` calls `Skills::list/delete/read_file/write_file` over
`<DATA_DIR>/skills` — the model C2 (115cedf) stopped and ROUTES-REVIEW:131-142 says is NOT the
target ("the source becomes the plugin, not a hub-authored store"; PUT/DELETE "conflict with
the security rule"). The session resource routes take the sid but never use it
(`_id`), so any sid sees the same global directory. The v1 contract STILL describes the old
directory model (never corrected) — which explains but does not authorise the fallback.
FIX (order): (1) correct the owning contract to the plugin-sourced, workspace+session-layered
model (remove the conflict); (2) wire the plugin source + effective set + real loader/reload;
(3) make resources session-scoped. Do NOT restore old mutators to fill routes.

## A2 — P1: resource symlink/junction refusal checks only the LAST component
ACCEPTED. `read_resource` does a lexical `resolve()` then `symlink_metadata` on the FINAL
path only; an intermediate symlink (`skills/S/link/file.txt`, `link` a dir symlink) is
followed. `read_file`/`write_file` have no symlink check at all (PUT can write through a
link). Fix at the resource boundary: validate EVERY component and the opened object, cover
Windows junctions and replace-races, and bound the ACTUAL bytes read (not only pre-read
metadata). Do not downgrade this to "security acceptance not run".

## A3 — P1: auth result is not bound to the provider incarnation; cancel can still store+approve
ACCEPTED. The background task captures only the provider id; `finish_auth` re-reads the row
by id with no incarnation/revision/type check, so delete+recreate of the same id lets an old
result write the NEW record. And `auth.rs` checks cancel before the token HTTP; if
`auth_cancel` sets the DB cancelled while the request is in flight, a later 200 still calls
`finish_auth`, and `auth_op_finish` is an UNCONDITIONAL update that overwrites `cancelled`
with `approved` (contract: terminal states are immutable). Fix: one serialized, recoverable
boundary for op-terminal decision + provider incarnation + secret commit.

## A4 — P1: the auth commit writes the endpoint BEFORE the journal
ACCEPTED. `finish_auth` writes url/api to the row, THEN `begin_op`, THEN the secret, and does
not bump the revision. A crash after the row write leaves a NEW url with the OLD secret and no
pending journal to block grant/refresh. Also `recover_one` treats "config written + secret
exists" as landed, which can keep an old secret for a new endpoint. Fix: reuse the
record-intent-then-commit path; do not make auth an exception to the two-store rule.

## A5 — P2: auth id repeats across restarts; the "generic" device-code is one vendor's shape
ACCEPTED. `seq` restarts at 0 each boot and id=`{provider}-auth-{n}` is a persisted PRIMARY
KEY -> the same id collides after a restart (a terminal row still blocks a new INSERT). Also
`DeviceCodeSpec::from_descriptor` ignores `configuration.auth.method`; every device-code uses
one JSON body and one success parse (`/api_key/secret`, `/endpoints/openai_base_url`,
forced `openai-completions`); ADR-0012 says missing `expires_in` must FAIL, but the code
guesses 900/5. Fix: a cross-restart-unique id, and a NAMED hub dialect boundary for a
vendor-specific flow (not vendor behaviour hidden inside the generic path).

## A6 — P1: the minimum-host refusal is overridable by enable + persisted status; provider types bypass it
ACCEPTED. `adapter/manager.rs`: the scan refusal is overwritten by the DB status
(`set_status` has no refused check); `plugins/routes.rs` enable calls it directly; session
admission checks only `status`, not `refused`. `providers/types.rs` reads the descriptor
without a minHubVersion/activation check, and `main.rs` uses that scan as the create/grant
authority. Fix: separate "user wants enabled" from "host-compatible activatable"; make every
activation edge (session, enable, provider types) consume ONE admission result.

## A7 — P2: approval/question is visible before its waiter registers; a fast /v1 answer is lost
ACCEPTED. `main.rs` raises then awaits (registering the waiter only in await); `humans`
resolve sends only if the waiter already exists -> a resolve between publish and register
loses the wakeup forever. Also `runtime.rs` waits the reverse handler before pumping the next
frame, so an undecided approval blocks later notifications. Fix: create-visible + register +
read-existing-result as one lossless coordination; do not rely on sleeps/retries.

## Status-doc deconfliction (does not replace the fixes)
CURRENT-STATE still names an old SHA and an "uncommitted auth/schema blocker" while its tail
says auth is done; system-alignment still says skills 501 / auth-executor TODO;
REAL-ACCEPTANCE contradicts itself on jouzu. Mark history vs current; fix the live claims.

## Acceptance-evidence limits the reviewer named (hold ourselves to these)
- `REAL-ACCEPTANCE.md` is an owner summary; the adapter SHAs listed coexist with LOCAL edits
  whose artifact SHAs are not recorded; the runner copies the local plugin dir, so it does
  not prove the listed committed objects or the real install/prepare/upgrade chain.
- shisa auth was OUT-OF-BAND; it does not accept the hub auth executor. `shisa-auth-flow.mjs`
  is source, not proof of a successful this-round run; its PASS (has assistant text) does not
  prove the full lifecycle or model identity.
- 39 workspace tests are not 39 product capabilities; one `pong` does not cover approval
  allow/deny, preset switching, delete durability, the skills loader/effective-set/reload,
  recovery, min-host refusal, >=100 concurrency or the two-platform Gate A.
