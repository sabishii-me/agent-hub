# 20261005-141000 — a POST body missing a required field returns 422 + axum's text, not a contract error code

## Status
OPEN — located.

## Observed (RELEASE build, real hub)
```
POST /v1/plugins  {}     (no `source` key)
-> 422  "Failed to deserialize the JSON body into the target type: missing field `source` at line 1 column 2"
```
This body is NOT a contract error: `contract/errors.json` has no 422 and no code here. The response
carries axum's own text, not a `{"error": CODE, "detail": ...}` body.

## Why it matters
Every route failure MUST use a contract code (the transport's whole design: a domain returns a code,
the transport maps it). A deserialization failure bypasses that, so a client that branches on the
contract code sees a shape it was never promised. It is a hole in the error surface.

## Fix
Make a malformed body answer a contract code. `validation_failed` (400) is the natural fit:
"the request body did not match the expected shape: <detail>". This requires the routes to accept the
raw body and deserialize themselves (or an extractor-rejection handler that renders a DomainError).
No contract CHANGE is needed if `validation_failed` is used as-is; confirm the owner is happy reusing
it for body-shape errors.

## Test
A release-build test: `POST /v1/plugins {}` -> 400 `validation_failed` with a JSON body carrying the
code. RED today (422 + text).
