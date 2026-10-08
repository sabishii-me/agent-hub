# 20261008-100100 - thinkingLevel was never injected, recorded, or validated (FIXED)

**State:** FIXED (20261008). Owner was pi adapter + hub. Verified: `tests/sessions/thinking-level-is-real-and-validated.py` **14/14**.

## The three real defects

1. **The pi adapter read the wrong field.** Contract `credentials/grant` passes the core-managed
   provider's model descriptions as **`declarations`** (a map keyed by model id); the adapter read
   `params.models` (an array), got nothing, and built the injected model with **no `reasoning` and no
   `thinkingLevelMap`**. pi's `getSupportedThinkingLevels` then offered only `["off"]`, so every
   reasoning level **clamped to `off`** and the adapter reported "not applied". Fixed: read
   `declarations` (map) with `models` as a legacy fallback.

2. **The hub never recorded the level.** `SessionView`/`SessionRow` had no `thinkingLevel` /
   `appliedThinkingLevel`, though the contract's session object specifies both. Added the two
   columns/fields and populated `appliedThinkingLevel` from the adapter's confirmed
   `applied.thinkingLevel` (start + patch).

3. **A client-input refusal quarantined the session.** A level the model does not offer was applied
   by pi (which CLAMPS) and came back as an unconfirmed/unknown outcome -> `502 adapter_crash` and the
   session was torn down. Fixed at both ends: the adapter asks
   `get_available_thinking_levels` FIRST and refuses an unoffered level with `validation-failed`
   (before any partial apply); the hub maps a REFUSAL (an answer: nothing ran) to its contract code -
   `400 validation_failed` - and KEEPS the session. Only a genuinely UNKNOWN outcome still quarantines
   (TASK-048 F3/F5).

## Also fixed here

`/v1/harnesses/{id}/models` returned 0 models - it omitted core-managed providers entirely (see
20261008-100000). With that fixed, (A) can state the model's real levels.
