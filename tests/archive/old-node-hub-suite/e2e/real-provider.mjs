// REAL end-to-end: the Rust hub, the REAL pi adapter, the REAL pi runtime, and the
// REAL provider from ~/.pi/agent/models.json (HOME-JP-prod). NOTHING is mocked.
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
const PROVIDER_NAME = process.env.PI_PROVIDER || 'HOME-JP-prod';
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function loadRealProvider() {
  const f = path.join(os.homedir(), '.pi', 'agent', 'models.json');
  const cfg = JSON.parse(fs.readFileSync(f, 'utf8'));
  const p = cfg.providers[PROVIDER_NAME];
  if (!p) throw new Error(`no provider '${PROVIDER_NAME}' in ${f} (have: ${Object.keys(cfg.providers)})`);
  if (!p.baseUrl || !p.apiKey) throw new Error(`provider '${PROVIDER_NAME}' is missing baseUrl/apiKey`);
  return p;
}
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
  let json = null; try { json = JSON.parse(text); } catch { /* leave null */ }
  return { status: r.status, json, text, headers: r.headers };
}

const provider = loadRealProvider();
const data = fs.mkdtempSync(path.join(os.tmpdir(), 'hub-real-'));
const plugins = path.join(data, 'plugins');
fs.mkdirSync(plugins, { recursive: true });
copyTree(PLUGIN_SRC, path.join(plugins, HARNESS));
console.log(`[e2e] data dir : ${data}`);
console.log(`[e2e] provider : ${PROVIDER_NAME} ${provider.baseUrl} api=${provider.api} keylen=${provider.apiKey.length}`);

const log = fs.openSync(path.join(data, 'hub.log'), 'a');
const child = spawn(HUB, [], { env: { ...process.env, AGENT_HUB_DATA_DIR: data, AGENT_HUB_ADDR: '127.0.0.1:0', AGENT_HUB_CONTRACT_DIR: path.join(REPO, 'contract') }, stdio: ['ignore', log, log], windowsHide: true });

let ep = null;
for (let i = 0; i < 200; i++) {
  try { const e = JSON.parse(fs.readFileSync(path.join(data, 'endpoint.json'), 'utf8')); if (e.pid === child.pid) { ep = e; break; } } catch { /* not yet */ }
  await sleep(100);
}
if (!ep) { console.error('[e2e] hub did not write endpoint.json'); process.exit(2); }
console.log(`[e2e] endpoint.json: ${JSON.stringify(ep)}`);
const base = ep.url || `http://${ep.host}:${ep.port}`;
const token = ep.token;
console.log(`[e2e] hub pid ${child.pid} at ${base}`);

try {
  let r = await j('POST', `${base}/v1/model-providers`, token, { id: 'jp', url: provider.baseUrl, api: provider.api, token: provider.apiKey });
  console.log(`[e2e] POST /v1/model-providers -> ${r.status} ${r.status < 300 ? 'ok' : r.text.slice(0, 300)}`);

  const model = process.env.PI_MODEL || 'deepseek-flash';
  r = await j('POST', `${base}/v1/sessions`, token, { harnessId: HARNESS, idempotencyKey: 'e2e-1', modelProviderId: 'jp', modelId: model }, 'e2e-1');
  console.log(`[e2e] POST /v1/sessions -> ${r.status}`);
  if (r.status !== 202) { console.error(r.text.slice(0, 400)); process.exit(3); }
  const sid = r.json.session.id;

  let st = 'starting';
  for (let i = 0; i < 240; i++) {
    const g = await j('GET', `${base}/v1/sessions/${sid}`, token);
    st = g.json?.session?.status;
    if (st === 'active' || st === 'failed' || st === 'needs-repair') break;
    await sleep(250);
  }
  console.log(`[e2e] session ${sid} status=${st}`);
  if (st !== 'active') {
    const g = await j('GET', `${base}/v1/sessions/${sid}`, token);
    console.error('[e2e] startError:', JSON.stringify(g.json?.session?.startError));
    process.exit(4);
  }

  const prompt = process.env.PI_PROMPT || 'Reply with exactly the word: pong';
  r = await j('POST', `${base}/v1/sessions/${sid}/turns`, token, { content: [{ type: 'text', text: prompt }], idempotencyKey: 'turn-1' }, 'turn-1');
  console.log(`[e2e] POST /turns -> ${r.status} ${r.status < 300 ? '(accepted)' : r.text.slice(0, 300)}`);
  if (r.status >= 300) process.exit(5);

  let answer = null;
  for (let i = 0; i < 600; i++) {
    const g = await j('GET', `${base}/v1/sessions/${sid}/messages`, token);
    const msgs = g.json?.messages || [];
    const asst = msgs.filter((m) => m.role === 'assistant' && (m.content || []).some((c) => c.type === 'text' && c.text));
    if (asst.length) { answer = asst[asst.length - 1]; break; }
    await sleep(500);
  }
  if (!answer) { console.error('[e2e] no assistant answer'); process.exit(6); }
  const text = (answer.content || []).map((c) => c.text || '').join('');
  console.log('\n[e2e] ===== REAL MODEL ANSWER =====');
  console.log(text.trim().slice(0, 800));
  console.log('[e2e] ===============================');
  console.log(`[e2e] model=${answer.model || '?'} assistantId=${answer.id}`);
  console.log('[e2e] PASS: a real model answered through the hub + real pi adapter');
  // Remove the provider we created: a DELETE drops the row AND its keychain
  // credential, so a run leaves no credential behind.
  r = await j('DELETE', `${base}/v1/model-providers/jp`, token);
  console.log(`[e2e] cleanup: DELETE /v1/model-providers/jp -> ${r.status}`);
} finally {
  try { child.kill(); } catch { /* ignore */ }
  console.log(`[e2e] data dir: ${data}`);
}
