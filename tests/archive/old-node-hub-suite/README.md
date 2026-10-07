# Archived: the old Node-hub test suite

These files drove the OLD Node hub (`server.mjs`) over `/v1/hub/...` routes, published via the
GitHub-releases registry. The Rust rewrite serves the `/v1` contract (no `/v1/hub/...`), so
these are kept as reference for their ADVERSARIAL DESIGN (real process, power-cut kill, a
named property per file, `tally`/`check`), NOT as a suite that runs here.

- `lib/`, `runtime/`, `interruption/`, `concurrency/` — the old adversarial set.
- `e2e/*.mjs` — the ping-pong runners that DID drive the Rust hub; superseded by
  `docs/tasks/test-suite.md` (real, layered, no fakes). Kept for reference.

The new suite is `tests/` (Python 3.13, stdlib) per docs/tasks/test-suite.md.
