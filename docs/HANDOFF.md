# HANDOFF — read this first (2026-10-03)

Written because the context is about to be lost. Read this, then `docs/tasks/CURRENT-STATE.md`,
then the two T0 issues. Do NOT act before reading the T0 issues.

Hub HEAD: `3d30f8c523630c044f8a8fc4e8d4c88aecab694e` (branch `feat/hub-modular-redesign`).
Working tree clean. Plugin SHAs: pi `9fb8c1c` (runtime 1.0.0), jouzu `31ce603`, dsh `580aca8`
(+ an UNCOMMITTED `deepseek-adapter.cjs` dialect fix + `runtime.sources.json` fix — real
defects, keep them).

## THE TASK (the original words)

> Implement the Rust agent-hub per the approved architecture, delivering REAL, usable `/v1`
> capabilities against the REAL adapters/plugins. NOT: reviewer numbers, green tests, mounted
> routes, or mock-driven "verification". §17 of ARCHITECTURE.md is the order of work.
> The contract is the interface; implementations are private (ADR-0011).

## WHY THIS HANDOFF EXISTS (the T0)

The owner found that the implementation does NOT follow the design: whenever a document was
SILENT, a value/shape was INVENTED and then treated as fact. Recorded in:
- `docs/issues/20261003-150000-T0-cancel-and-timeouts-diverge-from-the-design.md`
- `docs/issues/20261003-151000-T0-audit-my-deviations-from-the-design.md`  (D1-D7)

Headline: **cancel** is a blocking wait in the request path, but the design (`v1.json`
cancel = idempotent; `adapter-v1:387` "abort ACK != stopped, turn_end proves it"; ADR-0009
rule 2 "returns immediately, client never holds a connection") says fire-and-report. Plus
invented deadlines (control-request 60s env `AGENT_HUB_CONTROL_TIMEOUT_SECS`; default turn
deadline 1800s the contract's "No default deadline" forbids; cancel-confirm 30s). Plus D5:
the adapter seam is 2 env vars in the contract, 8 read by real adapters, 13 sent by code -
the interface is whatever the code grew, which violates ADR-0011. Plus skills built on the
excluded model (D7).

A "task-alignment review" PASSED because it checks the DIRECTION/target (aligned), not
whether the implementation OBEYS each contract/ADR clause (it does not). That gap is the T0.

## WHAT THE OWNER DECIDED / SAID (verbatim intent)

- "cancel 最小等待 1s 不少30" -> the cancel-confirm window should be ~1s, NOT 30.
- "取消为什么需要等待，我其实希望取消是0s" -> cancel itself should be ~0s (no wait).
- "turn 30分钟太短，如果我一个任务能跑1年你是不是也给我打断了" -> a turn must have NO
  default deadline (the contract agrees: "No default deadline").
- "60完全是你随意的无根据的值，你必须全部重新审核" -> the 60 is unfounded; re-audit ALL of them.
- "你这个不能补丁，可能需要review整个项目，考虑全部删掉重新设计" -> possibly a full review or
  a rebuild, not a patch.
- "你为何之前任务对齐review是没问题" -> the task-alignment review passing is expected; the
  failure is that the implementation violates the DESIGN clauses, which that review does not
  cover.

NOT YET DECIDED by the owner: **(A) review the whole project against the documents and correct
each seam**, or **(B) rebuild the affected parts from the contract outward.** The two T0
issues are the evidence base. ASK the owner A or B before writing code.

## THE RULE THIS RUN MUST HOLD (broken repeatedly before)

**When a document is silent, DO NOT invent.** Stop and ask for a design decision (contract or
ADR). Every invented value/shape in D1-D7 is this same mistake. Correcting values (30->1,
drop 1800) does NOT fix it, because the MOVE remains.

Also: never `git checkout --`/reset/discard; no `_`-prefixed scratch dirs; a contract/ADR
change needs the owner's review BEFORE it is written; docs first; tests are REAL (no fakes,
no mocks) - a missing real dependency prints SKIP.

## STATE OF THE WORK (what is actually true)

- Surface: `/v1/surface` == contract (74 == 74), no stub route. This is a ROUTING fact.
- `cargo test --workspace` = 39 result sets ok (function tests; NOT a capability claim).
- REAL acceptance done (docs/tasks/REAL-ACCEPTANCE.md): pi (runtime 1.0.0) and jouzu answer a
  real model turn; deepseek + shisa provider plugins work; hub-owned device-code sign-in
  works. dsh is BLOCKED (docs/tasks/dsh-blocked.md).
- New REAL test suite (Python, no fakes), `python tests/run.py [layer]`, layers under
  `tests/`: contract, lifecycle, interrupt, connections, skills, plugins, harnesses, provider,
  model, tools. ~116 real assertions across ~44 of 74 routes. The old Node-hub suite is
  archived under tests/archive/ (drives the OLD hub, not run).
- Open real defects (docs/issues/): F1 cancel reports failed (PLUGIN pi); F2 fork empty body
  400 (HUB); F3 contract close=closed vs enum readonly (CONTRACT); F4 remove deployment dir
  500 vs 409 (HUB); F5 idle-cancel 400 vs idempotent 200 (HUB); + the T0 issues.

## THE AUTHORITATIVE DOCUMENTS (read these, do not trust memory)

- `contract/v1.json` (the interface; 74 endpoints) and `contract/adapter-v1.json` (the
  hub<->adapter seam). `contract/errors.json`. OpenAPI is generated.
- `docs/ARCHITECTURE.md` (§17 order; §18 status; §5 providers-are-DATA; §6 adapter boundary;
  §7 skills/security). `docs/ROUTES-REVIEW.md` (the decided route/skills/auth surfaces).
- `prts-web/docs/decisions/ADR-*.md` (esp. 0008 minHubVersion, 0009 concurrent service,
  0010 mature components, 0011 contract-is-the-interface, 0012 auth-as-data).
- `docs/tasks/CURRENT-STATE.md`, `docs/tasks/system-alignment.md` (per-capability table).

## NEXT ACTION

Ask the owner: **(A) full review-and-correct against the documents, or (B) rebuild the
affected parts from the contract outward?** Do not start either without that answer. If the
owner picks A, start with **D5 (the adapter seam)** because ADR-0011 makes the contract the
precondition for everything else: produce a PROPOSAL for what the adapter env/seam contract
must say (env vars, what each carries), for the owner to review, BEFORE writing it.
