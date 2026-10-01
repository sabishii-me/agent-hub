# Deviation: the implementation was built against the *pre-decision* route surface

## The finding

The routes I implemented match **`contract/v1.json` as it stands today**. But
`docs/ROUTES-REVIEW.md` says that surface is the **"before" picture**, and that the
**decided sections are the target**:

> 1. Classes 1-10 - every route in `contract/v1.json` **today** ... This is a review of the
>    current surface; it is not the target.
> 2. The decided sections ... what the review settled. **Where a class verdict and a decided
>    section disagree, the decided section wins** (the class is the "before" picture).

The contract was **never updated** to the decided surface. So building against the contract
produced exactly the surface the review decided to change. This is an execution deviation, not a
new decision.

## What the decided surface requires (the target)

| today (`v1.json`, what I built) | decided target | authority |
|---|---|---|
| `/v1/hub/status`, `/v1/hub/surface`, `/v1/hub/events`, `/v1/hub/shutdown` | `/v1/status`, `/v1/surface`, `/v1/events`, `/v1/shutdown` (**drop `/hub/`**) | decided: drop `/hub/`; class 2 |
| `/v1/hub/plugins...` | `/v1/plugins...` (incl. `icon`, `prepare`) | class 3 |
| `/v1/hub/registry/refresh`, `/v1/hub/catalog` | plugins class (registry cache by plugins) | class 3 |
| `/v1/hub/orphans`, `/v1/hub/orphans/{id}` | **REMOVE - not a route** (internal; atomic replace + boot sweep) | class 3 |
| `/v1/hub/harnesses` + `/v1/harnesses` | **one** `/v1/harnesses` (thin top-level projection) | o4; class 4 |
| `/v1/hub/harnesses/{id}/enable|disable` and `/v1/harnesses/{id}/enable|disable` | **DELETE all four** -> `/v1/plugins/{id}/enable|disable` (plugin lifecycle) | class 4 |
| `/v1/hub/harnesses/{id}` (PATCH) | **MOVE**: `enabled`->plugin; `extensions`->`/v1/harnesses/{id}/extensions`; `skills`->skills mechanism | class 4 |
| `/v1/hub/providers...` | `/v1/model-providers...` (**rename**); `/v1/hub/provider-types` -> `/v1/model-providers/types` | decided; class 5 |
| `/v1/hub/providers/models/list` | **`/v1/models`** (top-level: the models the hub manages) | class 5 + 10 |
| `/v1/hub/connections...` | `/v1/connections...` | class 6 |
| `/v1/hub/skills...` | **`/v1/skills`** (top-level, no `/hub/`); **source becomes the plugin**, not a hub-authored store | class 7; decided |
| (extensions) | **`/v1/harnesses/{id}/extensions`** (private: a bare id is meaningless across harnesses) | decided; o4 |
| `/v1/models` | (new) the hub's own models | class 10 |
| every route | **requires the token** (no anonymous discovery) | o5 |

Also decided, and not yet reflected:

- **skills source is a plugin**, so the hub-side `PUT/DELETE` skills routes are **REVIEW** items
  (a write route conflicts with the security rule); my `PUT /v1/hub/skills/{id}/files` is the
  route the review flagged.
- **skills/extensions are layered**: a session inherits the workspace set and may carry a
  session-private set; the hub must not store one set on the harness row.

## Why this happened

I read `contract/v1.json` as the interface (ADR-0011: "the contract is the interface") and
did not reconcile it with `ROUTES-REVIEW.md`'s decided sections, which explicitly override the
class tables. The two documents **disagree**, and nothing marked the contract as stale.

## The real problem to settle first

`contract/v1.json` and `ROUTES-REVIEW.md` (decided) are **two different surfaces**. Per ADR-0011
the contract is the interface - so the **contract must be updated to the decided surface**, and
only then does implementing it mean the right thing. Editing code to the decided routes while
the contract still prints the old ones would be a second source of truth.

## Order of work this implies

1. Update `contract/v1.json` (and the adapter/errors files if needed) to the **decided** surface:
   drop `/hub/`, rename model-providers, move models, top-level skills, harness extensions,
   remove orphans, one harnesses route, plugin-lifecycle enable/disable, token on every route.
2. Re-run the T1 normalizer so `openapi.json` reflects it.
3. Then rework the crates' `routes.rs` to the contract (the implementation follows the contract).
4. The hub's mounted-surface self-check then compares against the updated contract.
