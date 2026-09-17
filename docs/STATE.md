# Where this stands, and how to pick it up on another machine

Read this first after a checkout. It is the state of the work, not the manual: the
README is the manual, [FEATURES.md](FEATURES.md) is what each feature promises and
what it does not, [CLIENT.md](CLIENT.md) is how a client connects.

## On a machine you have never used before

```
node -v                                 # 22+ (developed on 24)
node server.mjs                         # prints "agent-hub listening 127.0.0.1:<port>", and that it has no harness
# a harness arrives by install (the hub clones it into <DATA_DIR>/plugins) or by
# PRTS_PLUGINS_DIR pointing at checkouts; its runtime is prepared by the plugin itself:
#   POST /v1/hub/plugins {source:{url, ref?}}
#   POST /v1/hub/plugins/{id}/prepare        (also happens by itself before a session)

# the hub prints its port; hub-connect turns it into shell variables
eval "$(node scripts/hub-connect.mjs)"   # exports PRTS_URL and PRTS_TOKEN
curl -s "$PRTS_URL/v1/harnesses" | head -c 200          # discovery needs no token

# add a provider: its own url + token. The token goes to the OS secret store and is
# never written to the provider file; `declarations` is what a model list cannot tell
# anyone (modalities, context window, thinking levels, cost).
curl -s -X POST "$PRTS_URL/v1/hub/providers" \
  -H "Authorization: Bearer $PRTS_TOKEN" -H 'content-type: application/json' \
  -d '{"id":"myprovider","url":"https://api.example/v1","token":"sk-...","declarations":{"my-model":{"name":"My Model","input":["text"],"contextWindow":128000}}}'

curl -s -H "Authorization: Bearer $PRTS_TOKEN" "$PRTS_URL/v1/harnesses/pi/models" | head -c 300   # configured => available
```

What is **not** in the tree and must be recreated on a new machine: the harness
runtimes (materialised above — a build product, never committed) and the state under
`PRTS_DATA_DIR` (default `~/.prts-core`: sessions, providers, secrets, per-harness
homes). Provider tokens live in the OS secret store, so a token is re-entered once
and never lives in a file the repo tracks.

## What exists, and how each piece was verified

Every feature below was driven **by hand** through `/v1` when it was built, and the
observed facts are in the commit that shipped it. There is no test suite, on purpose
(see "How we work" below).

| | |
|---|---|
| the hub | `server.mjs` — one process, 53 routes, 17 stream events, no build step, loopback only, one hub per data dir |
| the contract | `contract/v1.json` (authority) + `contract/errors.json` (status + retryable) + `contract/adapter-v1.json` (stdio protocol) + `contract/openapi.json` (generated) |
| boot self-check | the hub refuses to start if its routes/events/error codes/OpenAPI/manifests disagree with the contract, and prints the differences |
| the harnesses | NOT in this repository: one harness = one plugin = one directory + manifest, each in its own repo (`<id>/manifest.json` + `<id>-adapter.cjs` + its own `extensions/`, `presets/`, `runtime/`). The hub searches a documented path for them at startup — `PRTS_PLUGINS_DIR` → `<hub>/plugins` → `<deployment>/plugins` (when the hub is at `<deployment>/apps/<name>`) → `<DATA_DIR>/plugins` (its own, where installs land) — printing every root; a client that sees an empty list gets the roots in a `note` |
| installing a harness | `POST /v1/hub/plugins {source:{url, ref?}}` clones it into the hub's own root; the plugin then materialises its own runtime (`runtime/prepare`, asked by the hub before its first session, and idempotent) — the hub knows no package names |
| sessions | create/read/patch/delete/close/reopen, per-knob `applied*` proofs, isolated harness homes, refusals (`session_busy`, `session_closed`, `needs_repair`, `unknown_session`, …) |
| turns | SSE stream with 17 events, cancel, repair, resume after a hub restart, native message id required |
| approvals and questions | both answered through `/v1`; a question is not an approval; deadline-denied dialogs are auto-cancelled rather than left hanging |
| plan / review | session-scoped knobs the harness owns; the hub follows the harness when it changes one by itself (`plan.changed`) |
| fork | a new session ending at a completed turn; the child's own process forks; the source is untouched |
| stats | read through from the harness after every turn (`session.stats`), absent ≠ zero |
| compact | `POST …/compact` plus the harness's own compactions reported on the turn stream (pi `threshold`/`overflow`, dsh `auto`) |
| rename | pushed into the harness; `title` vs `appliedTitle`; a name the harness gives itself is reported back |
| skills | installed by the hub into the harness's own directory, pointed at by the adapter, read back per session (`known:false` when a harness does not answer, never an empty list) |
| providers | one file per provider, token write-only in the secret store, declarations (modalities, window, thinking levels, cost), legacy single-file migration |
| model availability | for a hub-managed provider, the HUB answers (it holds the token): configured ⇒ available, on every harness (one rule, per plugin — not per harness) |
| isolation | no config scope; harness homes live under the data dir; the user's own `~/.agents/skills` and dsh credentials are neither read nor written (a marker skill in the user's directory never appeared in a session) |

## Open ends (recorded, not forgotten)

| | |
|---|---|
| the `501 unsupported` capability gate | never exercised: every installed harness declares the capability, so the refusal path has never run. It needs a stub plugin without one. |
| `tools` capability | declared in the adapter contract, declared by no plugin: `GET /v1/harnesses/{id}/tools` answers `known:false` for every harness — an empty catalogue is never faked. |
| artifacts | no producer: `GET /v1/sessions/{id}/artifacts` answers `[]`. |
| harness registry mutations, provider `models/refresh` | the routes answer and the boot check keeps them in the contract, but their semantics were checked once by hand, not driven since. |
| dsh's own model routes | `deepseek-official` reads `needs-auth` in a managed session because the user's own dsh credentials are deliberately not copied into the isolated home. If a session should use them, that is a decision to take, not a bug to fix. |
| jouzu's cold start | its first catalogue answer measured 9s once and 70s once (then 24ms): the hub bounds read-backs and says `known:false` rather than pretending. |
| dsh flakiness | one run's first dsh session did not get a port within 45s and answered 502; the next run was fine. Observed, not explained. |
| steering / queueing | `steer`, `follow_up`, `clear_queue` are deliberately not exposed: a second turn during a turn is `session_busy` by decision. |

## How we work on this

* **The contract is the authority and it is checked at boot.** Edit
  `contract/v1.json` (and `errors.json`), run `node scripts/emit-openapi.mjs`, and the
  hub will refuse to start until code and documents agree.
* **A contract change that can take something away from a consumer is not a
  cleanup.** Either the hub starts honouring the promise, or the removal is stated
  in the open with its reason. (This is written here because it was learned the hard
  way: a status response was missing `providers`/`connections`, and "fixing" it by
  deleting them from the contract was the wrong direction.)
* **No test suite.** Feature work is verified by driving the running hub through
  `/v1` by hand — the same surface a consumer uses — and the facts that came back
  (a status, an SSE event, a file on disk, a number the harness reported) go into the
  commit message. A suite of 29 model-driven scenarios was deleted for taking tens of
  minutes, needing an upstream model, and mostly re-verifying what a hand-run shows
  in a minute. A trial nobody reads is a trial nobody trusts.
* **Facts, not verdicts.** Descriptions and docs say what happened and what is
  unknown; a claim that cannot be read as a fact is a claim nobody can check. Where a
  mechanism is unverified, the docs say so.
* **The hub is the deliverable.** Tests, docs and tooling exist to make it correct
  and to make that correctness visible; when they stop doing that, they are removed.

## Where things live

```
server.mjs              the hub (sessions, routes, SSE, installs, boot self-check)
cli.mjs, secret-store.mjs, build-id.mjs, debate.mjs
contract/               v1.json · errors.json · adapter-v1.json · openapi.json
scripts/                emit-openapi.mjs · hub-connect.mjs · prepare-runtimes.mjs
docs/                   FEATURES.md · CLIENT.md · PROTOCOL.md · STATE.md (this file)
plugins/                empty here: harness plugins live in their own repositories and a
                        deployment points PRTS_PLUGINS_DIR at the directory holding them
```

`<PRTS_DATA_DIR>` (default `~/.prts-core`) holds everything the hub owns at
runtime: `endpoint.json` (port + token + pid + startedAt), `providers/`,
`secrets/`, `sessions.json`, `agents/<harness>/` (that harness's home, its installed
extensions and skills, its transcripts), `harnesses.json`.
