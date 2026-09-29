# Alignment audit: the accepted decisions vs the implementation

An audit of what the **approved** architecture (PR #12, approved at `6c8a0c9`) and its ADRs
require, against what the implementation actually does. It is a **fact check**, not a new
decision. Each row names the authority and the evidence.

The authority set: ADR-0009 (concurrency), ADR-0010 (infrastructure), ADR-0011 (the contract
is the interface), and `docs/ARCHITECTURE.md` (as revised through G1-G6 and R1).

## Findings

### D1 - A long route holds the connection instead of answering `202` (ADR-0009, §11, §13.2)

**Authority.** ARCHITECTURE §13.2: "A long route calls `accepted(location, work)` from
`transport/`, which answers `202 Accepted` + `Location` ... It **never holds the connection**."
§12: "a long operation does not hold a connection - it is **detached**". ADR-0009 rule 2: an
operation that can take more than a moment (install/remove/prepare, start/fork/compact, a turn)
**returns immediately** with a handle.

**Implementation.** `crates/plugins/src/routes.rs` `install`/`remove` and
`crates/sessions/src/routes.rs` `admit` **call the service inline and answer `200`** with the
finished result. The `Accepted` helper exists in `crates/transport` but no route uses it. So an
install holds the connection for its whole duration - the exact model ADR-0009 forbids.

**Deviation.** Yes.

### D2 - The plugin install ignores the client's command identity (R1, §11)

**Authority.** ARCHITECTURE §11 / R1: "Same command identity + same semantic request (a retry):
return / point at the **original** command's resource result; do not execute twice." The turn
route carries `idempotencyKey` (the contract mandates it).

**Implementation.** `crates/plugins/src/routes.rs` `install` builds its own id
(`format!("install-{}", uuid_like())`), ignoring any client key. `remove` does the same. So a
retried install/remove is always a **new** command - the R1 rule is implemented in the service
but unreachable from the route. (Sessions' turn route **does** read the client
`idempotencyKey`, so this is inconsistent as well as wrong.)

**Deviation.** Yes.

### D3 - A domain writes HTTP statuses (invariant §13.7)

**Authority.** ARCHITECTURE §13.7: "Errors are typed and mapped once. A domain returns a typed
error; the transport maps it via `contract/errors.json`. **No domain writes an HTTP status.**"

**Implementation.** Every domain's `routes.rs` maps its error enum to `StatusCode` itself
(`plugins`, `sessions`, `providers`). `contract/errors.json` is not read. The transport does not
own the mapping.

**Deviation.** Yes (and it means `contract/errors.json` is currently unused, which the contract
treats as authoritative).

### D4 - The adapter process gets no environment (the contract's adapter env)

**Authority.** `contract/adapter-v1.json`: the hub "hands `command` (**absolute**) to the adapter
as `AGENT_HUB_RUNTIME_COMMAND`"; `contract/v1.json` `session.cwd`: "passed to adapters as
`AGENT_HUB_CWD`". The contract names `AGENT_HUB_CWD`, `AGENT_HUB_INSTALLED_SKILLS_DIR`,
`AGENT_HUB_RUNTIME_COMMAND` (and the hub env `AGENT_HUB_PLUGINS_DIR` / registry vars). The old
hub injected the full set (`server.mjs:802/2964`).

**Implementation.** `crates/adapter/src/manager.rs` `ensure_started` spawns with an **empty
env** (`&[]`). A real adapter cannot start (`AGENT_HUB_RUNTIME_COMMAND` missing -> it refuses).

**Deviation.** Yes. **Reproduced** against the real pi/jouzu/deepseek adapters: they answer
`presets/list` over the bus (protocol compatible) but cannot boot a session without the runtime
command and the data dir.

### D5 - `manifest.runtime.command` is read as a string, not an argv array

**Authority.** `contract/adapter-v1.json`: `runtime` is `{package, version, command}` and the
hub hands that `command` to the adapter; the old hub validates `runtime.command must be a
non-empty argv array` and resolves its parts against the plugin directory
(`server.mjs:484/782`).

**Implementation.** `crates/adapter/src/manifest.rs` `RuntimeSpec.command: Option<String>` -
a **string**. The real manifests carry an **array** (`["node","runtime/dist/cli.js"]`), which
does not deserialize.

**Deviation.** Yes.

### D6 - The crates in §3 that are not started

**Authority.** ARCHITECTURE §3 lists the workspace: `contract/`, `transport/`, `events/`, `db/`,
`plugins/`, `harnesses/`, `sessions/`, `humans/`, `adapter/`, `providers/`, `skills/`,
`extensions/`, `registry/`; and `hub/state.rs`.

**Implementation.** Built: `contract`, `transport`, `events`, `db`, `plugins`, `sessions`,
`adapter`, `providers`. **Missing** (not yet started, and §18 says so): `harnesses`, `humans`,
`skills`, `extensions`, `registry`, and `hub/state.rs`.

**Deviation.** Not a deviation - these are **unimplemented work**, recorded as such in §18.

## What is aligned (spot-checked, not exhaustive)

- **ADR-0011 / the contract is the interface**: the wire shapes live in `crates/contract`; the
  DSL is normalized once (T1).
- **§13.3 the resource is the only truth**: no generic job object; state is read through GET.
- **§13.6 only `adapter/` spawns a process**: no other crate calls `Command::new` (checked).
- **§10 recovery before GC**, the recorded-step spine (T3), verified across a real crash.
- **§11 SSE convergence**: monotonic ids, bounded replay, resync (T4).
- **§5 providers are data**: one file per provider, no in-process plugin code.
