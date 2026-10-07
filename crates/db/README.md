# agent-hub-db

The hub's data layer (`ARCHITECTURE` §8) and the plugin install/replace **recovery
rules** (`ARCHITECTURE` §10, task T3).

## Why recovery rules, not "a transaction"

An install touches a directory tree (rename moves) and a database row. SQLite gives
atomicity for the row; it says nothing about the tree. So each move is preceded by a
durable `plugin_ops.step`, and a boot sweep (`recover`) finishes or rolls back what a
crash left, per the §10 table:

| crash after | recovery |
|---|---|
| `Staged` | drop staging; the old plugin is intact |
| `OldMovedAside` | restore `outgoing` -> `target`; drop staging |
| `NewInPlace` | finish the commit (or roll back if the new copy is gone) |
| `Committed` | sweep the transient `outgoing` |
| rollback failed | keep both copies + a record: "unusable, recovery retained" |
| interrupted again | resume from the recorded step, not from scratch |

Directory names are not the contract; the recorded step decides.

## Tests

`cargo test -p agent-hub-db` runs 7 boundary simulations plus 2 **real crash** tests:
`t3-kill-writer` records a step and then `abort()`s (no SQLite close, no checkpoint), and
the parent reopens the files and recovers.
