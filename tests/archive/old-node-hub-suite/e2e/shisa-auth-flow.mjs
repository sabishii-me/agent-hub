// REAL device-code sign-in OWNED BY THE HUB (ADR-0012): POST /v1/model-providers/{id}/auth
// returns the step; the human opens verifyUrl and enters userCode; the HUB polls the
// platform and stores the credential; then a session+turn run against the real API.
// NOTHING is mocked and NOTHING is done out of band.
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
const PROVIDER_PLUGIN = process.env.PROVIDER_PLUGIN_DIR || 'E:/AI/ideas/prts-providers/shisa';
const PROVIDER_TYPE = process.env.PROVIDER_TYPE_ID || 'shisa';
const MODEL = process.env.PI_MODEL || 'qwen3.7-flash';
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
function copyTree(src, dest) { fs.mkdirSync(dest, { recursive: true }); for (const e of fs.readdirSync(src, { withFileTypes: true })) { const s = path.join(src, e.name), d = path.join(dest, e.name); if (e.isDirectory()) copyTree(s, d); else if (e.isFile()) fs.copyFileSync(s, d); } }
async function j(method, url, token, body, key) { const headers = { authorization: `Bearer ${token}`, 'content-type': 'application/json' }; if (key) headers['idempotency-key'] = key; const r = await fetch(url, { method, headers, body: body ? JSON.stringify(body) : undefined }); const text = await r.text(); let json = null; try { json = JSON.parse(text); } catch {} return { status: r.status, json, text, headers: r.headers }; }

const data = fs.mkdtempSync(path.join(os.tmpdir(), 'hub-shisaauth-'));
const plugins = path.join(data, 'plugins');
fs.mkdirSync(plugins, { recursive: true });
copyTree(PLUGIN_SRC, path.join(plugins, HARNESS));
copyTree(PROVIDER_PLUGIN, path.join(plugins, PROVIDER_TYPE));
console.log(`[e2e] data dir: ${data}`);
const log = fs.openSync(path.join(data, 'hub.log'), 'a');
const child = spawn(HUB, [], { env: { ...process.env, AGENT_HUB_DATA_DIR: data, AGENT_HUB_ADDR: '127.0.0.1:0', AGENT_HUB_CONTRACT_DIR: path.join(REPO, 'contract') }, stdio: ['ignore', log, log], windowsHide: true });
let ep = null; for (let i = 0; i < 200; i++) { try { const e = JSON.parse(fs.readFileSync(path.join(data, 'endpoint.json'), 'utf8')); if (e.pid === child.pid) { ep = e; break; } } catch {} await sleep(100); }
if (!ep) { console.error('no endpoint.json'); process.exit(2); }
const base = ep.url, token = ep.token;

try {
  // A provider of the shisa TYPE, with NO credential: it is created by signing in.
  let r = await j('POST', `${base}/v1/model-providers`, token, { id: 'sh', providerType: PROVIDER_TYPE, providerTypeVersion: 1 });
  console.log(`[e2e] POST /v1/model-providers (no token) -> ${r.status} ${r.status < 300 ? '' : r.text.slice(0, 200)}`);

  // Start the hub-owned authorization. The hub runs the flow; it returns the step.
  r = await j('POST', `${base}/v1/model-providers/sh/auth`, token);
  console.log(`[e2e] POST /auth -> ${r.status} location=${r.headers.get('location')}`);
  if (r.status !== 202) { console.error(r.text.slice(0, 400)); process.exit(3); }
  const opId = r.json.operationId, next = r.json.next;
  console.log('\n[e2e] ================= SIGN IN =================');
  console.log(`[e2e]   OPEN : ${next.verifyUrl}`);
  console.log(`[e2e]   CODE : ${next.userCode}`);
  console.log(`[e2e]   (expires in ${next.expiresInSeconds}s, hub polls every ${next.intervalSeconds}s)`);
  console.log('[e2e] ============================================\n');

  // Poll the HUB's operation (not the platform): the hub owns the flow.
  let status = 'pending';
  for (let i = 0; i < Math.ceil((next.expiresInSeconds || 900)); i++) {
    const g = await j('GET', `${base}/v1/model-providers/sh/auth/${opId}`, token);
    status = g.json?.status;
    if (status !== 'pending') { console.log(`[e2e] operation -> ${status} account=${g.json?.account || ''}`); break; }
    await sleep(1000);
  }
  if (status !== 'approved') { console.error(`[e2e] sign-in did not complete: ${status}`); process.exit(4); }

  // The credential is now in the hub store; refresh the catalog and run a real turn.
  r = await j('POST', `${base}/v1/model-providers/sh/models/refresh`, token);
  console.log(`[e2e] POST /models/refresh -> ${r.status} ${r.status < 300 ? r.text.slice(0, 120) : r.text.slice(0, 200)}`);
  r = await j('GET', `${base}/v1/model-providers/sh/models`, token);
  const models = (r.json?.models || []).map((m) => m.id || m);
  console.log(`[e2e] GET /models -> ${r.status} count=${models.length} sample=${models.slice(0, 5).join(', ')}`);

  r = await j('POST', `${base}/v1/sessions`, token, { harnessId: HARNESS, idempotencyKey: 'sha-1', modelProviderId: 'sh', modelId: MODEL }, 'sha-1');
  console.log(`[e2e] POST /v1/sessions -> ${r.status}`);
  if (r.status !== 202) { console.error(r.text.slice(0, 400)); process.exit(5); }
  const sid = r.json.session.id;
  let st = 'starting'; for (let i = 0; i < 480; i++) { const g = await j('GET', `${base}/v1/sessions/${sid}`, token); st = g.json?.session?.status; if (['active', 'failed', 'needs-repair', 'starting_failed'].includes(st)) break; await sleep(250); }
  console.log(`[e2e] session ${sid} status=${st}`);
  if (st !== 'active') { const g = await j('GET', `${base}/v1/sessions/${sid}`, token); console.error('startError:', JSON.stringify(g.json?.session?.startError)); process.exit(6); }

  r = await j('POST', `${base}/v1/sessions/${sid}/turns`, token, { content: [{ type: 'text', text: 'Reply with exactly the word: pong' }], idempotencyKey: 'sha-turn-1' }, 'sha-turn-1');
  console.log(`[e2e] POST /turns -> ${r.status}`);
  let answer = null;
  for (let i = 0; i < 600; i++) { const g = await j('GET', `${base}/v1/sessions/${sid}/messages`, token); const a = (g.json?.messages || []).filter((m) => m.role === 'assistant' && (m.content || []).some((c) => c.type === 'text' && c.text)); if (a.length) { answer = a[a.length - 1]; break; } await sleep(500); }
  if (!answer) { console.error('no assistant answer'); process.exit(7); }
  console.log('\n[e2e] ===== REAL MODEL ANSWER (shisa, hub-owned sign-in) =====');
  console.log((answer.content || []).map((c) => c.text || '').join('').trim().slice(0, 800));
  console.log('[e2e] ========================================================');
  console.log('[e2e] PASS: the hub owned the device-code sign-in AND a real turn answered');
} finally {
  try { await j('DELETE', `${base}/v1/model-providers/sh`, token); } catch {}
  try { child.kill(); } catch {}
  console.log(`[e2e] data dir: ${data}`);
}
