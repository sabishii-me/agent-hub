# 20261008-100100 - PATCH thinkingLevel -> 502 adapter_crash ("thinking level was not applied")

**State:** OPEN. Owner: HUB/ADAPTER (to locate).
**Severity:** the contract says an unknown level is `400 validation_failed`; here a level the caller
believes the model offers is refused as a `502 adapter_crash`, and the session is torn down.

## Measured (dev, real provider, model `gpt-6-astra`, level declared `low`)

```
PATCH /v1/sessions/<sid> {"thinkingLevel":"low"}
-> 502 {"error":"adapter_crash",
        "detail":"the session could not be started: config/set outcome unknown:
                  adapter refused: thinking level was not applied: requested low"}
then the session is not running, so a later PATCH is 400 "the session is not running; reopen it".
```

## Notes

- The level was DECLARED on the model (`declarations[model].thinkingLevels=["low","high"]`) and the
  provider catalog carried it, but `/v1/harnesses/{id}/models` did not (see 20261008-100000), so the
  adapter may never have been told the model offers `low`.
- A refusal that tears down a live session over a config knob is a bigger effect than the knob; the
  contract wants a clean `400 validation_failed` for a level the model does not offer.
