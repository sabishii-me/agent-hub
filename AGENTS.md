# AGENTS.md - agent-hub

The rules for working in **this** repository. This repository is the implementation: the
Rust code and the `/v1` contract. It holds **no documentation** - no architecture file, no
design notes, no issues log. Those live elsewhere.

## The task

**Implement the agent-hub: real, usable `/v1` capabilities against the REAL adapters.**

- A capability is DONE only when driven through `/v1` against a real adapter. A mock proves
  only the hub's half; it is not acceptance.
- Do not treat a bare "routes mounted" or "tests green" as done.
- Finish a capability, then pick the next one; do not stop for a "continue".

## Integrity

1. **Never render or record a conclusion stronger than the basis.** "Not run" is not
   "broken" and is not "passes".
2. **A claim of behaviour comes from a real run against `/v1`** - not a DOM string, not an
   echo, not an assertion that a status is `active`.
3. **Fix at the root / contract boundary.** No local patches; no repair of a command the
   contract already names differently.
4. **Honest unavailability beats a fake success.** An unbuilt capability answers `501`. A
   test that cannot run prints SKIP and returns; it is never reported green.
5. **Explore first, then decide.** Do not ask two-choice questions when both are true; do
   not ask when you can determine it.

## Boundaries

- The hub is a **separate project**: branch `feat/hub-modular-redesign`; PRs go through
  review, not direct to `main`.
- **No blocking requests**: long work is `202 + Location` and reports via the event stream;
  nothing in request handling blocks the runtime.
- **Mature components, not hand-rolled.**
- **The contract is the interface**: a change is expressed in `contract/` and only there.
- **Never** commit or overwrite uncommitted user work; never reset hard.

## Harness acceptance is a MERGE GATE

**A harness (plugin/adapter) that does not pass the required tests may NOT be merged into the
main branch.**

- For a harness `H`, run the whole-chain test and every `tests/interrupt/*.py` against its
  REAL adapter (`PI_PLUGIN_DIR=<H> PI_HARNESS_ID=<H>`): all green, or `H` does not merge.
- A RED case is a red harness: fixed in the adapter, or blocked by a recorded issue; the
  harness does not merge while the case is RED.
- A case `H` genuinely cannot express is a recorded limit, not a pass and not a skip.
- **Never** make the gate green by weakening a test, skipping a case, or faking an adapter.

## Authorization boundary

**Never hardcode a secret.** No token, API key, password, or private key is written into
source, a test, a fixture, a default, or a commit message - not even as a fallback value
(`process.env.KEY || '<literal>'`). A secret always comes from the environment or the OS
keychain; a test that needs one reads it from an env var and FAILS (never skips, never
falls back) when it is absent. A literal `sk-...` in the tree is a leak: it must be revoked
and removed, not trusted.

Without explicit authorization, do **not**: read real credentials
(`~/.pi/agent/models.json`), import an apiKey, touch the real OS keychain, or make a real
vendor/paid model call. That authorization blocks only the real call; it does not block the
implementation. Never substitute a fake provider or a fake green. Merge/push/publish follow
their own authorization.

## Where things live

- `contract/` - the `/v1` contract, the adapter protocol, the error table. The interface.
- `crates/` - the domains; `hub/` - the binary and the boot self-check.
- `tests/` - the Python acceptance suite (env-driven, against a real hub and real adapters).
- Verification: `cargo build --workspace` (zero warnings) and `cargo test --workspace`,
  plus the gated real-hub tests that spawn the actual binary.
