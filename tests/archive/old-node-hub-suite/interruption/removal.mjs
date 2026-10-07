// A removal is not one thing: it deletes files, a record, and a registry row. If the hub
// is KILLED in the middle - a power cut, a crash - it must come back to a state that makes
// sense and FINISH the removal, not be left holding half of it.
//
// Nothing is faked: a real hub process, the real published artifact (fetched from the
// registry the deployment publishes), the real HTTP surface. The hub is killed with no
// cleanup, exactly as a power cut would.
import fs from 'node:fs';
import path from 'node:path';
import { Hub, ReleaseServer, publishedRegistry, sleep, tally } from '../lib/hub.mjs';

const t = tally();
const reg = await publishedRegistry();
const harness = reg.plugins.find((p) => p.pluginType === 'harness-adapter' && p.id === 'deepseek');
if (!harness) throw new Error('the published registry has no deepseek harness-adapter');

const rel = await new ReleaseServer().start();
const asset = await rel.stage(harness);
const key = `${harness.pluginType}-${harness.id}`;

console.log(`interruption: a hub killed during a removal — ${key} comes from ${harness.versions[0].url}`);

// 1) a real hub installs the real artifact and waits for it to settle
const hub = new Hub();
await hub.start();
const api = hub.api();
{
  const ins = await api.install(asset);
  t.check(!!ins.plugin, 'the real artifact installs', JSON.stringify(ins).slice(0, 120));
  for (let i = 0; i < 600; i++) { const p = (await api.plugins()).find((x) => x.id === key); if (p && ['ready', 'failed'].includes(p.state)) break; await sleep(1000); }
  const p = (await api.plugins()).find((x) => x.id === key);
  t.check(!!p && ['ready', 'failed'].includes(p.state), 'it settles before the removal', p ? p.state : 'gone');
}

// 2) start a removal and KILL the hub WHILE the intent is on disk - i.e. the removal has
// really begun and its files have not all gone. Poll installed.json for the intent; once it
// is there (or once the whole thing is already finished), kill.
{
  const readRec = () => { try { return JSON.parse(fs.readFileSync(path.join(hub.plugins, 'installed.json'), 'utf8')); } catch { return {}; } };
  const removing = api.remove(key).catch(() => 'killed');
  let landed = 'none';
  for (let i = 0; i < 200; i++) {
    const rec = readRec();
    const dir = fs.existsSync(path.join(hub.plugins, key));
    if (rec[key] && rec[key].deleting) { landed = dir ? 'intent+dir' : 'intent'; break; }
    if (!rec[key] && !dir) { landed = 'finished'; break; }
    await sleep(20);
  }
  await hub.kill();
  await removing;
  const rec = readRec();
  const dir = fs.existsSync(path.join(hub.plugins, key));
  console.log(`  kill landed as: ${landed}; after the kill dir=${dir ? 'present' : 'gone'} record=${rec[key] ? (rec[key].deleting ? 'deleting' : 'present') : 'none'}`);
  t.check(landed !== 'none', 'the kill landed while the removal was observable (intent, or already done)', landed);
}

// 3) restart on the SAME data dir: the hub must finish what was interrupted
{
  const hub2 = new Hub(hub.dir); console.log('  hub2 dir:', hub2.dir, 'log:', hub2.log().slice(-300));
  await hub2.start();                 // start() waits for endpoint.json (up to 20s)
  const api2 = hub2.api();
  console.log('  ports: hub1=' + hub.ep.port + ' hub2=' + hub2.ep.port);
  // The finish is async at boot; give it a moment and then require full consistency.
  for (let i = 0; i < 100; i++) {
    const rec = (() => { try { return JSON.parse(fs.readFileSync(path.join(hub2.plugins, 'installed.json'), 'utf8')); } catch { return {}; } })();
    const dir = fs.existsSync(path.join(hub2.plugins, key));
    if (!dir && !(rec && rec[key])) break;
    await sleep(200);
  }
  const rec = (() => { try { return JSON.parse(fs.readFileSync(path.join(hub2.plugins, 'installed.json'), 'utf8')); } catch { return {}; } })();
  const dir = fs.existsSync(path.join(hub2.plugins, key));
  let rows = [];
  try { rows = await api2.plugins(); } catch (e) { t.check(false, 'the restarted hub answers', e.message); }
  const row = rows.find((x) => x.id === key);
  t.check(!(rec && rec[key]), 'no installed.json record survives for a removed plugin', JSON.stringify(rec[key] || null));
  t.check(!dir, 'no plugin directory survives the finished removal');
  t.check(!row || row.state === 'removing', 'the restarted hub does not report the removed plugin as installed', row ? row.state : 'not listed');
  // And it still works: a fresh install of the same plugin succeeds.
  const again = await api2.install(asset);
  t.check(!!again.plugin, 'the restarted hub can install again afterwards', JSON.stringify(again).slice(0, 100));
  await hub2.stop();
}

rel.stop();
console.log(t.failures ? `\n${t.failures} failure(s)` : '\nthe interrupted removal was finished and the hub is consistent');
process.exitCode = t.failures ? 1 : 0;
