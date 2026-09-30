# The lifecycle chain: one coherent slice

A bounded, whole-chain analysis of the ONE vertical chain the hub declares usable. It
exists because the last rounds fixed lines instead of the chain: a journal gained an
expected revision but not which version it records; a DB claim was mistaken for delivery
atomicity; a timeout took a lock a cancel could hold forever; a staging rename was added
while the caller still deleted the shared dir first; typed errors were kept but not
traced to the wire. This is that picture; the implementation follows it.

## The chain, and what each step IS

```
provider relationship + version
  -> secret ownership + a cross-store operation
    -> resolver reads a consistent config+credential
      -> session start / reopen / switch
        -> grant / config / actual applied confirmation
          -> turn admission, prompt delivery, cancel, terminal
            -> fault, restart, resource reclamation
              -> the official GET / event / error reflect ONE fact
```

### Identities: what an object IS and who owns it

- Provider: `id` is its name; `incarnation` a lifecycle instance (delete+recreate is a
  NEW incarnation); `revision` counts material changes. A credential is owned by a
  `(id, incarnation, revision)` triple, not by an id alone.
- Connection: same shape, separate domain.
- Session: `id` stable; `status` (`starting|active|needs-repair|readonly|starting_failed`)
  is the hub's ONE verdict; a native `ref` is the harness's handle.
- Turn: durable identity `(session_id, idempotency_key)`; `state`
  `admitted|running|cancelling|ended`; `ended` is the terminal.
- Adapter process: one per running session; `RequestHandle.is_alive()` is the liveness truth.

### Steps: admission vs side effect vs confirmation vs durable commit

| Step | Admission | Side effect | Confirmation | Durable commit |
|---|---|---|---|---|
| create session | reserve command+row (tx) | spawn adapter (start+grant+config) | adapter `applied` | row `active` only after confirmation |
| turn | `admit_turn` (tx: identity+busy) | `session/prompt` send | adapter `turn_end` | `end_turn` (guarded, once) |
| cancel | durable `cancelling` | `session/abort` send | adapter `turn_end` | `end_turn` by run_turn / timeout |
| provider write | journal op (tx) | keychain set/delete | (none) | row save (incarnation-guarded) |
| provider grant | resolver (provider lock) | `credentials/grant`+`config/set` | adapter `applied` | row `applied_*` |
| placement | (none) | build+swap the shared extensions tree | (none) | the tree IS the artifact |

### Concurrency and failure around awaits / cross-store boundaries

- P1 provider: `patch` writes the secret in place on the same key BEFORE the row save.
  A URL+token patch where the secret lands but the row save fails leaves the row at the
  OLD incarnation/revision and the key at the NEW token. Recovery cannot currently tell
  "row unchanged, key is the operation's" from "row changed, key is the operation's".
- P2 placement: `harness_env` holds the per-harness lock only for its body; the adapter
  that READS the tree spawns AFTER the lock is released. A second start can swap the
  tree between the release and the read.
- P3 cancel lock: `cancel_turn` holds the session lock across the `session/abort`
  request, which waits for the adapter; the timeout sweep needs the same lock. A
  non-answering adapter deadlocks the rescue.
- P4 timeout target: the sweep selects rows then acts; it re-checks under the lock, but
  `stop(session_id)` stops the session's process, not "the process that ran turn A" -
  safe ONLY because the re-check found A still active; that must stay explicit.

### Locks: who holds what, across which wait, who else needs it

- session lock: held across accept/close/reopen/patch and now cancel. MUST NOT be held
  across an unbounded adapter wait (cancel violates this).
- provider lock: held across the read-modify-write and the resolver (catalog fetch held
  deliberately). Short.
- placement lock: sync; covers only the rebuild.

### Uncertain state: who stores it, who resolves it, what is never replayed

- Provider: the journal (`provider_ops`) stores an in-flight op with the intended row
  state; the boot sweep resolves it. Never re-run a secret side effect blindly.
- Session: `needs-repair` + the in-memory `quarantined` gate store "unknown config";
  resolved by `reopen` (re-grant + re-confirm).
- Turn: a non-terminal turn at boot is settled; a prompt is never replayed.

### The official view

- GET reads the row (the durable fact).
- An event is published ONLY after the durable write it describes.
- An error carries a CONTRACT code end to end (adapter `data.code` -> domain -> HTTP).

## Concrete defects found (fixed as ONE change)

1. **Migrate**: `provider_ops` gained `expected_incarnation`/`expected_revision` but
   `providers::migrate` adds neither for an existing DB (and the guard skips when only
   `provider_ops` exists). An upgraded DB cannot run the new recovery.
2. **Recovery intent**: `recover_one` compares the recorded intent against the row NOW.
   A patch that changed the URL but failed to save leaves the row at the OLD
   incarnation/revision - which MATCHES the recorded pre-write intent, so recovery
   attaches the NEW token to the OLD URL. The intent must distinguish "the row save
   landed" from "it did not": record the intended POST-write `(incarnation, revision)`,
   and confirm only when the row matches THAT.
3. **Placement reader**: the lock releases before the adapter reads. Publish the tree
   behind a STABLE pointer the adapter follows (a `<base>/extensions` junction to a
   versioned dir), swapped atomically, so a reader always sees a complete set - and keep
   the writer lock.
4. **Cancel lock**: do not hold the session lock across the abort request; send the
   abort after durably recording the intent + state, without the lock, so the timeout
   can run.
5. **Stop target**: the timeout stops/confirms the process it owns and keeps the turn
   held on an unconfirmed stop.
6. **Typed error end to end**: carry the adapter's code from the bus to the HTTP error
   body and into persisted `startError`/turn cause.

## Implementation order

(1) migrate, (2) recovery intent, in the provider/secret half; then (3) placement
publication; then (4)+(5) the turn half; then (6) the error path.

## Invariants (the acceptance)

- Credential ownership consistent: never pair one config version's endpoint with
  another's token; the journal distinguishes side-effect-done / row-committed /
  unconfirmed; recovery is not "row exists and key has a value".
- Config confirmed consistently: start/reopen/switch share one target-vs-applied
  judgement; an unknown config is never served on the old identity.
- Execution order consistent: admitted/claimed/sent/executing/cancel-requested/
  stop-confirmed are distinct; a DB write is not delivery atomicity; cancel can
  interrupt; no operation permanently holds a lock a rescue needs.
- Stop target accurate.
- Recovery trustworthy: a DB failure is not hidden by an event; a known terminal is not
  discarded then guessed; never silently replay an unknown side effect.
- Shared publication complete: writer mutex + staging + one rename do NOT by themselves
  prove a reader always sees a complete resource.
- Contract semantics end to end: no string formatting drops an error identity; a
  protocol/route/alias has owning-contract basis.

## Implemented (this pass)

1. **Migrate** (`crates/db/src/providers.rs`): a per-table additive `migrate` adds
   `providers.incarnation` AND `provider_ops.expected_incarnation/expected_revision`,
   each independently (a DB missing only one column still upgrades). Test:
   `an_old_provider_ops_gains_the_intent_columns`.
2. **Recovery intent**: the journal now records the intended POST-write
   `(incarnation, revision)`; `patch_session` merges fields and computes the final
   revision BEFORE `begin_op`. `recover_one` confirms the credential only when the row
   matches that state; otherwise it DROPS the credential. Test:
   `recovery_drops_a_credential_whose_row_save_did_not_land`.
3. **Placement publication**: `install_for_harness` publishes IN PLACE and ADDITIVELY -
   each selected extension is built in a staging dir and renamed over its own final
   name; de-selected ones are removed LAST. The caller no longer deletes the shared
   dir. A reader always sees a complete extension, never a half/absent tree. Verified
   live: two sessions on one harness both reach `active`; the tree stays complete.
4. **Cancel lock**: `cancel_turn` records the intent under the session lock, then
   RELEASES it before the abort request, so a non-answering adapter cannot hold the
   lock the timeout needs.
5. **Timeout target**: the sweep re-checks under the lock that the OLD turn is still
   active (never kills a newer turn), requires a CONFIRMED stop (a failed stop keeps
   the turn held), and marks the session `needs-repair` in memory AND durably.
6. **Typed error**: grant/config bus errors carry the adapter's `data.code`
   (`bus_error`), and `SessionError::from_start` maps a typed refusal to the contract
   code instead of `adapter_crash`.

Verified: zero warnings; `cargo test --workspace` all green (39 result sets); live
create->active, turn->terminal, cancel idempotent, GET reflects the row.

## Rehearsed on a real process (this pass)

A mock adapter (`E:/AI/ideas/_mock/plugins/mock`, NOT in the repo) obeys the protocol
but deliberately never answers `session/prompt`/`session/abort`. Rehearsal result:

- create -> `active`; prompt -> turn `running`; cancel -> the abort is not answered.
- The turn stays held; within one sweep tick it becomes `ended/interrupted` and the
  session becomes `needs-repair` (in memory AND in the DB), logged `timed_out=1`.
- Reopen -> `active` again.

Two defects found and fixed in the process:

7. **Reader treated any end-of-stream as "the adapter exited"** (`crates/adapter`): a
   transient EOF detached a LIVE adapter, so a delivered abort failed and the turn was
   left unsettled until the sweep. The reader now re-reads on EOF and only concludes the
   adapter is gone after a SUSTAINED EOF.
8. **The cancel could not be delivered** returned the UNDECLARED `abort_failed` (500).
   It now returns the declared `adapter_unreachable` (502, retryable) - the adapter is
   alive, just not answering; the sweep still rescues the turn.

Honest limit: the live rehearsal needed a real long-running turn, which the mock
provides; a REAL vendor turn still needs a credential authorization.

## Bounded control requests (this pass, live)

Rehearsed a boundary the earlier pass left open: a `config/set` whose **response was
lost** (the adapter may have applied it, the hub never learns the outcome).

- **Defect 9 (real)**: the mid-session `config/set` for a policy knob (`plan`/`review`)
  and for the provider/model switch had **no bound**. An adapter that never answered
  hung the PATCH handler **indefinitely** and left the session `active` on an UNKNOWN
  configuration - a direct ADR-0009 violation and a fail-closed violation.
- Fix: every control request (`credentials/grant`, `config/set`) now has a bounded wait
  (`runtime::control_request_timeout`, default 60s, `AGENT_HUB_CONTROL_TIMEOUT_SECS`
  overridable). On timeout the outcome is UNKNOWN, so the session is **quarantined**
  (`needs-repair`), never assumed failed and never left running.

Live evidence (bound = 3s):

- PATCH `plan` against a mute adapter -> returns in **3.4s** with `502 adapter_crash`,
  the session becomes `needs-repair`, and a new turn is refused (was: hang forever,
  session stays `active`).
- create against a mute adapter -> `starting` -> `starting_failed`
  ("the adapter did not answer config/set in time") at ~3s (was: hang).

## REVIEW-f1f6801 fixes (this pass)

Three P1s from the reviewer, fixed at the root:

1. **Legacy journal semantics (F1)**: an entry written at base `b0459a5` stored the
   PRE-write revision in `expected_revision`; the new recovery read it as POST-write.
   `provider_ops` now carries `intent_version` (2 = post-write intent). A LEGACY entry
   (`intent_version` NULL) is ISOLATED: recovery never blesses it and never deletes a
   credential the row already references; only a TRULY orphaned credential (no row
   references it) is removed. Tests: `a_legacy_journal_entry_neither_blesses_nor_destroys`,
   `a_legacy_entry_removes_a_truly_orphaned_credential`.
2. **Turn delivery ordering (F4)**: a per-session DELIVERY lock now serializes a turn's
   `session/prompt` frame write against a cancel's `session/abort` frame write, and the
   row is re-checked under it. The prompt frame is wholly written before any abort, or the
   cancel wins the row and the prompt never sends. The lock is held ONLY across the frame
   write; the long wait for an answer is outside it. The abort re-checks the turn under
   the lock, so it can never hit a later turn. A `claim_running` DB error fails the turn
   (never swallowed as "someone won"). Live: 5 rapid prompt+cancel pairs all show
   `session/prompt` before `session/abort`.
3. **Repair unbounded wait (new)**: repair's re-abort is now a BOUNDED, best-effort send
   (never an unbounded wait under the session lock); the STOP is the real replacement.
   Non-native modes (`truncate`/`tombstone`) are refused honestly (the hub does not
   perform hub-side history surgery).

Also closed: `applied.connectionId` now MUST be the injected `hub-<id>` (a bare id is not
the resolved native route); `SessionError::from_start` is wired (typed adapter errors keep
their identity through reopen/run_start) and the adapter's `abort-failed` maps to
`adapter_unreachable`; quarantine/fail_start/timeout no longer PUBLISH a state change whose
DB write failed; the `thinkingLevel` and policy-knob `config/set` branches are bounded and
fail closed (quarantine) on an unknown/unconfirmed outcome.

## REVIEW-71ef0d6 fixes (this pass)

Two remaining P1s and the reported P2s:

- **F1 legacy isolation made REAL**: the previous pass returned `Ok` for a legacy
  entry whose row referenced the credential, so the sweep CLEARED the pending and the
  resolver granted the possibly-mismatched credential. Now such an entry returns an
  `Unresolved` error: the journal entry STAYS, `has_pending_op` stays true, and
  `resolve_grant` keeps refusing until it is explicitly resolved. An orphaned
  credential (no row references it) is still cleaned up. The legacy test now asserts
  the barrier is KEPT and the resolver REFUSES, not just that the token survives.
- **F4 delivery bound to process GENERATION**: `Sessions` keeps a per-session
  generation, bumped on every start AND stop under the same `requests` lock that holds
  the handle map. A cancel captures the generation and uses `send_if_generation`, so an
  abort captured against the old process is REFUSED if the process was replaced
  (stop/reopen/repair) in between - an old abort can never reach a new process.
- **F4 send-error protection restored**: `runtime.send` now distinguishes
  `NotDelivered` (no live process / stale generation / dead pipe) from `Unknown` (a
  write failure with the process ALIVE). The prompt path settles `failed` ONLY on
  `NotDelivered`; an `Unknown` leaves the turn `running` for the timeout sweep, exactly
  the unknown-result protection the previous pass had bypassed.
- **F4 cancel reply bounded**: the abort-reply wait is bounded by the control timeout;
  on timeout the turn stays held (`cancelling`) and the sweep resolves it.
- **F5**: `from_start` is now used by the PATCH grant/config paths (a typed refusal
  keeps its contract code) as well as reopen/fork.
- **N4**: the stale "junction/stable pointer" comment is removed; the honest guarantee
  is stated where the replacement happens, and `read_dir`/entry/rollback failures are
  now REPORTED instead of ignored.

Lifecycle note: `recover_pending` at boot keeps an unresolved legacy entry forever
until an explicit resolution - that is the intended barrier, not a stuck sweep.

## S0: no-legacy cleanup (this pass)

Per the owner's correction, the development-era legacy/compat layer is REMOVED:

- Deleted `providers::migrate`, `sessions::migrate`, `migrate_plugins` (ALTER-based
  upgrades) and their tests.
- Deleted the legacy journal interpretation branch in `recover_one` and the
  `begin_provider_op_legacy`/`begin_op_legacy` helpers, the `testing` feature, and the
  legacy recovery tests.
- ADDED version recognition BEFORE any schema write / journal sweep / secret side
  effect: `PRAGMA user_version` = `Db::SCHEMA_VERSION` (=1). A fresh, empty database is
  created at the current schema and stamped; a database already at the current version
  reopens; a database with tables and NO version, or a different version, is REFUSED
  with `UnsupportedFormat` (never migrated or overwritten). Tests:
  `schema_version_tests` (fresh stamp, reopen, unversioned refused + untouched, foreign
  version refused).

Kept (NOT legacy): the current schema, `provider_ops` journal with the intended
row-state, the per-instance secret namespace, and current-format crash recovery.

## S1: every provider mutator/consumer honors the unresolved barrier (this pass)

The pending guard was only on the resolver and the credential writes, so
`refresh` (which SENDS the token as a bearer), a token-less PATCH and a model
selection could bypass it. Now a single `admit_no_pending(id)` check runs at the top
of `patch`, `set_selection` and `refresh`, under the provider lock: every entry that
changes the row's config/revision or consumes the credential refuses while a
transition is unresolved (`ProviderError::Unresolved` -> `revision_conflict`).

Recovery also no longer reads an UNRELATED update as an op's commit: the journal now
records `expected_config`, a stable digest of the row's `url`/`api`/`declarations`, and
`intent_matches` requires incarnation AND revision AND that digest. An unrelated change
that happens to reach the same revision (a URL patch, a selection) has a different
digest and is refused.

Tests: `every_mutator_refuses_while_a_transition_is_unresolved`,
`recovery_rejects_an_unrelated_update_at_the_same_revision`.

## S2: the abort is bound to the turn's DISPATCHED process (this pass)

The previous pass captured the CURRENT generation at send time, so a replacement
between the turn check and the send could still receive the old abort. Now:

- `turns.process_gen` records the generation the turn dispatched on, written by
  `claim_running(id, process_generation)` in the SAME SQL as the state change.
- `cancel_turn` reads the TURN'S recorded generation and uses `send_if_generation`;
  if the process was replaced since, the send is REFUSED (never re-bound to the new
  process). A turn cancelled before dispatch has no generation and is not aborted.
- `start` publishes the request handle AND bumps the generation in ONE critical
  section under the `requests` lock, so `send_if_generation` can never observe a new
  handle with an old generation.

Live: a dispatched turn records `process_gen=1`; cancel binds to it; after a restart
the process is gone and the turn is reconciled `interrupted` (no stale abort).
