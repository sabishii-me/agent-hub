# AGENTS.md - agent-hub

The rules for working in **this** repository. The orchestration repository (`prts-web`) holds
the ADRs, reviewer tasks and cross-review reports; this repository is the **implementation**.

## The main task

**Implement the Rust agent-hub per the approved architecture, delivering real, usable `/v1`
capabilities.** Work the vertical chain in `README.md` end to end. Do not treat a reviewer
handoff (`prts-web/docs/tasks/TASK-048-hub-next-review.md`) or a review report
(`PROVIDER-TURN-REVIEW-*.md`) as your task: they are correction input. Finish a capability, then
pick the next one; do not wait for "continue".

## Integrity (inherited from the project; ADR-0002)

1. **Never render or record a conclusion stronger than the basis.** "Not run" is not "broken"
   and is not "passes".
2. **A claim of behaviour comes from a real run against `/v1`** - not a DOM string, not an
   echo, not an assertion that a status is `active`.
3. **Fix at the root / contract boundary.** No local patches; no repair of a command the
   contract already names differently.
4. **Honest unavailability beats a fake success.** An unbuilt capability answers `501`. A test
   that cannot run prints SKIP and returns.
5. **Explore first, then decide.** Do not ask two-choice questions when both are true; do not
   ask when you can determine it.

## Boundaries

- The hub is a **separate project**: changes require reporting. Branch `feat/hub-modular-redesign`;
  PRs go through review, not direct to `main`.
- **No blocking requests** (ADR-0009): long work is `202 + Location` and reports via the event
  stream; nothing in request handling blocks the runtime.
- **Mature components, not hand-rolled** (ADR-0010).
- **The contract is the interface** (ADR-0011): express a change in `contract/` and only there.
- **Never** commit or overwrite uncommitted user work; never reset hard.

## Authorization boundary

Without explicit authorization, do **not**: read real credentials, import an apiKey, touch the
real OS keychain, or make a real vendor/paid model call. That authorization blocks only the real
call; it does not block implementation. Never substitute a fake provider or a fake green.
Merge/push/publish follow their own authorization.

## Where things live

- `docs/ARCHITECTURE.md` - the architecture; §17 the order of work, §18 status, §24 the
  remaining surface. **§17 is the plan of record.**
- `docs/tasks/*.md` - the implementation work records.
- `crates/*` - the domains; `hub/` - the binary and the boot self-check.
- Verification: `cargo build/test --workspace` (zero warnings), plus the gated real-hub tests
  (`hub/tests/*.rs`) that spawn the actual binary.
