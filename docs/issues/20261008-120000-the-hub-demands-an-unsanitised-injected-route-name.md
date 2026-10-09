# 20261008-120000 - the hub demands `hub-<id>` verbatim, but dsh sanitises its route key

**State:** OPEN. Owner: HUB/contract (a decision is needed).
**Severity:** with a hub-managed provider, a `deepseek` (dsh) session cannot start: the hub refuses
the adapter's applied proof.

## Measured (dev, real dsh 0.2.0-rc.2, real provider `custom:HOME-JP-prod`, model `gpt-6-astra`)

```
POST /v1/sessions {harnessId: deepseek, modelProviderId: "custom:HOME-JP-prod", modelId: "gpt-6-astra"}
-> session starting_failed:
   the adapter did not confirm the requested provider/model/route:
   requested provider=Some("custom:HOME-JP-prod") model=Some("gpt-6-astra"),
   applied={"connectionId":"hub-custom_HOME-JP-prod",
            "modelProviderId":"custom:HOME-JP-prod","model":"gpt-6-astra"}
```

The adapter applied the right provider and model (both confirmed). Only the ROUTE failed.

## Cause

`crates/sessions/src/runtime.rs` (the grant check) requires the applied native route to be exactly
`hub-<providerId>`:

```rust
let ok_route = match applied_route.as_deref() {
    Some(r) => !r.is_empty() && r == format!("hub-{want}"),
    _ => false,
};
```

`pi` builds its injected route as `hub-<connectionId>` verbatim, so it matches. `dsh` route keys are a
settings path segment and cannot contain `:`; the adapter sanitises the id
(`'hub-' + connectionId.replace(/[^A-Za-z0-9_.-]/g, '_')`), producing `hub-custom_HOME-JP-prod`. So the
`custom:` namespace (the rule from 20261008-090000) makes the two disagree for dsh.

## The contract

adapter-v1 says `applied.connectionId` is "its RESOLVED NATIVE ROUTE" and that an injected provider
is "named `hub-<coreProviderId>` in every harness". The hub's check reads the second half literally
and rejects the first: the adapter cannot report a route the harness does not actually hold.

## Decision needed (owner)

One of:
- the hub accepts the ADAPTER's reported native route as the proof (it is what the harness really
  holds), not a hub-computed `hub-<id>` string; or
- the contract fixes the injected name to a harness-safe charset (no `:`), so `custom:` becomes e.g.
  `custom-`, and every adapter builds the same `hub-<safeid>`.

Until then, a hub-managed provider on dsh cannot start a session (a native-routing dsh session
without a hub provider works).
