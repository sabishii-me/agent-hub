# The hub under attack — the adversarial baseline

These tests run a **real hub process** (this repository's `server.mjs`) against a throwaway
data directory and drive it over its **real HTTP surface**.

They install the **real released plugins** - the harness adapters and model providers this
deployment actually ships - not hand-made zips. `tests/fixtures/` holds those release
artifacts byte for byte, with the registry that names their version, sha256 and size
(`fixtures/README.md` says how they are refreshed). A fake manifest the hub happens to
accept proves nothing; the real ones carry the real id, kind, protocol, command, runtime
pin and capabilities, and the runtime closure that actually gets materialised. The install
path is exercised for real: download, size, sha256, safe unpack, manifest check, atomic
replace, and the runtime install that follows.

They are the baseline: every change to the hub runs them (`npm test`).

## Layout — one directory per thing attacked

| directory | what it attacks |
|---|---|
| `lifecycle/isolation.mjs` | acting on one plugin must not change any OTHER plugin's facts, and one read must never contradict itself |
| `lifecycle/events.mjs` | the hub.plugins.changed event must not lie: it names a real plugin and a real state, no event for a plugin nothing touched, and the state it names is the state a read returns |
| `events/` | the hub-level stream: the handshake vs a real change, subscribe, drop, reconnect, two subscribers |
| `hostile/` | malformed and hostile requests, and that the hub refuses them as data and stays alive |
| `recovery/` | the hub killed mid-removal and restarted on the same data dir; a plugin whose manifest is broken (ADR-0002: it degrades, the hub still starts) |
| `fixtures/` | the real release artifacts and the registry that describes them |
| `lib/hub-harness.mjs` | the shared harness: start a hub, serve a real artifact over loopback, a `/v1` client, an event subscriber, and the checks that a sample is self-consistent (`contradictions`) or isolated (`leaked`, `stableFields`) |
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
1b. A test uses the REAL plugins (`release('harness-adapter-deepseek')`), never a manifest
   it invented. Data that a test shaped to pass does not test anything.
2. A test never reads a source path from another project. It starts a hub from THIS
   repository and talks to it over HTTP.
3. A test cleans up: it kills the hubs it started and closes the servers it opened, so a
   run leaves no process behind.
4. When a check fails, **the hub is wrong until proven otherwise**. Look at the hub: a
   failing check is a finding, recorded as an issue (`agent-hub` repo). Editing the test
   or widening an assertion to make it pass is how a real bug gets buried - the state of
   a plugin, the event next to a read, and the isolation between two plugins are exactly
   the things this suite exists to catch.

## Running

```
npm test                 # every file
node tests/run.mjs lifecycle   # one group
```
