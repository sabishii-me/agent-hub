// debate.mjs — a CLIENT-side orchestrator (S5, B-1). Two harnesses talk; each
// is fed the other's reply. No core logic: this is just a client that creates
// two sessions and relays turns. Every injected turn carries source:'bridge' +
// bridge provenance so a bridge message is never mistaken for a human turn.
//
// Usage:
//   node debate.mjs --a <harnessId> --b <harnessId> [--rounds N] [--stop TOKEN] [--topic TEXT]
//
// Env: AGENT_HUB_DATA_DIR (like the CLI). Requires the core daemon to be running.

import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';

const DATA_DIR = process.env.AGENT_HUB_DATA_DIR || path.join(os.homedir(), '.sabishii-me/agent-hub');
const ENDPOINT = path.join(DATA_DIR, 'endpoint.json');

function endpoint() {
  if (!fs.existsSync(ENDPOINT)) {
    process.stderr.write('core is not running (no endpoint.json). Start the core first.\n');
    process.exit(3);
  }
  return JSON.parse(fs.readFileSync(ENDPOINT, 'utf8'));
}

function parseFlags(argv) {
  const flags = {};
  for (let i = 0; i < argv.length; i++) {
    if (argv[i].startsWith('--')) flags[argv[i].slice(2)] = argv[i + 1] && !argv[i + 1].startsWith('--') ? argv[++i] : true;
  }
  return flags;
}

async function rpc(method, p, body) {
  const ep = endpoint();
  const r = await fetch(`http://127.0.0.1:${ep.port}${p}`, {
    method,
    headers: { authorization: `Bearer ${ep.token}`, ...(body ? { 'content-type': 'application/json' } : {}) },
    body: body ? JSON.stringify(body) : undefined,
  });
  const j = await r.json().catch(() => ({}));
  if (!r.ok) throw new Error(`${(j.error && j.error.code) || r.status}: ${(j.error && j.error.message) || ''}`);
  return j;
}

// One turn over SSE; returns the completed assistant text.
async function say(sid, text, { source = 'user', bridge = null } = {}) {
  const ep = endpoint();
  const r = await fetch(`http://127.0.0.1:${ep.port}/v1/sessions/${encodeURIComponent(sid)}/turns`, {
    method: 'POST',
    headers: { authorization: `Bearer ${ep.token}`, 'content-type': 'application/json' },
    body: JSON.stringify({ content: [{ type: 'text', text }], source, bridge }),
  });
  if (!r.ok) {
    const j = await r.json().catch(() => ({}));
    throw new Error(`${(j.error && j.error.code) || r.status}: ${(j.error && j.error.message) || ''}`);
  }
  const reader = r.body.getReader();
  const dec = new TextDecoder();
  let buf = '';
  let reply = '';
  let ended = null;
  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    buf += dec.decode(value, { stream: true });
    let i;
    while ((i = buf.indexOf('\n\n')) >= 0) {
      const block = buf.slice(0, i); buf = buf.slice(i + 2);
      let event = 'message';
      let data = '';
      for (const line of block.split('\n')) {
        if (line.startsWith('event: ')) event = line.slice(7);
        else if (line.startsWith('data: ')) data += line.slice(6);
      }
      if (!data) continue;
      let payload;
      try { payload = JSON.parse(data); } catch { continue; }
      if (event === 'message.delta' && payload.kind !== 'reasoning') reply += payload.text || '';
      else if (event === 'message.completed') { reply = payload.text || reply; }
      else if (event === 'turn.ended') ended = payload.turn;
    }
  }
  return { reply, ended };
}

async function main() {
  const flags = parseFlags(process.argv.slice(2));
  const a = flags.a, b = flags.b;
  if (!a || !b) {
    process.stderr.write('usage: node debate.mjs --a <harnessId> --b <harnessId> [--rounds N] [--stop TOKEN] [--topic TEXT]\n');
    process.exit(2);
  }
  const rounds = Number(flags.rounds) || 3;
  const stop = flags.stop || null;

  const sa = (await rpc('POST', '/v1/sessions', { harnessId: a, connectionId: null, modelId: null, title: 'debate A' })).session.id;
  const sb = (await rpc('POST', '/v1/sessions', { harnessId: b, connectionId: null, modelId: null, title: 'debate B' })).session.id;
  process.stdout.write(`debate: A=${a}(${sa})  B=${b}(${sb})\n`);

  // Opening line goes to A as a real user turn.
  const topic = flags.topic || 'Talk to the other agent. Keep it short.';
  let toB = topic;
  process.stdout.write(`[user -> A] ${topic}\n`);

  for (let round = 1; round <= rounds; round++) {
    // A replies (first round: the topic is the human turn; later rounds carry B's bridge)
    const ra = await say(sa, toB, round === 1 ? { source: 'user' } : { source: 'bridge', bridge: { from: `B:${sb}`, round } });
    process.stdout.write(`[A r${round}] ${ra.reply}\n`);
    if (stop && ra.reply.includes(stop)) { process.stdout.write(`[stop marker from A at round ${round}]\n`); break; }

    // B replies to A's bridge
    const rb = await say(sb, ra.reply, { source: 'bridge', bridge: { from: `A:${sa}`, round } });
    process.stdout.write(`[B r${round}] ${rb.reply}\n`);
    if (stop && rb.reply.includes(stop)) { process.stdout.write(`[stop marker from B at round ${round}]\n`); break; }

    toB = rb.reply; // B's next line is what A will answer
  }
  process.stdout.write('debate: done\n');
}

main().catch((e) => { process.stderr.write(`${e.message}\n`); process.exit(1); });
