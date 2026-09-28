// ADR-0009, proven: the hub is a concurrent, non-blocking service.
//
// Two things are checked against a REAL hub process, not asserted:
//
//   1. a long operation (a large plugin's removal) does not stop the hub answering
//      anyone else - a concurrent GET /v1/hub/status keeps succeeding throughout;
//   2. the event stream survives a dropped connection: a client that reconnects with
//      `Last-Event-ID` receives the changes it missed (WHATWG Server-sent events).
//
// The removal tree is built locally (many small files) so the test needs no network and
// no 363 MB download: what matters is that the delete is large enough that a SYNCHRONOUS
// one would hold the loop - which is exactly the reproduced defect.
import fs from 'node:fs';
import fsp from 'node:fs/promises';
import path from 'node:path';
import http from 'node:http';
import { Hub, sleep, tally } from '../lib/hub.mjs';

const t = tally();

// A plugin the hub OWNS (so removal is allowed), with a large runtime tree.
async function makeHeavyPlugin(hub) {
  const id = 'harness-adapter-bulk';
  const dir = path.join(hub.plugins, id);
  await fsp.mkdir(path.join(dir, 'runtime', 'node_modules'), { recursive: true });
  const protocol = JSON.parse(fs.readFileSync(path.join(HUB_ROOT(), 'contract', 'adapter-v1.json'), 'utf8')).version;
  await fsp.writeFile(path.join(dir, 'manifest.json'), JSON.stringify({
    id: 'bulk', pluginType: 'harness-adapter', name: 'Bulk', protocol,
    command: ['node', 'adapter.mjs'], runtime: { package: 'bulk', version: '0.0.0', command: ['node', 'runtime.mjs'] },
  }));
  for (let d = 0; d < 150; d++) {
    const sub = path.join(dir, 'runtime', 'node_modules', `pkg-${d}`);
    await fsp.mkdir(sub, { recursive: true });
    for (let f = 0; f < 120; f++) await fsp.writeFile(path.join(sub, `f-${f}.js`), `module.exports=${f};\n`);
  }
  await fsp.writeFile(path.join(hub.plugins, 'installed.json'), JSON.stringify({ [id]: { source: 'local://bulk', installedAt: new Date().toISOString() } }));
  return id;
}
function HUB_ROOT() { return path.resolve(new URL('../..', import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, '$1')); }

function status(port, token) {
  return new Promise((resolve) => {
    const req = http.request({ host: '127.0.0.1', port, path: '/v1/hub/status', headers: { authorization: `Bearer ${token}` }, timeout: 3000 }, (res) => {
      let b = ''; res.on('data', (c) => { b += c; }); res.on('end', () => resolve({ ok: true, status: res.statusCode }));
    });
    req.on('timeout', () => { req.destroy(new Error('timeout')); });
    req.on('error', (e) => resolve({ ok: false, error: e.message }));
    req.end();
  });
}

const hub = new Hub();
await hub.start();
const id = await makeHeavyPlugin(hub);
const { port, token } = hub.ep;
const H = { authorization: `Bearer ${token}`, 'content-type': 'application/json' };

// 1) the removal must not stop the hub answering. Poll throughout, until the plugin is gone.
{
  const samples = [];
  let stopped = false;
  const poller = (async () => { while (!stopped) { const r = await status(port, token); samples.push(r); await sleep(150); } })();
  const del = await new Promise((resolve) => {
    const req = http.request({ host: '127.0.0.1', port, path: `/v1/hub/plugins/${id}`, method: 'DELETE', headers: H, timeout: 60000 }, (res) => { let b = ''; res.on('data', (c) => { b += c; }); res.on('end', () => resolve({ status: res.statusCode, body: b })); });
    req.on('error', (e) => resolve({ error: e.message })); req.end();
  });
  // wait for the resource to settle (gone), still polling
  let gone = false;
  for (let i = 0; i < 400; i++) {
    const list = await new Promise((resolve) => { const req = http.request({ host: '127.0.0.1', port, path: '/v1/hub/plugins', headers: H }, (res) => { let b = ''; res.on('data', (c) => { b += c; }); res.on('end', () => resolve(JSON.parse(b).plugins || [])); }); req.end(); });
    if (!list.some((p) => p.id === id)) { gone = true; break; }
    await sleep(150);
  }
  stopped = true; await poller;
  const timeouts = samples.filter((s) => !s.ok);
  t.check(del.status === 202, 'the removal is accepted with 202, not held open', `status=${del.status} body=${String(del.body).slice(0, 80)}`);
  t.check(gone, 'the plugin is gone from the list when the removal finishes');
  t.check(timeouts.length === 0, 'the hub answered every concurrent status poll during the removal', `${timeouts.length} timeout(s) of ${samples.length}`);
}

// 2) SSE reconnect: an event emitted while a client is away is delivered on reconnect by
// Last-Event-ID (WHATWG Server-sent events). We hold a subscription open, cause a change,
// capture the id it carries, drop the connection, then reconnect with that id and require
// the change to arrive.
{
  await makeHeavyPlugin(hub);                  // a fresh bulk plugin to remove
  let buffer = '';
  const stream = http.request({ host: '127.0.0.1', port, path: '/v1/hub/events', headers: H }, (res) => {
    res.setEncoding('utf8'); res.on('data', (c) => { buffer += c; });
  });
  stream.on('error', () => {}); stream.end();
  await sleep(250);
  // Cause a change while connected: accept a removal (async).
  const d = http.request({ host: '127.0.0.1', port, path: `/v1/hub/plugins/${id}`, method: 'DELETE', headers: H }, (res) => { res.resume(); });
  d.on('error', () => {}); d.end();
  // Wait for an event with an id to arrive.
  for (let i = 0; i < 40 && !/id: hub-\d+/.test(buffer); i++) await sleep(100);
  const ids = [...buffer.matchAll(/id: (hub-\d+)/g)].map((m) => m[1]);
  const lastId = ids.length ? ids[ids.length - 1] : null;
  t.check(!!lastId, 'a live change carries an id (the standard reconnect key)', `ids=${JSON.stringify(ids)}`);
  stream.destroy();
  if (lastId) {
    // "Away": recreate the plugin, then remove it - a real change while no client is
    // listening - and reconnect with Last-Event-ID to receive it.
    await makeHeavyPlugin(hub);
    await sleep(150);
    const d2 = http.request({ host: '127.0.0.1', port, path: `/v1/hub/plugins/${id}`, method: 'DELETE', headers: H }, (res) => { res.resume(); });
    d2.on('error', () => {}); d2.end();
    await sleep(300);
    const replay = await new Promise((resolve) => {
      const r = http.request({ host: '127.0.0.1', port, path: '/v1/hub/events', headers: { ...H, 'last-event-id': lastId } }, (res) => {
        let b = ''; res.on('data', (c) => { b += c; }); setTimeout(() => { r.destroy(); resolve(b); }, 700);
      });
      r.on('error', () => resolve('')); r.end();
    });
    t.check(/id: hub-[0-9]+/.test(replay), 'a reconnect with Last-Event-ID receives the events it missed', JSON.stringify(replay.slice(0, 120)));
  }
}

await hub.stop();
console.log(t.failures ? `\n${t.failures} failure(s)` : '\nthe hub stayed concurrent and the stream survived a reconnect');
process.exit(t.failures ? 1 : 0);
