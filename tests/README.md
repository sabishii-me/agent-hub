# The hub under attack — the adversarial baseline

These tests run a **real hub process** (this repository's `server.mjs`) against a throwaway
data directory and drive it over its **real HTTP surface**. A plugin is a zip built with
the hub's own writer and served over loopback, so the install path exercised is the real
one: download, size, sha256, safe unpack, manifest check, atomic replace.

They are the baseline: every change to the hub runs them (`npm test`).

## Layout — one directory per thing attacked

| directory | what it attacks |
|---|---|
| `lifecycle/` | installing and removing plugins: concurrent, staggered, same-id races, an aborted request, the in-progress window |
| `events/` | the hub-level SSE stream: subscribe, drop, reconnect, two subscribers |
| `hostile/` | malformed and hostile requests, and that the hub refuses them as data and stays alive |
| `recovery/` | the hub killed mid-removal and restarted on the same data dir; a half-written plugin |
| `lib/hub-harness.mjs` | the shared harness (start a hub, build/serve a plugin zip, a `/v1` client, an event subscriber) |
| `run.mjs` | runs every file; `node tests/run.mjs <group>` runs one group |

## Attribution — a run names the contract it was written against

Every file prints, before it runs:

```
<name> against contract protocol=<protocol> version=<version> sha=<contractSha> build=<buildId>
```

`sha` is the hash of `contract/v1.json` and `build` is this build's id, so a run is
attributable to a version. A test asserts **behaviour**, not structure; a test written
against another contract is reported, not silently passed.

## Rules

1. A test states a behaviour, in a sentence a person can read: "a raced removal converges
   to gone", not "removes() returns ok".
2. A test never reads a source path from another project. It starts a hub from THIS
   repository and talks to it over HTTP.
3. A test cleans up: it kills the hubs it started and closes the servers it opened, so a
   run leaves no process behind.
4. When a check fails, it is a finding. It is recorded as an issue (`agent-hub` repo), not
   weakened to pass.

## Running

```
npm test                 # every file
node tests/run.mjs lifecycle   # one group
```
