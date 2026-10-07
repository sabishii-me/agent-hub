# FIX PROPOSAL — a keychain target name that fits the OS limit (20261004-080000)

Owner review REQUIRED before any code/doc change. This changes what ARCHITECTURE §19 states
literally (`:717` "The secret store service is `agent-hub:<instance-id>`").

## What is broken (proven, isolated)

The OS keychain (Windows, `keyring` v3 `windows-native`) has HARD field limits, measured on
this host:

- **target name (the `service`) ≤ 29 chars**; 30+ -> `set_password` fails with Windows error 8.
- **user name (the `key`) ≤ 16 chars**; 17+ -> same failure.
  (`svc_len=29 ok=true`, `svc_len=30 ok=false`; `key_len=16 ok=true`, `key_len=17 ok=false`;
  each field's own limit — `svc=20+key=9` fails while `svc=29+key=1` works.)

The hub builds BOTH fields longer than that:

- `SecretStore::for_instance` -> service `format!("agent-hub:{instance}")` where the instance id
  is 32 hex chars (`crates/db/src/instance.rs`) => **42 chars** (`crates/secrets/src/lib.rs:34`).
- `Providers::secret_ref` / `Connections::secret_ref` -> key `format!("{namespace}:provider-{id}")`
  / `"{namespace}:connection-{id}"` where namespace is the same 32-hex id => **>= 42 chars**
  (`crates/providers/src/service.rs:164`, `crates/connections/src/service.rs:106`).

So the store's **boot probe** (`agent-hub:<42>`) fails, the store is marked UNAVAILABLE for the
whole hub, and every credential write is refused: `POST /v1/model-providers` -> `501 no secret
store`. This is the CURRENT reason all real-provider tests cannot run. It is a hub defect: the
fields are the hub's own construction, and the lengths are incompatible with the OS it claims
to use (`keyring` is a §8 platform choice).

## Why this needs an owner decision (not a silent patch)

ARCHITECTURE §19 (`:717`) fixes the service as `agent-hub:<instance-id>` and §provider
(`:659`) fixes the key as `<instance>:provider-<id>`. Both literally exceed the OS limit, so the
documents as written cannot be implemented on Windows. Changing the keychain naming is a
**design-adjacent** change (a document states the format; a migrated instance must still find
its credentials), so per the rule the OWNER decides the direction before it is written.

## The single correct fix (derived from the constraints, not invented)

Keep the design's INTENT (a namespace keyed by the persistent, path-independent instance id,
so a moved data dir keeps its credentials and two dirs never collide) and make the two FIELDS
fit:

1. **service** = `"agent-hub"` — a fixed 9-char constant. This is exactly what the module's own
   doc already says ("each secret is one keychain entry under the service `agent-hub`"
   `crates/secrets/src/lib.rs:4`). Instances are separated by the KEY, not the service.
2. **key** = a deterministic, 16-hex-char token derived from the (instance, kind, id) triple:
   `hex(sha256(instance || 0x1f || kind || 0x1f || id))[..16]`. 64 bits of collision space;
   deterministic, so the same triple always yields the same key; stable across restarts and a
   moved directory (it depends on the persisted instance id, not a path).
3. **Migration (read-old-then-write-new)**: on read, first try the new key; if absent, try the
   LEGACY `(service = "agent-hub:<instance>", key = "<instance>:<kind>-<id>")`; on success,
   copy to the new key and delete the legacy entry. On hosts where the store worked with the
   old (long) name, existing credentials migrate on first use instead of being orphaned. On a
   host where the old name never worked (like this one), there is nothing to migrate.

This keeps §19's intent (persistent instance identity, path independence, no collisions) intact
while making the two keychain fields fit the OS. The instance id stays the single source; only
its ENCODING into the keychain changes.

## Not the alternative

- Do NOT drop the instance namespace (it is the design: two dirs must not collide).
- Do NOT fall back to a plaintext/file store (the design forbids it; a credential must never be
  a file).
- Do NOT truncate the raw id (a truncated id can collide across instances - the hash is the
  collision-resistant short form).

## Verify (when implemented)

- `cargo test -p agent-hub-secrets -- --nocapture` no longer prints
  `SKIP: OS secret store unavailable`; the probe succeeds.
- `POST /v1/model-providers` -> 2xx; `GET` reports `tokenConfigured` true; logout clears it.
- Real suite: `tests/model/*`, `tests/approvals/*` and the full run (28/28 on pi).
- A moved-data-dir test: same instance id in a new path still reads the credential.
