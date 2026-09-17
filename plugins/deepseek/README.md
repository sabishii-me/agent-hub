# deepseek plugin — DeepSeek Harness (`dsh`) over its own web protocol

Speaks the protocol dsh's own desktop/browser UI speaks: unary JSON-RPC over
HTTP (`POST /api/<method>`) plus a downlink WebSocket (`/api/events.mux`)
carrying the full internal session event stream. Token-level text deltas,
reasoning deltas, tool calls/results, approvals, question prompts, and real
cross-process session resume all ride this one surface. Nothing in the dsh
checkout is patched.

The adapter spawns one dedicated `dsh web --port 0` server per bus session
(the port is parsed from the server's stdout banner) and talks to it over
loopback only.

## Machine-local installation (one-time; none of it is runtime data)

1. Check out deepseek-harness and build it:

   ```
   git clone https://github.com/sabishii-me/deepseek-harness "%LOCALAPPDATA%\prts\agents\deepseek-harness"
   cd /d "%LOCALAPPDATA%\prts\agents\deepseek-harness"
   pnpm install
   pnpm run build
   ```

2. Record the checkout's absolute path in `dsh-root.txt` next to this README.

Credentials and the LLM route are **migrated from your existing dsh desktop
install** (`~/.dsh`) on every boot — no keys live in the source tree or this
plugin directory:

- `settings.yaml` → copied into the isolated home minus its `permission:`
  section, which is FORCED to `defaultPreset: workspace-write` so tool
  approvals ask. (The desktop default is often `danger-full-access`; silently
  inheriting it would turn every write into a no-prompt action.)
- `.credentials.yaml` → synced when present.

## Isolated DSH_HOME

All runtime state lives under `<PRTS_AGENT_DATA_DIR>/dsh-home` — sessions,
persisted history, credentials — never next to plugin code and never mixed
into the user's desktop dsh session list. Because dsh persists sessions, the
bus `ref` is a REAL resume handle: a new adapter process reattaches to the
full history. An unknown ref fails loudly (dsh's `session.create` would
otherwise accept any id as create-or-attach — the adapter checks
`session.list` first).

## Semantics worth knowing

- `session.prompt` answers when the turn settles, not when admitted (the
  core's serial prompt rhythm relies on that; `mode: "queue"` queues).
- `turn_end.status`: `completed` (completed/max-tokens), `failed`
  (error/blocked), `aborted` (aborted/interrupted).
- Approvals fire on wider-retry writes (outside the workspace) and on shell
  commands the sandbox escalates; plain in-workspace reads/writes execute
  directly — approval policy is the agent's own, the shell only carries it.
- `ask_user_question` frames (question/requested) are outside the v0 event
  contract. Following the pi adapter's rule — an agent must never hang on a
  UI nobody renders — the adapter cancels the host's pending question
  (client-response `{ok:false, error:{code:"cancelled", details:{}}}`;
  dsh then fails the tool call with ASK_CANCELLED) and surfaces an
  `adapter_dialog_auto_cancelled` event. Note dsh's rpc error schema REQUIRES
  the `details` field — a bare `{code,message}` is rejected with HTTP 400.
