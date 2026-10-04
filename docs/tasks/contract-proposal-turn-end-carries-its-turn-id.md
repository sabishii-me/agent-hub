# Contract proposal (for owner review): `turn_end` must carry the turn's identity

Status: PROPOSAL - not written into `contract/`. Needs the owner's review before it is.

## Why

A turn's terminal must be bound to the turn it ends. Today the `turn_end` event carries only
`{sid, state}`, so the hub keys the terminal by SESSION. A cancelled turn settles just before
the next turn begins; its terminal is then taken by the NEXT turn, which wrongly settles
`failed` (docs/issues/20261004-040000). The event names nothing, so the hub guesses by order -
the same "guess instead of read the fact" defect the sweep had.

The prompt already names its turn: `session/prompt` params are
`["sid", "message", "clientMessageId", "images?"]`. The terminal event must echo that identity
back.

## The change (one line in `contract/adapter-v1.json`)

In `coreCompliance.events`, the `turn_end` entry:

before: `{"type": "turn_end", "field": "state: ok|aborted|failed"}`
after:  `{"type": "turn_end", "requires": ["clientMessageId"], "field": "state: ok|aborted|failed", "note": "clientMessageId echoes the session/prompt it ends, so a terminal is bound to the exact turn, not to the session"}`

`session/prompt` is unchanged (it already names its `clientMessageId`).

## What changes with it

- an adapter MUST echo the prompt's `clientMessageId` on its `turn_end`.
- the hub binds a terminal BY that id: `run_turn` stores/reads the terminal under the turn id,
  never the session id.

## Effect on adapters

pi/jouzu/dsh adapters each add the `clientMessageId` they were sent to their `turn_end` event.
A plugin that does not is a plugin bug caught by the adapter self-check once this is in
`contract/adapter-v1.json` (`checkedBy`).

## Not proposed

No `turnId` field invented alongside `clientMessageId` - the prompt's existing identity is the
one identity; a second name for the same thing is the "version in two fields" mistake ADR-0004
warns about.
