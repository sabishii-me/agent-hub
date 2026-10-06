# EVERY test, EVERY plugin, EVERY line — real vs mock (2026-10-05)

UPDATE (2026-10-05): the `plugins_src` COPY mock is GONE from `tests/lib/hub.py`; it now installs
the REAL registry artifact (url+sha256+size) and runs `prepare`. Found and fixed a real bug in the
process (runtimeReady hardcoded false, 20261005-090000). The table below records the state BEFORE
this change (the finding). No test runs the REAL
install path (a git URL, or a verified release artifact + `prepare`), and NONE calls
`prepare` — so the runtime is never materialised the way the hub actually does it.
A REAL artifact install of pi reaches `ready` but a session FAILS:
`starting_failed: the adapter closed its stdout` — the zip is adapter-only; the runtime
must be prepared. The mock hid this.

| test | plugin setup | prepare | provider | session | verdict |
|---|---|---|---|---|---|
| tests\approvals\an-approval-preset-asks-before-every-tool.py | MOCK copy (local path) | - | yes | yes | MOCK |
| tests\approvals\denying-an-approval-blocks-the-side-effect.py | MOCK copy (local path) | - | yes | yes | MOCK |
| tests\approvals\review-remains-switchable-after-a-preset.py | MOCK copy (local path) | - | yes | yes | MOCK |
| tests\concurrency\a-second-turn-while-one-runs-is-refused.py | MOCK copy (local path) | - | yes | yes | MOCK |
| tests\concurrency\cancelling-one-session-does-not-disturb-another.py | MOCK copy (local path) | - | yes | yes | MOCK |
| tests\concurrency\many-sessions-cancel-at-once.py | MOCK copy (local path) | - | yes | yes | MOCK |
| tests\concurrency\many-sessions-of-one-harness.py | MOCK copy (local path) | - | yes | yes | MOCK |
| tests\concurrency\one-hundred-connections-do-not-time-out.py | n/a | - | - | - | no plugin |
| tests\connections\connections-are-crud-and-delete-is-real.py | n/a | - | - | - | no plugin |
| tests\contract\status-surface-openapi-and-models-answer.py | n/a | - | - | - | no plugin |
| tests\contract\the-events-stream-delivers-plugin-changes.py | v1 install | - | - | - | REAL (v1 install) |
| tests\contract\the-served-surface-equals-the-contract.py | n/a | - | - | - | no plugin |
| tests\harnesses\an-uninstalled-harness-cannot-be-used.py | v1 install | - | - | yes | REAL (v1 install) |
| tests\harnesses\harness-discovery-and-its-capability-gated-routes.py | MOCK copy (local path) | - | - | - | MOCK |
| tests\interrupt\a-cancelled-turn-releases-the-session.py | MOCK copy (local path) | - | yes | yes | MOCK |
| tests\interrupt\a-killed-hub-recovers-its-sessions.py | MOCK copy (local path) | - | - | yes | MOCK |
| tests\interrupt\a-provider-that-cannot-be-reached-is-refused-not-faked.py | MOCK copy (local path) | - | - | yes | MOCK |
| tests\interrupt\a-provider-that-hangs-does-not-hang-the-turn.py | MOCK copy (local path) | - | - | yes | MOCK |
| tests\interrupt\cancel-is-idempotent-and-bounded.py | MOCK copy (local path) | - | - | yes | MOCK |
| tests\interrupt\killing-the-adapter-during-a-cancel-settles-the-turn.py | MOCK copy (local path) | - | yes | yes | MOCK |
| tests\interrupt\killing-the-adapter-leaves-an-honest-state.py | MOCK copy (local path) | - | - | yes | MOCK |
| tests\interrupt\killing-the-adapter-mid-turn-does-not-hang-the-turn.py | MOCK copy (local path) | - | yes | yes | MOCK |
| tests\interrupt\killing-the-runtime-process-settles-the-turn.py | MOCK copy (local path) | - | yes | yes | MOCK |
| tests\interrupt\killing-the-whole-tree-then-restarting-recovers.py | MOCK copy (local path) | - | yes | yes | MOCK |
| tests\interrupt\send-immediately-after-cancel.py | MOCK copy (local path) | - | yes | yes | MOCK |
| tests\interrupt\the-network-drops-mid-turn-and-the-turn-settles.py | MOCK copy (local path) | - | yes | yes | MOCK |
| tests\lib\__init__.py | n/a | - | - | - | no plugin |
| tests\lib\hub.py | MOCK copy (local path) | - | yes | yes | MOCK |
| tests\lib\tally.py | n/a | - | - | - | no plugin |
| tests\lib\tcpfwd.py | n/a | - | - | - | no plugin |
| tests\lifecycle\a-session-closes-reopens-and-forks-without-touching-its-source.py | MOCK copy (local path) | - | - | yes | MOCK |
| tests\lifecycle\the-session-crud-and-read-through-are-real.py | MOCK copy (local path) | - | - | yes | MOCK |
| tests\lifecycle\the-whole-chain-plugin-to-session-to-tools-over-v1.py | v1 install | - | yes | yes | REAL (v1 install) |
| tests\model\a-real-model-answers-with-a-confirmed-identity.py | MOCK copy (local path) | - | yes | yes | MOCK |
| tests\plugins\a-hub-killed-mid-remove-finishes-the-removal.py | v1 install | - | - | - | REAL (v1 install) |
| tests\plugins\a-plugin-is-installed-listed-enabled-and-removed.py | v1 install | yes | - | - | REAL (v1 install+prepare) |
| tests\plugins\concurrent-install-and-remove.py | v1 install | - | - | - | REAL (v1 install) |
| tests\plugins\concurrent-mixed-install-remove.py | v1 install | - | - | - | REAL (v1 install) |
| tests\presets\a-preset-is-listed-and-applied.py | MOCK copy (local path) | - | yes | yes | MOCK |
| tests\presets\plan-mode-is-applied-and-reports.py | MOCK copy (local path) | - | yes | yes | MOCK |
| tests\provider\model-provider-crud-and-types-are-real.py | MOCK copy (local path) | - | - | - | MOCK |
| tests\skills\skills-crud-over-the-real-surface.py | n/a | - | - | - | no plugin |
| tests\tools\the-model-really-runs-a-tool.py | MOCK copy (local path) | - | yes | yes | MOCK |
