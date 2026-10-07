# 20261003-151000 — T0 audit: where the implementation invented beyond the design

Recorded: 2026-10-03. Severity: **T0**. This is the EVIDENCE for deciding whether to review
the whole project and rebuild, or correct in place. Read-only: no code is changed here.

The owner's finding: the built code follows a DIFFERENT model than the design, by inventing
values and shapes where the documents were silent, and then treating the inventions as fact.
This file lists the deviations found so far, each with the authority it violates.

## D1 — cancel is a blocking wait, not a fire-and-report command (see 20261003-150000)
- Authority: `v1.json` cancel = "idempotent; terminal state returns current state without
  error"; `adapter-v1.json:387` "session/abort acknowledges only that the harness was ASKED to
  stop; the turn's own turn_end proves it stopped"; ADR-0009 rule 2 "returns immediately with
  a handle ... a client never holds a connection waiting".
- Built: the cancel request awaits the abort reply (`control_request_timeout`, up to 60 s).
- Shape wrong, not a value.

## D2 — an invented control-request deadline under 9 calls
- Value: env `AGENT_HUB_CONTROL_TIMEOUT_SECS`, default 60 (`sessions/runtime.rs`).
- Authority: NONE. `adapter-v1.json` defines a deadline ONLY for `session/prompt` ("No default
  deadline") and `question` ("cancel the waiting question"). It never defines a deadline for
  `credentials/grant`, `config/set` or `session/abort`.
- Built into: grant, config/set (x4), abort-reply wait, repair re-abort, mid-session grant.
- The env NAME and the 60 are both invented.

## D3 — an invented default turn deadline (1800 s)
- Authority: `adapter-v1.json:259` `session/prompt` = "No default deadline".
- Built: `timeout_unconfirmed_cancels(30, 1800)` stops a `running` turn after 30 min with NO
  cancel. A long legitimate task is aborted by an unauthorised default.

## D4 — the cancel-confirm window is invented (30 s)
- Authority: the intent is the SHORTEST possible (owner: cancel ~0 s). 30 s is invented.
- Authority for the MECHANISM (a core-side timeout) is `adapter-v1.json:387`; the VALUE is
  invented.

## D5 — the adapter environment surface is mostly UNDOCUMENTED
- The adapter contract DECLARES **2** variables: `AGENT_HUB_INSTALLED_SKILLS_DIR`,
  `AGENT_HUB_RUNTIME_COMMAND`.
- The REAL adapters READ **8**: the 2 above + `ADDITIONAL_DIRS`, `CWD`, `HARNESS_DIR`,
  `INSTALLED_EXTENSIONS_DIR`, `PRESETS_DIR` (+ `DIALOG_TRACE`, a debug one).
- The CODE SENDS **13** (adds `ADDR`, `CONTRACT_DIR`, `DATA_DIR`, `PLUGINS_DIR`, `SECRET_KEY`,
  `SESSION_ID`).
- So the actual hub<->adapter seam is 8 variables where the contract names 2. The integration
  WORKS (the real adapters read them), but the CONTRACT was never updated: the interface is
  whatever the code happened to grow. This is the same disease as D2/D3 (invent, then treat
  as fact), on the seam that matters most.

## D6 — a provider auth schema was invented, then corrected only after being caught
- `defs.authStep` / `defs.authOperation` + ADR-0012 were written by the owner's instruction
  after the code had already invented `kind` (the doc says `method`) and step fields the
  contract did not define. The contract now defines them, but they were invented first.
- (Recorded so the "invent then backfill the doc" pattern is visible; the schema is now
  explicit.)

## D7 — skills was implemented on the excluded model
- See 20261003-123000 and the bb23f7d review A1. `PUT/DELETE /v1/skills` + a `<DATA_DIR>/skills`
  store is the model C2 stopped; `ROUTES-REVIEW` says the SOURCE becomes a plugin. Built
  anyway, because the (stale) contract still described it.

## The pattern (the real T0)

Every deviation is the same move: **the document did not say, so a value/shape was chosen and
then treated as the design.** It appears on the timeout model (D2-D4), the adapter seam (D5),
the auth schema (D6) and skills (D7). Correcting individual values (30 -> 1, drop 1800) leaves
the MOVE in place, which is why the owner says a patch is not the answer.

## What a decision needs

Two options for the owner:
- **(A) Review the whole project against the documents**, and correct each seam so the
  implementation FOLLOWS the contract (no invented env vars, no invented deadlines, the
  adapter seam documented, skills on the decided model). This is large: it touches the
  timeout/cancel model, the adapter env contract, the skills model, and every "value with no
  authority".
- **(B) Rebuild** the affected parts from the contract outward, treating the current code as a
  reference, not a base.

This file does not choose. It is the evidence base for that choice.
