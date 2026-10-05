# 20261003-121000 — POST /v1/sessions/{id}/fork rejects an empty body with 400

Recorded: 2026-10-03. Found by:
`tests/lifecycle/a-session-closes-reopens-and-forks-without-touching-its-source.py`.
Owner: **HUB**. Status: FIXED (routes.rs fork accepts an absent body via Option<Json<ForkRequest>> + unwrap_or_default()).

## Repro (real)

- `POST /v1/sessions/{id}/fork` with NO body -> `400 Failed to parse the request body as
  JSON: EOF while parsing a value at line 1 column 0`.
- with `{}` -> 200 (works).

## Why it is a defect

The contract's fork request is `{"afterTurnId": "string?"}` - every field optional, so the
BODY is optional. A route whose body is optional must accept an absent body (and an absent
content-type), not 400. A client sending `POST .../fork` with nothing should fork at the tip.

## Fix direction

Accept an absent/empty body as `{}` for routes whose request has no required field (fork is
the concrete case; audit the other all-optional POSTs: close, reopen, compact, cancel).

## Verify

`python tests/lifecycle/a-session-closes-reopens-and-forks-without-touching-its-source.py`
with a no-body fork must be 2xx, not 400.
