#!/usr/bin/env node
// Ask a running hub for its connection material — and only trust it when the hub
// is really there.
//
// This is the reference implementation of the client-side discovery rule, and the
// tool a developer runs while wiring a front end up by hand:
//
//   node scripts/hub-connect.mjs                     # default dir (~/.prts-core)
//   node scripts/hub-connect.mjs --data-dir /tmp/x   # a hub started with PRTS_DATA_DIR
//   node scripts/hub-connect.mjs --pid 12345         # only accept THIS process (a sidecar)
//   node scripts/hub-connect.mjs --json              # machine readable
//   node scripts/hub-connect.mjs --wait 30           # wait up to 30s for it to come up
//   node scripts/hub-connect.mjs --watch             # keep printing as it restarts
//
// What it does, in the order that matters:
//   1. read <data-dir>/endpoint.json (atomic:  temp + rename, so never half a file);
//   2. refuse it if `pid` is not alive — a killed hub leaves the file behind, and a
//      stale token must never be handed out as if it worked;
//   3. probe GET /v1/harnesses (the ONE route that needs no token) and require 200:
//      that proves the port answers AND that this is a hub, not something else on a
//      port that has been reused;
//   4. only then print the url and token.
//
// It never prints the token unless asked to (--json or the default shell form),
// because the token is a credential: for anything shipped to a user, the token
// belongs in the process that spawned the hub, not in a log or a UI.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const args = process.argv.slice(2);
const opt = (name, fallback) => {
  const i = args.indexOf(name);
  return i >= 0 && args[i + 1] ? args[i + 1] : fallback;
};
const flag = (name) => args.includes(name);

const dataDir = path.resolve(opt('--data-dir', path.join(os.homedir(), '.prts-core')));
const wantPid = opt('--pid', null) ? Number(opt('--pid', null)) : null;
const waitSeconds = Number(opt('--wait', '0'));
const asJson = flag('--json');
const watch = flag('--watch');
const endpointFile = path.join(dataDir, 'endpoint.json');

const alive = (pid) => { try { process.kill(pid, 0); return true; } catch { return false; } };

async function readOnce() {
  let raw;
  try { raw = fs.readFileSync(endpointFile, 'utf8'); } catch { return { state: 'absent' }; }
  let ep;
  try { ep = JSON.parse(raw); } catch { return { state: 'unparsable', raw: raw.slice(0, 120) }; }
  if (wantPid !== null && ep.pid !== wantPid) return { state: 'other-process', ep };
  if (!alive(ep.pid)) return { state: 'dead', ep };
  let status = 0;
  try {
    const r = await fetch(`http://127.0.0.1:${ep.port}/v1/harnesses`, { signal: AbortSignal.timeout(1500) });
    status = r.status;
  } catch { return { state: 'unreachable', ep }; }
  if (status !== 200) return { state: `http-${status}`, ep, status };
  return { state: 'ready', ep };
}

function report(ep) {
  if (asJson) { process.stdout.write(JSON.stringify(ep, null, 2) + '\n'); return; }
  process.stdout.write([
    `export PRTS_URL=http://127.0.0.1:${ep.port}`,
    `export PRTS_TOKEN=${ep.token}`,
    `# pid=${ep.pid} startedAt=${ep.startedAt ?? 'n/a'} buildId=${String(ep.buildId).slice(0, 12)}`,
    '',
  ].join('\n'));
}

const describe = {
  absent: () => `no endpoint file at ${endpointFile} — nothing is running there (start a hub with PRTS_DATA_DIR=${dataDir})`,
  unparsable: (r) => `endpoint file is not JSON (${r.raw})`,
  'other-process': (r) => `endpoint file belongs to pid ${r.ep.pid}, not the process being waited for — a stale or foreign hub`,
  dead: (r) => `endpoint file names pid ${r.ep.pid}, which is not running — a hub that was killed leaves this behind; ignore it`,
  unreachable: (r) => `port ${r.ep.port} does not answer — the file is stale or the hub is still shutting down`,
};

const deadline = Date.now() + Math.max(0, waitSeconds) * 1000;
for (;;) {
  const r = await readOnce();
  if (r.state === 'ready') {
    report(r.ep);
    if (!watch) break;
    // The file is REPLACED by an atomic rename, so watching the file itself would
    // follow a dead inode: watch the directory and re-validate on every change.
    await new Promise((resolve) => {
      const w = fs.watch(dataDir, async () => {
        const again = await readOnce();
        if (again.state === 'ready' && again.ep.token !== r.ep.token) { report(again.ep); w.close(); resolve(); }
      });
    });
    continue;
  }
  if (!asJson) process.stderr.write(`waiting: ${(describe[r.state] || (() => `${r.state}`))(r)}\n`);
  if (Date.now() >= deadline) {
    if (asJson) process.stdout.write(JSON.stringify({ error: r.state }) + '\n');
    process.exit(1);
  }
  await new Promise((res) => setTimeout(res, 250));
}
