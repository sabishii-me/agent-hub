# 20261005-150000 — the host's real provider returns HTTP 400, so any session test is blocked (environment, not the hub)

## Status
OPEN — an ENVIRONMENT fault on this host, NOT a hub defect. Recorded so a red session test is not
mistaken for a regression.

## Observed (this session)
`tests/lib/hub.py::real_provider()` reads the real provider from `~/.pi/agent/models.json`
(`HOME-JP-prod` -> `http://192.168.31.29:8990`, api `anthropic-messages`). A hub session on the
installed `pi` harness fails to start:
```
session status: starting_failed
startError: adapter refused: cannot inject provider http://192.168.31.29:8990:
            provider http://192.168.31.29:8990 /models -> 400
```
Reproduced OUTSIDE the hub, with the token:
```
GET http://192.168.31.29:8990/models   (Authorization: Bearer <real token>)  ->  400 Bad Request
GET http://192.168.31.29:8990/models   (no token)                            ->  401
```
So the provider itself answers 400 to its own `/models` with the host's token. Every test that needs a
real session (whole-chain, model, tools, presets, interrupt, ...) will fail `starting_failed` for this
reason until the provider answers.

## It is NOT the registry/install work
The install/reconcile change is independent: lifecycle 16/16, install-refusals 14/14, concurrent 6/6,
killed-mid-remove 3/3, install-reconciles 4/4, no-arbitrary-url-install 4/4, events 8/8 all pass with
the real registry path. The blocker is upstream of the adapter turning a turn.

## Action
An owner/host matter: restore/point the provider (`192.168.31.29:8990`) so `/models` answers 2xx with
a valid token, or update `~/.pi/agent/models.json`. No hub change is implied.
