# 20261007-181935 - the deepseek harness runtime cannot be prepared (npm ERESOLVE)

**State:** recorded (found while measuring real installs). Owner: the deepseek harness bundle, not the
hub.

## Observed (real, dev)

Install `deepseek` from the registry; now that install really prepares the runtime
(docs/issues/20261007-200000), the failure surfaces honestly:

```
GET /v1/plugins/deepseek -> state "failed", runtimeReady false,
  detail: "runtime prepare failed: the adapter closed its stdout"
```

The hub log shows the adapter's runtime prepare running npm, which fails:

```
npm error ERESOLVE unable to resolve dependency tree
npm error While resolving: jouzu@0.1.18
npm error Found: @earendil-works/pi-coding-agent@0.87.1
npm error Could not resolve dependency:
npm error peer @earendil-works/pi-coding-agent@^0.83.0 || ^0.84.0 || ^0.85.0 from pi-webaio@1.0.5
```

## Why it matters

The hub now reports the truth (`failed` + a reason) instead of a fake `ready`. But a plugin that
cannot prepare its runtime is not installable, so `deepseek` is unusable until its bundle pins a
compatible `@earendil-works/pi-coding-agent` (or the peer range of `pi-webaio` is widened). This is
a DEFECT IN THE PLUGIN BUNDLE, exposed by the hub doing a real install.

## Impact on the measurement

The install/remove timing report cannot install all three plugins concurrently until deepseek is
fixable; the report must still measure (and show) deepseek's honest failure rather than break.
