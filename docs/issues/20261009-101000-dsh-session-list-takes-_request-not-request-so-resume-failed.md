# 20261009-101000 - dsh `session/list` takes `_request`, not `request`; a resume after a restart failed

**State:** FIXED (adapter). Owner: adapter (dsh endpoint arg name).
**Severity:** after a power cut the dsh server is gone, so a reopen must respawn it and
`session/start(resume)` re-attaches by verifying the ref via `session/list`. That call failed every
time, so `POST /v1/sessions/{id}/reopen` answered `502 adapter_crash` and the recovered session could
not be used again.

## Measured (dev, real dsh 0.2.0-rc.2, Windows)

    POST /v1/sessions/{id}/reopen -> 502
    {"error":"adapter_crash","detail":"... rpc error -32000: session/start failed:
     dsh rpc session/list: {\"ok\":false,\"error\":{\"code\":\"gateway/arguments-invalid\",
     \"message\":\"typert gateway: session/list: args fields do not match the descriptor:
     missing \\\"_request\\\"; unexpected \\\"request\\\"\"}}"}

## Cause

In the dsh source (`@deepseek-ai/dsh-api-session-controller/lib/typert.remote-client.d.ts`) this ONE
method names its argument with a leading underscore:

    'session/list': (_request: SessionListRequest, signal?: AbortSignal) => ...

every other `session/*` method takes `request` (session/create, session/prompt, session/cancel,
session/fork, session/page, session/rename). The adapter sent `{ request: {} }`, so the typert
gateway rejected it: `missing "_request"; unexpected "request"`.

## Fix (adapter)

`dshInvoke('session', 'list', { _request: {} })`, with a comment naming the source line. Verified:
reopen after a power cut -> 200, session `active`, a new turn accepted.
