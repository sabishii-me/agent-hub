# Real-adapter wire findings (static, against committed SHAs)

Read from the REAL adapters' committed sources (not mocks). SHAs: pi
`23ae330d2beb5d567b556a358117c3f7c768a027`, jouzu
`453eff2ea06d05438320b1d30eeac18f74a498d2`, deepseek
`580aca8984978cead650c2389aa189a4313e96ad`. These paths were present locally.

## approval_need (G1) — the hub's reply MUST be `{approved, reason}`

All three send a **JSON-RPC REQUEST** (id `appr-<rpcId>`), so it needs an answer:

- pi `pi-adapter.cjs:569`; jouzu `jouzu-adapter.cjs:680`; deepseek
  `deepseek-adapter.cjs:575`.
- Params: `{sid, kind:'confirm', detail, approval_id?, respond_rpc_id?, options?}`.
  pi/jouzu may carry `tool`/`args` instead of `detail`.
- The reply is read from **`msg.result`** and the resolver uses
  **`ans.approved === true`** (pi:564, jouzu:675, deepseek:1441) and
  **`ans.reason`** from a CLOSED vocabulary `timeout|allowed|denied`
  (pi:564, jouzu:675). pi keys its pending map by **`id.slice(5)`** (strip `appr-`),
  so the hub MUST echo the adapter's exact id string.

**Hub change (`_real.py`):** read both param shapes (`tool`/`args` OR `detail`) and
reply **`{approved, reason:'allowed'|'denied'}`** (was `{approved, decision, reason:null}`,
which the real `reason` vocabulary does not accept). A withdrawn approval replies
`{approved:false, reason:'denied'}`.

## question_need — the reply is `{answers:[{id?,selected,custom?}]}`

deepseek `deepseek-adapter.cjs:600`: `{sid, question_id, questions:[{id,header,question,
options:[{label}],multiSelect,intent}]}`. pi/jouzu read `msg.result.answers[0].selected[0]`
(jouzu:699). The `/v1` answer body is ALREADY `{answers:[{id,selected,custom}]}` (contract
`/v1/sessions/{id}/questions/{qid}`), so the hub's `{answers: resolved.answers}` matches.

## connections/delete (G2) — the parameter is `id`

deepseek `deepseek-adapter.cjs:1308`: `rows.find((r) => r.id === p.id)`. Sending
`connectionId` misses and the adapter idempotently returns `{}` (a false success). The
contract `adapter-v1.json` `connections/delete` params are `[id, expectedRevision?]`. The
hub now sends `{id}` — CONFIRMED against the real source.

## Still NOT run

A real end-to-end run needs the hub to START, which **unconditionally probes the real OS
keychain** (`crates/secrets/src/lib.rs:probe_store` writes/reads/deletes a random
`__probe__<hex>` entry). That side effect is NOT authorized, and the handoff forbids a test
switch/backend swap to dodge it. So no real pi/jouzu/dsh ACCEPTANCE has been run; the
findings above are STATIC wire comparisons, not runtime proof.
