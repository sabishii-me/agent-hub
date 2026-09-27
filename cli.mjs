#!/usr/bin/env node
// agent-hub CLI — a thin client of the hub process. No logic lives here: every
// command is one HTTP call (chat is one POST + SSE read). The CLI is the test
// main battlefield: every feature is exercised here before any UI work.
//
//   hub daemon start | stop
//   hub harness list | enable <id> | disable <id>
//   hub status
//   hub chat -h <harnessId> [-m <modelId>] [-s <sessionId>] <message>
//   hub session list | show <id> | delete <id> | turns <id>
//   hub cancel <id>
//   hub model <id> <modelId>
//   hub repair <id> [--mode tombstone|truncate] [--confirm] [--preview]
//   hub approvals <id>

import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const DATA_DIR = process.env.AGENT_HUB_DATA_DIR || path.join(os.homedir(), '.sabishii-me/agent-hub');
const ENDPOINT = path.join(DATA_DIR, 'endpoint.json');

function endpoint() {
  if (!fs.existsSync(ENDPOINT)) {
    process.stderr.write('core is not running (no endpoint.json). hub daemon start\n');
    process.exit(3);
  }
  return JSON.parse(fs.readFileSync(ENDPOINT, 'utf8'));
}

async function call(method, p, body) {
  const ep = endpoint();
  const r = await fetch(`http://127.0.0.1:${ep.port}${p}`, {
    method,
    headers: {
      authorization: `Bearer ${ep.token}`,
      ...(body ? { 'content-type': 'application/json' } : {}),
    },
    body: body ? JSON.stringify(body) : undefined,
  });
  const j = await r.json().catch(() => ({}));
  if (!r.ok) {
    process.stderr.write(`${(j.error && j.error.code) || r.status}: ${(j.error && j.error.message) || ''}\n`);
    process.exit(2);
  }
  return j;
}

const print = (v, raw) => process.stdout.write(JSON.stringify(v, null, raw ? 0 : 2) + '\n');

const argv = process.argv.slice(2);
const VALUE_FLAGS = ['h', 'm', 's', 'mode', 'limit'];
function parseArgs() {
  const pos = [];
  const flags = {};
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a.startsWith('-')) {
      const name = a.replace(/^-+/, '');
      if (VALUE_FLAGS.includes(name)) flags[name] = argv[++i];
      else flags[name] = true;
    } else pos.push(a);
  }
  return { pos, flags };
}
const { pos, flags } = parseArgs();

async function chat() {
  let sid = flags.s;
  const harnessId = flags.h || 'conformance';
  const modelId = flags.m;
  const message = pos.slice(1).join(' ');

  if (!sid) {
    const j = await call('POST', '/v1/sessions', { harnessId, connectionId: null, modelId: modelId || null, title: null });
    sid = j.session.id;
    process.stdout.write(`session ${sid} (${harnessId})\n`);
  }

  const ep = endpoint();
  const r = await fetch(`http://127.0.0.1:${ep.port}/v1/sessions/${encodeURIComponent(sid)}/turns`, {
    method: 'POST',
    headers: { authorization: `Bearer ${ep.token}`, 'content-type': 'application/json' },
    body: JSON.stringify({ content: [{ type: 'text', text: message }] }),
  });
  if (r.status === 409) {
    const j = await r.json().catch(() => ({}));
    process.stderr.write(`${(j.error && j.error.code) || 409}: ${(j.error && j.error.message) || 'busy'}\n`);
    process.exit(2);
  }
  if (!r.ok) {
    const j = await r.json().catch(() => ({}));
    process.stderr.write(`${(j.error && j.error.code) || r.status}: ${(j.error && j.error.message) || ''}\n`);
    process.exit(2);
  }

  const reader = r.body.getReader();
  const dec = new TextDecoder();
  let buf = '';
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
      if (event === 'message.delta' && payload.kind !== 'reasoning') {
        process.stdout.write(payload.text || '');
      } else if (event === 'message.completed') {
        process.stdout.write('\n');
      } else if (event === 'approval.requested') {
        process.stdout.write(`\n[approval requested] ${payload.approvalId} tool=${payload.tool}\n`);
      } else if (event === 'turn.ended') {
        process.stdout.write(`[turn ${payload.turn.ended}]\n`);
      }
    }
  }
}

async function main() {
  if (pos[0] === 'daemon' && pos[1] === 'start') {
    const child = spawn(process.execPath, [path.join(HERE, 'server.mjs')], { detached: true, windowsHide: true, stdio: 'ignore', env: process.env });
    child.unref();
    for (let i = 0; i < 20; i++) {
      await new Promise((r) => setTimeout(r, 250));
      if (fs.existsSync(ENDPOINT)) {
        const ep = JSON.parse(fs.readFileSync(ENDPOINT, 'utf8'));
        print({ started: true, port: ep.port, pid: ep.pid });
        return;
      }
    }
    process.stderr.write('core did not come up within 5s\n');
    process.exit(4);
  } else if (pos[0] === 'daemon' && pos[1] === 'stop') {
    print(await call('POST', '/v1/hub/shutdown'));
  } else if (pos[0] === 'harness' && pos[1] === 'list') {
    const j = await call('GET', '/v1/harnesses');
    if (flags.json) print(j, true);
    else for (const h of j.harnesses) process.stdout.write(`${h.status === 'enabled' ? '*' : ' '} ${h.id.padEnd(12)} ${(h.name || '').padEnd(22)} ${h.status}  caps=${(h.capabilities || []).join(',') || '-'}\n`);
  } else if (pos[0] === 'harness' && (pos[1] === 'enable' || pos[1] === 'disable')) {
    print(await call('POST', `/v1/harnesses/${encodeURIComponent(pos[2])}/${pos[1]}`));
  } else if (pos[0] === 'harness' && pos[1] === 'tools') {
    const j = await call('GET', `/v1/harnesses/${encodeURIComponent(pos[2])}/tools`);
    if (flags.json) print(j);
    else if (!j.known) process.stdout.write(`${pos[2]}: catalog unknown (harness declares no tools capability)\n`);
    else for (const t of j.tools) process.stdout.write(`${t.enabled ? '*' : ' '} ${t.name.padEnd(14)} ${t.kind.padEnd(8)} approval=${t.approvalDefault}\n`);
  } else if (pos[0] === 'status') {
    print(await call('GET', '/v1/hub/status'));
  } else if (pos[0] === 'provider' && pos[1] === 'list') {
    print(await call('GET', '/v1/hub/providers'));
  } else if (pos[0] === 'provider' && pos[1] === 'create') {
    const [label, url, token] = pos.slice(2);
    print(await call('POST', '/v1/hub/providers', { label: label || '', url, token: token || undefined }));
  } else if (pos[0] === 'provider' && pos[1] === 'delete') {
    print(await call('DELETE', `/v1/hub/providers/${encodeURIComponent(pos[2])}`));
  } else if (pos[0] === 'provider' && pos[1] === 'logout') {
    print(await call('POST', `/v1/hub/providers/${encodeURIComponent(pos[2])}/logout`));
  } else if (pos[0] === 'skill' && pos[1] === 'list') {
    print(await call('GET', '/v1/hub/skills'));
  } else if (pos[0] === 'skill' && pos[1] === 'get') {
    print(await call('GET', `/v1/hub/skills/${encodeURIComponent(pos[2])}/files/${encodeURIComponent(pos[3] || 'SKILL.md')}`));
  } else if (pos[0] === 'skill' && pos[1] === 'put') {
    print(await call('PUT', `/v1/hub/skills/${encodeURIComponent(pos[2])}/files/${encodeURIComponent(pos[3] || 'SKILL.md')}`, { content: pos.slice(4).join(' ') }));
  } else if (pos[0] === 'skill' && pos[1] === 'delete') {
    print(await call('DELETE', `/v1/hub/skills/${encodeURIComponent(pos[2])}`));
  } else if (pos[0] === 'connection' && pos[1] === 'list') {
    print(await call('GET', '/v1/hub/connections'));
  } else if (pos[0] === 'connection' && pos[1] === 'create') {
    const [name, scheme, endpoint, envName, token] = pos.slice(2);
    print(await call('POST', '/v1/hub/connections', { name, scheme, endpoint, envName: envName || undefined, token: token || undefined }));
  } else if (pos[0] === 'connection' && (pos[1] === 'enable' || pos[1] === 'disable')) {
    print(await call('PATCH', `/v1/hub/connections/${encodeURIComponent(pos[2])}`, { state: pos[1] }));
  } else if (pos[0] === 'connection' && pos[1] === 'delete') {
    print(await call('DELETE', `/v1/hub/connections/${encodeURIComponent(pos[2])}`));
  } else if (pos[0] === 'chat') {
    await chat();
  } else if (pos[0] === 'session' && pos[1] === 'list') {
    print(await call('GET', '/v1/sessions'));
  } else if (pos[0] === 'session' && pos[1] === 'show') {
    print(await call('GET', `/v1/sessions/${encodeURIComponent(pos[2])}`));
  } else if (pos[0] === 'session' && pos[1] === 'delete') {
    print(await call('DELETE', `/v1/sessions/${encodeURIComponent(pos[2])}`));
  } else if (pos[0] === 'session' && pos[1] === 'turns') {
    print(await call('GET', `/v1/sessions/${encodeURIComponent(pos[2])}/turns`));
  } else if (pos[0] === 'session' && pos[1] === 'messages') {
    const q = new URLSearchParams();
    if (pos[3]) q.set('beforeId', pos[3]);
    if (flags.limit) q.set('limit', flags.limit);
    const qs = q.toString();
    print(await call('GET', `/v1/sessions/${encodeURIComponent(pos[2])}/messages${qs ? '?' + qs : ''}`));
  } else if (pos[0] === 'cancel') {
    print(await call('POST', `/v1/sessions/${encodeURIComponent(pos[1])}/cancel`));
  } else if (pos[0] === 'model') {
    print(await call('PATCH', `/v1/sessions/${encodeURIComponent(pos[1])}`, { modelId: pos[2] }));
  } else if (pos[0] === 'repair') {
    print(await call('POST', `/v1/sessions/${encodeURIComponent(pos[1])}/repair`, {
      mode: flags.mode || 'tombstone',
      confirm: Boolean(flags.confirm),
      preview: Boolean(flags.preview),
    }));
  } else if (pos[0] === 'approvals') {
    print(await call('GET', `/v1/sessions/${encodeURIComponent(pos[1])}/approvals`));
  } else {
    process.stdout.write(
      'agent-hub — multi-harness manager\n' +
        '  hub daemon start | stop\n' +
        '  hub harness list [--json]\n' +
        '  agent-hub harness enable <id> | disable <id>\n' +
        '  agent-hub harness tools <id>\n' +
        '  hub status\n' +
        '  hub chat -h <harnessId> [-m <modelId>] [-s <sessionId>] <message>\n' +
        '  hub session list | show <id> | delete <id> | turns <id> | messages <id>\n' +
        '  hub cancel <id>\n' +
        '  hub model <id> <modelId>\n' +
        '  hub repair <id> [--mode tombstone|truncate] [--confirm] [--preview]\n' +
        '  hub approvals <id>\n' +
        '  agent-hub provider list\n' +
        '  agent-hub provider create <label> <url> [token]\n' +
        '  agent-hub provider delete <id>\n' +
        '  agent-hub provider logout <id>\n' +
        '  agent-hub skill list\n' +
        '  agent-hub skill get <id> [path]\n' +
        '  agent-hub skill put <id> <path> <content>\n' +
        '  agent-hub skill delete <id>\n' +
        '  agent-hub connection list\n' +
        '  agent-hub connection create <name> <scheme> <endpoint> [envName] [token]\n' +
        '  agent-hub connection enable <id> | disable <id>\n' +
        '  agent-hub connection delete <id>\n'
    );
    process.exit(pos.length ? 2 : 0);
  }
}

main().catch((e) => { process.stderr.write(`${e.stack}\n`); process.exit(1); });
