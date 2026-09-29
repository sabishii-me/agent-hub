# Verification tasks (implementation stage)

The architecture stage is done: G1–G6 are revised as **design** (decisions and stated boundaries),
not as running code. **Runtime verification is not a merge condition for the architecture** and is
not claimed here. It is turned into **implementation tasks** below, each with a **minimal
verification implementation** and an **exit condition**. A reviewer accepts the architecture
against its design; these tasks run when a real, attributable artifact exists.

Two things are marked per task: whether it **affects the route** (a feasibility unknown that could
change the architecture if it fails) and its exit condition.

---

## T1 — Contract normalization is a real, single authority (G3)
- **What**: `contract/v1.json` is a compact DSL; the current projection is not valid JSON Schema
  (reproduced: `type:[]` x4, `nullable:true` x12, `type:binary` x1, `const:manual` x1, and PATCH
  `/v1/sessions/{id}` marks all 8 optional source fields required).
- **Minimal implementation**: one normalizer DSL -> JSON Schema; regenerate OpenAPI and derive the
  serde types from the same output; a check that the committed artifact equals a fresh generation.
- **Exit condition**: the artifact is valid JSON Schema 2020-12; positive and negative samples pass
  through the normalizer; `jsonschema` accepts valid and rejects invalid bodies; the PATCH body is
  no longer over-constrained. **Affects the route: no** (the defect is local; the fix is bounded).

## T2 — Security: state the boundary, then prove what is provable (G1)
- **What**: ARCHITECTURE 7 already narrows the claim (placement is not an authorization boundary).
- **Minimal implementation**: (a) define the OS principal/permission model for hub, adapter, harness
  and its children; (b) state the tool set each harness enables, read from `/v1/harnesses/{id}/tools`
  (never assumed); (c) place the skills hook + the extension placement; (d) decide the management
  bearer's reachability to an agent.
- **Exit condition**: for pi/jouzu/dsh on Windows x64 and macOS arm64, an agent identity cannot
  mutate a one-shot skill or answer its own approval **before any side effect** - or the claim is
  explicitly narrowed to "reduces accidental loading/exposure". Absolute-path write, shell/child,
  links and the reboot load chain are each recorded for all three harnesses. **Affects the route:
  YES** (see T2a/T2b below).

### T2a — is an execution boundary reachable? (feasibility unknown)
- **Minimal implementation**: determine whether the harness can run under a distinct principal with
  read-only access to the hub-owned resource paths (OS-level), or whether a tool policy that cannot
  be bypassed (a gate outside the agent's writable space) is achievable per harness.
- **Exit condition**: a working boundary on at least one platform, **or** a written decision to
  narrow the claim and rely on the tool/adapter gate. **Exit of the unknown: yes/no answer recorded.**

### T2b — does the skills hook deliver on real harnesses? (feasibility unknown)
- **Minimal implementation**: the `node:fs` hook must serve the **directory loader**, **relative
  resources**, the **effective set** and **reload** (a `readFileSync` probe is not enough).
- **Exit condition**: pi, jouzu and dsh each load a skill via `skills://` end to end (install ->
  session -> tool sees the skill), with no real path exposed. **If a harness cannot**, its skills
  are delivered another way and that is recorded. **Affects the route: YES** (per-harness).

## T3 — Plugin install/replace recovery (G2)
- **What**: recovery table in ARCHITECTURE 10 (before/after each move, after commit, rollback
  failure, recovery re-interrupted).
- **Minimal implementation**: implement the two renames + DB commit with the recorded state; a boot
  pass that finishes what a kill left; GC by rebuildability + references.
- **Exit condition**: a kill at each boundary (before move, after move 1, after move 2, after
  commit, during rollback, during recovery) leaves a consistent state with no user content lost,
  and recovery resumes from the recorded state. **Kill and power-loss recorded separately.** The
  same plugin's prepare/delete/replace never contaminates a newer instance. **Affects the route:
  no** (the rules are decided; this proves them).

## T4 — Long operations: acceptance, result ownership, SSE convergence (G4)
- **What**: semantics table in ARCHITECTURE 11.
- **Minimal implementation**: the resource answers accepted/in-progress/failed/unknown through GET;
  the command is idempotent by target (a turn is not replayed); SSE has subscribe-then-read, a
  bounded subscriber, and a rule for an expired `Last-Event-ID`.
- **Exit condition**: for install/remove, start/fork/compact, turn and auth: a lost `202` retried
  does not double-apply; a restart leaves a state a client can interpret; an unknown terminal state
  is readable; SSE does not lose the last change across a drop/overflow/restart. **Affects the
  route: no** (semantics are decided).

## T5 — Adapter / skills / session boundaries (G5)
- **What**: the shared adapter library (written once), baseline implemented by the adapter, native
  conversation vs hub control state distinguished.
- **Minimal implementation**: the library + one adapter converted; the baseline (preset/approval/
  plan/review) implemented for that harness; the ID/ref mapping kept.
- **Exit condition**: two adapters (e.g. pi + dsh) share the library and each meets the baseline;
  a session's native history and the hub's control state are both correct across close/reopen/fork.
  **Affects the route: no** (the boundary is decided).
- **Deferred gate**: the Rust/Node bridge and the two-platform delivery — a choice with a reason,
  or explicitly deferred (does not block the architecture).

## T6 — Concurrency numbers (G6)
- **What**: ARCHITECTURE 12 states mechanisms, not proof; bounded **acceptance**, DB locks not held
  across long work, overload/disconnect/cancel/exit behaviour.
- **Minimal implementation**: the concurrent-poll harness that reproduced the freeze; explicit
  budgets per operation class; a busy/lock measurement.
- **Exit condition**: **>= 100 concurrent connections** and no in-flight operation times another
  out; **thousands of idle** connections without degradation; two heavyweight operations progress
  without starving each other; the numbers used are frozen **here**, not taken from the review.
  **Affects the route: no** (the architecture is measured, not changed).

---

## Summary: which unknowns can change the architecture

| task | affects the route? | if it fails |
|---|---|---|
| T1 normalization | no | fix inside the contract layer |
| T2a execution boundary | **yes** | narrow the security claim (allowed) |
| T2b skills hook, per harness | **yes** | deliver that harness's skills another way |
| T3 recovery | no | implement the stated rules |
| T4 async/SSE semantics | no | implement the stated semantics |
| T5 adapter boundary / bridge | no (bridge is a deferred gate) | pick or defer the bridge |
| T6 numbers | no | tune budgets; the architecture stands |

**None of these is a merge condition for the architecture stage.** They are the implementation
stage's tasks; the architecture states the design and the exit conditions, and does not claim the
runs.
