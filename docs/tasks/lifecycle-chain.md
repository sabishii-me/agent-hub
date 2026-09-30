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
