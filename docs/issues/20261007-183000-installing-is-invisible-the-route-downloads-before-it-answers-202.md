# 20261007-183000 - `installing` is invisible: the install route downloads BEFORE it answers 202

**State:** OPEN (fixing)  
**Severity:** T0 - the state machine lies. During a real install the resource is `404`, then jumps to
`ready`; `installing` never appears.

## Measured (dev, a 1.8s slow download)

```
t=0.0s  GET /v1/plugins/pi -> 404            (downloading; the client sees "not installed")
...
t=1.8s  GET /v1/plugins/pi -> 200 ready      (straight to ready)
POST /v1/plugins           -> 202            (only AFTER the download finished)
```

`installing` never appears; a client cannot tell "not installed" from "installing". The contract
(openapi): a plugin's state is "installing (fetching and landing it) ... The resource a 202 Location
names" - the resource must be readable, in `installing`, while it fetches.

## Cause

`crates/plugins/src/routes.rs`: the POST handler `.await`s `begin_install`, and `begin_install`
(`crates/plugins/src/service.rs`) performs the ENTIRE download+verify+unpack (`source::resolve`)
SYNCHRONOUSLY. Only AFTER that does it `enter(Installing)` + `announce` and return `Proceed`. The
route then answers 202 and spawns `finish_install`, which finds the staging already complete and
lands it in ~0ms. So `installing` is announced but lives ~0ms.

## Fix

`begin_install` must validate cheaply, register the command identity, `enter(Installing)` and
announce - and NOT download. The download+stage+land move into the detached `finish_install`. Then
the route answers 202 + state `installing` FIRST, the resource is readable as `installing` for the
real duration of the download, and only then becomes `ready`/`failed`.
