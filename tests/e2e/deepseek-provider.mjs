// REAL end-to-end for a MODEL-PROVIDER PLUGIN: the hub reads the plugin's
// provider.json DESCRIPTOR (data) and owns the HTTP/catalog; the person supplies
// only the key. Chain: Rust hub -> real pi adapter -> real pi runtime -> the
// provider type's OWN endpoint (https://api.deepseek.com), model `deepseek-chat`
// (the "flash" tier). NOTHING is mocked.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const REPO = path.resolve(HERE, '..', '..');
const HUB = path.join(REPO, 'target', 'debug', process.platform === 'win32' ? 'agent-hub.exe' : 'agent-hub');
const PLUGIN_SRC = process.env.PI_PLUGIN_DIR || 'E:/AI/ideas/prts-harness-pi';
const HARNESS = process.env.PI_HARNESS_ID || 'pi';
const PROVIDER_PLUGIN = process.env.PROVIDER_PLUGIN_DIR || 'E:/AI/ideas/prts-providers/deepseek';
const PROVIDER_TYPE = process.env.PROVIDER_TYPE_ID || 'deepseek';
const KEY = process.env.DEEPSEEK_KEY || 'sk-1ea3012ec8d84d91ae038149f94cfe2e';
const MODEL = process.env.PI_MODEL || 'deepseek-chat';
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function copyTree(src, dest) {
  fs.mkdirSync(dest, { recursive: true });
  for (const e of fs.readdirSync(src, { withFileTypes: true })) {
    const s = path.join(src, e.name), d = path.join(dest, e.name);
    if (e.isDirectory()) copyTree(s, d);
    else if (e.isFile()) fs.copyFileSync(s, d);
  }
}
async function j(method, url, token, body, key) {
  const headers = { authorization: `Bearer ${token}`, 'content-type': 'application/json' };
  if (key) headers['idempotency-key'] = key;
  const r = await fetch(url, { method, headers, body: body ? JSON.stringify(body) : undefined });
  const text = await r.text();
  let json = null; try { json = JSON.parse(text); } catch { /* null */ }
  return { status: r.status, json, text, headers: r.headers };
}

const data = fs.mkdtempSync(path.join(os.tmpdir(), 'hub-ds-'));
const plugins = path.join(data, 'plugins');
fs.mkdirSync(plugins, { recursive: true });
copyTree(PLUGIN_SRC, path.join(plugins, HARNESS));
copyTree(PROVIDER_PLUGIN, path.join(plugins, PROVIDER_TYPE));
console.log(`[e2e] data dir : ${data}`);
console.log(`[e2e] provider type: ${PROVIDER_TYPE} (from ${PROVIDER_PLUGIN}/provider.json)`);

const log = fs.openSync(path.join(data, 'hub.log'), 'a');
const child = spawn(HUB, [], { env: { ...process.env, AGENT_HUB_DATA_DIR: data, AGENT_HUB_ADDR: '127.0.0.1:0', AGENT_HUB_CONTRACT_DIR: path.join(REPO, 'contract') }, stdio: ['ignore', log, log], windowsHide: true });
let ep = null;
for (let i = 0; i < 200; i++) { try { const e = JSON.parse(fs.readFileSync(path.join(data, 'endpoint.json'), 'utf8')); if (e.pid === child.pid) { ep = e; break; } } catch {} await sleep(100); }
if (!ep) { console.error('[e2e] no endpoint.json'); process.exit(2); }
const base = ep.url, token = ep.token;

try {
  // The type is DATA the hub read from the plugin's provider.json.
  let r = await j('GET', `${base}/v1/model-providers/types`, token);
  const types = (r.json?.types || []).map((t) => `${t.id}@${t.version}(${t.owner})`);
  console.log(`[e2e] GET /types -> ${r.status} [${types.join(', ')}] broken=${JSON.stringify(r.json?.broken)}`);
  if (!types.some((t) => t.startsWith(`${PROVIDER_TYPE}@`))) { console.error(`[e2e] type ${PROVIDER_TYPE} not shipped by the plugin`); process.exit(3); }

  // The descriptor's fixed endpoint is the plugin's fact; the person supplies the key.
  let desc = (r.json.types || []).find((t) => t.id === PROVIDER_TYPE);
  console.log(`[e2e] descriptor endpoint: ${JSON.stringify(desc.configuration.endpoint)}`);

  // Create a provider of that TYPE - no url/api (the type owns them), just the key.
  r = await j('POST', `${base}/v1/model-providers`, token, { id: 'ds', token: KEY, providerType: PROVIDER_TYPE, providerTypeVersion: 1 });
  console.log(`[e2e] POST /v1/model-providers -> ${r.status} ${r.status < 300 ? JSON.stringify({ url: r.json.provider.url, api: r.json.provider.api, type: r.json.provider.providerType, available: r.json.provider.providerTypeAvailable }) : r.text.slice(0, 300)}`);
  if (r.status >= 300) { process.exit(4); }

  // Refresh the catalog from the type's endpoint (the hub owns the HTTP).
  r = await j('POST', `${base}/v1/model-providers/ds/models/refresh`, token);
  console.log(`[e2e] POST /models/refresh -> ${r.status} ${r.status < 300 ? JSON.stringify(r.json).slice(0, 200) : r.text.slice(0, 200)}`);

  r = await j('GET', `${base}/v1/model-providers/ds/models`, token);
  const models = (r.json?.models || []).map((m) => m.id || m);
  console.log(`[e2e] GET /models -> ${r.status} count=${models.length} sample=${models.slice(0, 6).join(', ')}`);

  r = await j('POST', `${base}/v1/sessions`, token, { harnessId: HARNESS, idempotencyKey: 'ds-1', modelProviderId: 'ds', modelId: MODEL }, 'ds-1');
  console.log(`[e2e] POST /v1/sessions -> ${r.status}`);
  if (r.status !== 202) { console.error(r.text.slice(0, 400)); process.exit(5); }
  const sid = r.json.session.id;

  let st = 'starting';
  for (let i = 0; i < 480; i++) { const g = await j('GET', `${base}/v1/sessions/${sid}`, token); st = g.json?.session?.status; if (st === 'active' || st === 'failed' || st === 'needs-repair' || st === 'starting_failed') break; await sleep(250); }
  console.log(`[e2e] session ${sid} status=${st}`);
  if (st !== 'active') { const g = await j('GET', `${base}/v1/sessions/${sid}`, token); console.error('[e2e] startError:', JSON.stringify(g.json?.session?.startError)); process.exit(6); }

  r = await j('POST', `${base}/v1/sessions/${sid}/turns`, token, { content: [{ type: 'text', text: 'Reply with exactly the word: pong' }], idempotencyKey: 'ds-turn-1' }, 'ds-turn-1');
  console.log(`[e2e] POST /turns -> ${r.status} ${r.status < 300 ? '(accepted)' : r.text.slice(0, 300)}`);
  if (r.status >= 300) process.exit(7);

  let answer = null;
  for (let i = 0; i < 600; i++) {
    const g = await j('GET', `${base}/v1/sessions/${sid}/messages`, token);
    const asst = (g.json?.messages || []).filter((m) => m.role === 'assistant' && (m.content || []).some((c) => c.type === 'text' && c.text));
    if (asst.length) { answer = asst[asst.length - 1]; break; }
    await sleep(500);
  }
  if (!answer) { console.error('[e2e] no assistant answer'); process.exit(8); }
  const text = (answer.content || []).map((c) => c.text || '').join('');
  console.log('\n[e2e] ===== REAL MODEL ANSWER (deepseek provider) =====');
  console.log(text.trim().slice(0, 800));
  console.log('[e2e] ==================================================');
  console.log(`[e2e] assistantId=${answer.id}`);
  console.log('[e2e] PASS: a real deepseek model answered via the provider PLUGIN descriptor');
} finally {
  try { await j('DELETE', `${base}/v1/model-providers/ds`, token); } catch {}
  try { child.kill(); } catch {}
  console.log(`[e2e] data dir: ${data}`);
}
