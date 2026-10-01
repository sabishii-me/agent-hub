# System alignment table (7af2866 review)

One row per capability: the approved target, the hub entry + responsibility, the
adapter/plugin responsibility, the CURRENT real implementation, the gap owner, the required
upgrade, and the real acceptance. Sources: ARCHITECTURE.md, docs/review/VERIFICATION-TASKS.md,
ADR-0007/0008, RECONSTRUCTION-ALIGNMENT-7af2866. This is a WORK RECORD, not a new authority.

Legend for gap owner: HUB (hub-side work owed) | PLUGIN (adapter/provider must be upgraded
to the new contract) | CONTRACT (owning contract must publish the artifact) | UNAUTH (needs
an authorized real call).

## Per-capability

### Plugin install / prepare / activate / upgrade
- Target: install from git/artifact; prepare runtime; enable/disable; a minimum-host gate.
- Hub: POST /v1/plugins, /{id}/prepare, /{id}/enable|disable; owns landing + version gate.
- Plugin: ships a manifest (id, pluginType, protocol, command, runtime, extensions, minHubVersion).
- Current: install (git + verified artifact) + enable/disable + catalog + icon REAL; NO
  minHubVersion field or gate anywhere.
- Owner: HUB + CONTRACT + PLUGIN. Upgrade: add minHubVersion to the manifest contract;
  enforce at ONE accept/activate boundary; upgrade the adapters to declare it.
- Real acceptance: low-version plugin refused with reason; supported activates; the adapter
  starts only when allowed.

### Session create / close / reopen / fork / history
- Target: process-backed session; native history stays in the harness.
- Hub: create/close/reopen/fork + read-through (messages/stats). Plugin: spawn/resume/fork
  and own the native log.
- Current: REAL (create/close/reopen/fork/messages/stats); reopen re-grants.
- Owner: none. Real acceptance: lifecycle across close/reopen/fork, history continuous.

### Provider type / secret / catalog / grant / model
- Target: provider is DATA; the hub owns HTTP/auth/catalog/field-map.
- Hub: /v1/model-providers*; keychain ref; grant -> config/set. Plugin: ships a TYPE descriptor.
- Current: secret+grant+refresh+selection+pending barrier REAL; `types` loader REAL (reads
  provider.json); create accepts ANY type; availability = "field present"; grant ignores type.
- Owner: HUB + CONTRACT + PLUGIN. Upgrade: publish the descriptor location/version/semantics
  in the owning contract; ONE type authority for enumerate/create/availability/execution;
  upgrade the provider plugin's artifact.
- Real acceptance: unknown type refused BEFORE the secret store; a record naming a missing
  type is readable+unusable; grant refused for an unusable type.

### prompt / cancel / terminal / restart recovery
- Target: turn identity + terminal from the adapter; cancel bound to the process.
- Hub: POST turns, /cancel, GET turns. Plugin: answers session/prompt/abort/turn-ended.
- Current: REAL, but prompt uses a plain send (can cross stop/reopen to a new process); EOF
  is still used as exit in places; terminal_intent is consumed only at boot.
- Owner: HUB. Upgrade: bind prompt to the turn's process generation (like cancel); confirm
  stop via the child; wire terminal_intent into the online sweep.
- Real acceptance: an old turn never reaches a new process; a known terminal write that
  failed is reconciled online.

### preset / plan / review / approval / question
- Target: preset switchable without wedging; approval is a REAL round-trip.
- Hub: PATCH /sessions/{id}; GET/POST approvals|questions. Plugin: baseline preset/approval/
  plan/review; pi/jouzu/dsh already send `approval_need` RPC.
- Current: G1 CLOSED - reverse requests keep their id and are answered (approvals round-trip).
  preset restart+resume REAL (success path); restart failure can leave `active`; the restart
  early-return skips thinkingLevel (G5).
- Owner: HUB (G5). Upgrade: restart must fail into needs-repair; a composite PATCH must run
  every field.
- Real acceptance: a real approval preset waits before a side effect; /v1 allow/deny resolves
  the SAME waiting adapter; deny -> no side effect.

### harness-private connections / auth
- Target: the hub owns the route + pass-through; the adapter owns the harness mechanism.
- Hub: /v1/harnesses/{id}/connections*|auth*. Plugin: implements connections/*, auth/*.
- Current: G2 CLOSED - 8 routes mounted + forwarded with the contract parameter names.
- Owner: HUB (done) + PLUGIN (adapter must implement the methods for real data).
- Real acceptance: on a real capable harness, delete actually removes (list + persisted
  state), not just 200.

### skills / extensions / resources
- Target: plugin-sourced layered skills + skills:// hook; extensions placed per harness.
- Hub: extensions REAL (complete snapshot per start); skills/resources 501. Plugin: ships
  extensions; implements the hook.
- Current: extensions snapshot REAL; skills/resources NOT built (T2b pending); snapshot
  retention = fixed 8 with no reader-usage basis.
- Owner: HUB. Upgrade: build the new skills model (T2b) + resources; retention by real
  usage/reference.
- Real acceptance: a real harness loads a skill via skills:// end to end; a snapshot is not
  deleted while a running session reads it.

### events / errors / concurrency / recovery
- Target: SSE is a notification, not the fact; unknown results not faked; >=100 concurrent.
- Hub: /v1/events; every error typed. Plugin: emits notifications.
- Current: SSE real; typed error mapping real; terminal_intent online coordination missing;
  composite-command fields can be dropped.
- Owner: HUB. Upgrade: finish the execution-identity/lifecycle unification.
- Real acceptance: GET never shows active without a live confirmed process; unknown results
  held/quarantined.

## Delivery log

- 248f6be..7af2866: A2 preset switch (success) + applied_preset persisted; GET
  /v1/model-providers/types loader; provider auth operation store (start/status/cancel); the
  8 harness connections/auth routes mounted + forwarded.
- 5a7139c G1 CLOSED (live): the adapter bidirectional control protocol. A reverse request
  (approval_need) keeps its id and is answered; Humans raises the resource and awaits the /v1
  decision. Live: request -> /v1 visible -> /v1 allow -> the adapter gets the reply to the
  SAME id -> turn ok; reject -> no side effect, turn cancelled.
- 4aa9b99 G2 CLOSED (live): harness connections/delete sends {id} (not {connectionId});
  validate->draft; auth/start->providerId. Live: a real delete removes the target.
- G3 CLOSED: ONE provider-type authority (`Providers::with_type_resolver`, wired from the
  plugins root) serves create validation, `providerTypeAvailable` and grant admission. An
  explicit unknown type is refused BEFORE any row/secret write; a provider whose type later
  becomes unavailable is readable but UNUSABLE (grant refused with a reason); the built-in
  `custom-compatible` type is always available (the hub provides it). Live: unknown type ->
  400 before write; shipped type -> available; descriptor removed -> available=false and
  grant refused.
- G6 CLOSED: minHubVersion is declared in the manifest contract (adapter-v1.json) and
  enforced at the plugin accept (begin_install) and the adapter ACTIVATION scan; a plugin
  that needs a newer hub is registered disabled with a named reason
  ("needs hub >= X, this is Y"), never started. The harness view publishes
  `disabledReason`; the harness def gained an optional `disabledReason` (contract + openapi
  regenerated). Live: a plugin requiring 9.9.9 -> disabled + reason; a normal plugin ->
  enabled.
- G5 CLOSED: a preset restart that cannot re-establish the session now lands
  `needs-repair` (in memory AND durably) instead of staying `active`; a COMPOSITE PATCH
  (preset + thinkingLevel) runs every field against the fresh process and re-reads the
  confirmed applied identity before the final save. Live: composite switch -> applied
  tracks the effect and thinkingLevel reaches the new process; a resume failure ->
  502 repair_failed + status needs-repair (not active).
- G4 CLOSED: POST /v1/model-providers/{id}/auth NO LONGER fabricates a pending operation.
  The interactive-method check runs first and the route refuses 501 with NO side effect;
  an operation is created only by a real flow executor. Live: a refused start leaves no
  operation (status -> 404). The step schema still needs the owning contract before a real
  flow lands.

## Open links, by owner (next work)

- G4 remainder (HUB+CONTRACT): implement the real declarative auth flow (accept/execute/
  result/cancel) once the step schema is published; the placeholder is gone, the executor
  is still owed.
- G3 (HUB+CONTRACT+PLUGIN): ONE type authority; publish the descriptor artifact; upgrade the
  provider plugin.
- 17 remainder: the new plugin-sourced skills/resources (T2b) and the shared adapter layer.
- UNAUTH: a real pi/jouzu/dsh acceptance needs an explicit isolated-side-effect authorization
  (the hub probes the OS keychain at startup) and a credential authorization for a real turn.
