# 20261008-005306 - the deepseek adapter `die()`s at startup, so `runtime/prepare` can never run

**State:** recorded (a DEEPSEEK plugin defect; the hub is correct). Owner: the deepseek adapter.
**Severity:** the deepseek plugin is NOT installable from a release zip at all.

## Measured

Installing `deepseek` from its repo's local build with a real hub:

```
POST /v1/plugins -> 202
GET  /v1/plugins/deepseek -> "failed" in ~1.3s
  detail: "runtime prepare failed: the adapter closed its stdout"
```

The hub log shows the REAL reason:

```
deepseek-adapter: dsh runtime not found: <data>/plugins/deepseek/runtime/lib/bin.js
                  (the plugin's runtime is missing)
adapter stdout closed (reader ending)
```

## Cause

`deepseek-adapter.cjs` line 97, at TOP LEVEL (before any method is handled):

```js
const DSH_BIN = DSH_RUNTIME[DSH_RUNTIME.length - 1];
if (!fs.existsSync(DSH_BIN)) die(`dsh runtime not found: ${DSH_BIN} (the plugin's runtime is missing)`);
```

The adapter requires the runtime to ALREADY be on disk and exits if it is not. But installing the
runtime is `runtime/prepare` (line ~1928: `npm pack <pkg>@<version>` then unpack), which the hub
calls precisely when the runtime is ABSENT. So the adapter kills itself before the hub can ask it to
prepare - `runtime/prepare` is UNREACHABLE for a fresh install. `npm` never runs (hence the ~1.3s).

## Not a version problem

Changing the pin `@deepseek-ai/dsh@0.1.0-rc.7` -> the latest `0.2.0-rc.2` changes nothing: the
adapter still dies at line 97 before any fetch. (dsh 0.1.0-rc.7 IS old - npm latest is 0.2.0-rc.2 -
but that is a separate matter.)

## Fix direction (plugin repo)

The adapter must NOT require the runtime at startup. It should answer `runtime/prepare` (install the
pinned runtime) and only fail when the runtime is genuinely needed (e.g. at `session/start`). The pi
repo's adapter is the working reference: it prepares on demand and does not die if the runtime is
absent.
