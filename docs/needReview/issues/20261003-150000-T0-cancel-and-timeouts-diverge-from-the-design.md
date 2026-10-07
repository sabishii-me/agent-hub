# 20261003-150000 — T0: cancel and the timeouts are built against the wrong model

Recorded: 2026-10-03. Severity: **T0** — this is not a bug in a value; the implementation is
built on a different model than the design, so "our project" and "the thing that was built"
diverge. Owner: **HUB**. Status: recorded, NOT fixed. Nothing below is changed yet.

## The design (the authority, quoted)

1. **`POST /v1/sessions/{id}/cancel`** — contract `v1.json`:
   > "idempotent; terminal state returns current state without error"

   Cancel is an IDEMPOTENT REQUEST THAT RETURNS. It does not say "wait".

2. **What an abort means** — contract `adapter-v1.json:387`:
   > "`session/abort` acknowledges only that the harness was ASKED to stop; the turn's own
   > `turn_end` proves it stopped. An abort that could not be delivered -> error
   > `data.code='abort-failed'`; an abort delivered but never confirmed is the core's cancel
   > timeout to report, not the adapter's to hide"

   So: send the abort; the ACK means "asked", not "stopped"; the PROOF is the harness's own
   `turn_end` (an event); a NON-DELIVERY is the only error; "delivered but unconfirmed" is
   reported by the **core's cancel timeout** - a separate, later judgement, NOT a wait inside
   the cancel request.

3. **Long work is a command, not a call** — ADR-0009 rule 2:
   > "An operation that can take more than a moment (... a turn) returns immediately with a
   > handle, runs as an independent background task, and reports its state and progress on
   > the event stream. A client never holds a connection waiting for the work."

4. **`session/prompt` has no default deadline** — contract `adapter-v1.json:259`:
   > "timeoutBehaviour": "No default deadline. Explicit operator timeout is fail-closed; ..."

## What was built instead (the divergence)

- **Cancel waits.** `crates/sessions/src/service.rs` (cancel path) sends `session/abort` and
  then AWAITS the reply with `tokio::time::timeout(control_request_timeout(), abort_rx)` —
  the request holds up to **60 s** waiting for a reply. The design says cancel returns; the
  ACK is not the stop, and the stop is proven by `turn_end`, not by this reply.
- **An invented "control request timeout", 60 s.** `crates/sessions/src/runtime.rs`
  `control_request_timeout()` = env `AGENT_HUB_CONTROL_TIMEOUT_SECS` or **60**. It is applied
  to 9 control calls (grant/config/set/abort). The environment variable name and the value 60
  are **invented here**: neither the contract nor an ADR defines a control-request deadline.
  (`adapter-v1` defines a deadline ONLY for `session/prompt` - "No default deadline" - and for
  `question` - "cancel the waiting question". Nothing for grant/config/set.)
- **A default turn deadline, 30 min.** `hub/src/main.rs` calls
  `timeout_unconfirmed_cancels(30, 1800)`; the `1800` stops and settles a `running` turn
  after **30 minutes even with no cancel**. The design says `session/prompt` has **no default
  deadline**. A long legitimate task is aborted by a value nobody authorised.
- **The cancel-confirm window, 30 s.** The first argument (`30`) is the window before an
  unconfirmed `cancelling` turn is settled. The number is invented; the intent is the
  **shortest** possible (the owner's words: cancel should be ~0 s).

## Why this is T0

- It is not a wrong constant to tune. The **shape** is wrong: cancel is a blocking wait in
  the request path instead of a fire-and-report command; a turn has an invented default
  deadline the contract forbids; an invented env-var deadline sits under 9 calls. Every one
  of these is the opposite of the design ("issue and subscribe", "no default deadline",
  "abort ACK != stopped").
- It is a pattern, not one slip: when the document gave no value, a value was invented and
  then treated as fact (60, 30, 1800; the env-var name).

## What "fixed" must look like (for the later work, not yet done)

1. **Cancel returns immediately** with the current turn state (idempotent). It SENDS the
   abort and returns; the ACK is not awaited as the stop proof.
2. **The stop is proven by the harness's `turn_end`** (the event), as the contract says. A
   non-delivery of the abort is `abort-failed`; "delivered but unconfirmed" is judged by the
   core's cancel timeout, externally.
3. **No invented default turn deadline.** A turn runs until it ends; a deadline exists only
   where the design put one (none for prompt). If an operator deadline is wanted, it is an
   explicit, named input, not a hidden 1800.
4. **Every timeout is audited against the contract.** A deadline is applied only where the
   contract names one; where it does not, either there is none, or a design decision (ADR)
   adds it explicitly. The env-var name and default must be a decision, not an artifact.

## Note

This issue does not itself change code. The timeout/cancel semantics are re-specified first
(with the owner), then implemented; the 9 `control_request_timeout` sites and the sweep are
re-audited one by one against the contract as part of that.
