# 20261003-122000 — the contract contradicts itself on the closed-session status

Recorded: 2026-10-03. Found by: the lifecycle test. Owner: **CONTRACT**. Status: recorded.

## The contradiction

- `POST /v1/sessions/{id}/close` description: "The session is kept with status='closed' and
  can be reopened."
- `defs.session.status` enum: `starting | active | starting_failed | needs-repair | readonly`
  with "`readonly` = closed". There is NO `closed`.

A consumer reading the close description looks for a status that can never occur.

## Fix direction

Make the two agree: either the description says `readonly` (= closed), or the enum gains
`closed` and the hub writes it. Pick ONE name. (The hub writes `readonly` today.)

## Verify

The contract describes one status name; `/v1/sessions/{id}` after close reports that name.
