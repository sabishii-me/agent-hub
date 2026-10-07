# 20261005-160000 — the Python suite could report PASS while checks FAILED (exit code discarded; a stale release binary)

## Status
PARTIALLY FIXED. The exit-code hole and the stale-binary measurement are fixed. The suite's
numbers are NOT yet fully trustworthy: the runner's PASS/FAIL/BLOCKED classification itself had
holes (fixed in a later commit), several test designs were falsified (listed in the convergence
report's item four), and most capabilities are BLOCKED. Do not read the numbers as 'trustworthy';
read them as 'no longer silently wrong in these specific ways'.

## Two distortions found while converging (owner's item 4)
1. **A failing assertion still exited 0.** Five files ended with a bare `t.done()` whose return
   value was discarded, and `tests/run.py` counts files by EXIT CODE. So a FAIL in
   a-caller-cannot-install-from-an-arbitrary-url.py, install-reconciles-against-the-registry.py,
   registry-refresh-updates-the-local-registry.py, install-refusals-have-honest-codes.py and
   the-release-hub-error-surface.py was invisible to the runner. FIXED: each ties its result to
   `sys.exit(0 if ok else 1)`. Re-measured with real exit codes.
2. **A stale release binary was measured.** target/release/agent-hub.exe was built BEFORE the
   registry-reconcile change (it contained 0 occurrences of `plugin_not_in_registry`). Testing it
   produced "the release hub still installs an arbitrary git source" - a FALSE statement about the
   product, an artifact of the stale binary. FIXED by rebuilding release; it now refuses with 403.
   Lesson recorded: a test must ensure the binary under test matches the source.

## Also corrected by this convergence (owner's items 1-3)
- install-reconciles-against-the-registry.py asserted "no registry -> refused" on a bare `Hub()`,
  which the harness had ALREADY given a registry: the failure-injection did not take effect. Now
  the no-registry case explicitly clears AGENT_HUB_REGISTRY_FILE. (A test that does not induce the
  condition it claims to test.)
- tests/lib/tally.py gained an explicit BLOCKED state (separate from pass/fail/skip) so a capability
  missing a real precondition is recorded as UNVERIFIED, never as a pass.

## Not fixed here (owner direction: registry stays unpublished)
Every capability that needs the published registry is BLOCKED, not passed - see
docs/tasks/20261005-convergence-report.md.
