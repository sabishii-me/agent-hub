// @since hub 0.1.5 / contract v1
// The hub is killed MID-REMOVAL, then restarted on the same data dir: the set is readable
// again, nothing is stuck busy, and work can continue. This is the recovery a user needs
// when the application itself was killed.
import fs from 'node:fs';
import path from 'node:path';
import { Hub, AssetServer, adapter, client, sleep, tally, contractStamp } from './lib/hub-harness.mjs';

const stamp = contractStamp();
console.log(`crash-recovery against contract protocol=${stamp.protocol} version=${stamp.version} sha=${stamp.contractSha}`);
const t = tally();
const check = t.check;

const assets = await new AssetServer().start();
const A = assets.asset(adapter('alpha'));
const B = assets.asset(adapter('beta'));
const managed = (id) => `harness-adapter-${id}`;

console.log('killed mid-removal, restarted on the same data dir');
{
  const hub = new Hub();
  const ep = await hub.endpoint();
  const c = client(ep);
  await c.install(A);
  const killing = c.remove(managed('alpha')).catch(() => 'killed');
  await sleep(40);
  hub.stop();
  await killing;
  await sleep(600);
  // Endpoint.json belongs to the dead process; a restart writes its own.
  try { fs.rmSync(path.join(hub.dir, 'endpoint.json'), { force: true }); } catch { /* fine */ }
  const hub2 = new Hub(hub.dir);
  const ep2 = await hub2.endpoint();
  const c2 = client(ep2);
  const after = await c2.plugins();
  check('a restarted hub answers the plugin list', Array.isArray(after), `${after.length} plugin(s)`);
  check('the restarted hub is not stuck busy', !after.some((x) => x.busy), after.map((x) => `${x.id}:${x.busy}`).join(' '));
  check('a plugin can still be installed after the restart', !!(await c2.install(B)).plugin);
  await hub2.stop();
}

console.log('a half-written plugin directory does not lock the hub out');
{
  // The reported shape: a plugin directory with a manifest the boot self-check refuses.
  // ADR-0002 says a bad plugin must degrade, not take the product down - so a hub whose
  // data dir holds one must still START.
  const hub = new Hub();
  const ep = await hub.endpoint();
  const c = client(ep);
  await c.install(A);
  await hub.stop();
  await sleep(300);
  fs.writeFileSync(path.join(hub.plugins, managed('alpha'), 'manifest.json'), '{"id":"alpha","kind":"harness-adapter"}');
  try { fs.rmSync(path.join(hub.dir, 'endpoint.json'), { force: true }); } catch { /* fine */ }
  const hub2 = new Hub(hub.dir);
  let started = true;
  try { await hub2.endpoint(); } catch { started = false; }
  check('a hub with a bad plugin still starts (ADR-0002: degrade, do not die)', started, started ? '' : hub2.log().split('\n').slice(-4).join(' | '));
  await hub2.stop();
}

await assets.stop();
console.log(t.failures ? `\n${t.failures} failure(s)` : '\ncrash-recovery: all checks passed');
process.exitCode = t.failures ? 1 : 0;
// Let node tear its own sockets down; a forced exit can assert in the runtime on Windows.
