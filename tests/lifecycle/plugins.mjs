// @since hub 0.1.5 / contract v1 (see contractStamp() in the run output)
// The plugin install/remove lifecycle under attack: concurrent work, staggered work,
// same-id races, and a client that aborts mid-flight. Behaviour, not structure.
import { Hub, AssetServer, adapter, client, sleep, tally, contractStamp } from '../lib/hub-harness.mjs';

const stamp = contractStamp();
console.log(`plugins-lifecycle against contract protocol=${stamp.protocol} version=${stamp.version} sha=${stamp.contractSha} build=${stamp.buildId}`);
const t = tally();
const check = t.check;

// A slow asset server: a local zip installs in under a millisecond, so the hub's
// in-progress window would be unobservable. The delay is on the DOWNLOAD, which is
// part of a real install.
const assets = await new AssetServer({ slowMs: 400 }).start();
const A = assets.asset(adapter('alpha'));
const B = assets.asset(adapter('beta'));
const C = assets.asset(adapter('gamma'));
const managed = (id) => `harness-adapter-${id}`;

const hub = new Hub();
const ep = await hub.endpoint();
const c = client(ep);

console.log('concurrent installs');
{
  const ins = await Promise.all([c.install(A), c.install(B), c.install(C)]);
  check('three concurrent installs all land', ins.every((r) => r.plugin), ins.map((r) => r.plugin?.id || r.error?.code).join(' '));
  check('each install named its own id (no cross-talk)', ins.map((r) => r.plugin?.id).sort().join() === [managed('alpha'), managed('beta'), managed('gamma')].sort().join());
}

console.log('concurrent removals, staggered 0/80/160ms');
{
  const rem = await Promise.all([c.remove(managed('alpha')), sleep(80).then(() => c.remove(managed('beta'))), sleep(160).then(() => c.remove(managed('gamma')))]);
  check('three staggered removals all report ok', rem.every((r) => r.ok === true), rem.map((r) => (r.ok ? 'ok' : r.error?.code)).join(' '));
  check('the plugin set is empty afterwards', (await c.plugins()).length === 0);
}

console.log('the same id installed concurrently (a race)');
{
  const same = await Promise.all([c.install(A), c.install(B), c.install(A)]);
  const after = await c.plugins();
  check('a raced install of one id never yields two of it', after.filter((p) => p.id === managed('alpha')).length <= 1, same.map((r) => r.plugin?.id || r.error?.code).join(' '));
}

console.log('the same id removed concurrently (a race)');
{
  const sameRem = await Promise.all([c.remove(managed('alpha')), c.remove(managed('alpha'))]);
  check('a raced removal converges to gone', !(await c.plugins()).some((p) => p.id === managed('alpha')), sameRem.map((r) => (r.ok ? 'ok' : r.error?.code)).join(' '));
}

console.log('an abort mid-install leaves the hub usable');
{
  const ac = new AbortController();
  const p = c.install(B, ac.signal).catch(() => 'aborted');
  await sleep(25); ac.abort();
  await p;
  await sleep(2000);
  check('the hub still answers after an aborted install', Array.isArray(await c.plugins()));
  check('an aborted install leaves no stuck busy flag', !(await c.plugins()).some((x) => x.busy));
}

console.log('busy is published while an install runs');
{
  // A local zip installs in milliseconds, so a single sample can miss the window.
  // Poll for the whole duration of the request instead of guessing a delay.
  let busySeen = false;
  const p = c.install(C);
  while (true) {
    const mid = await Promise.race([c.plugins(), sleep(60).then(() => null)]);
    if (mid && mid.some((x) => x.busy === 'install')) { busySeen = true; break; }
    const settled = await Promise.race([p.then(() => true), sleep(10).then(() => false)]);
    if (settled) break;
  }
  await p;
  check('the hub reports busy=install while installing', busySeen);
  check('busy clears once the install answers', !(await c.plugins()).some((x) => x.busy));
}

await hub.stop();
await assets.stop();
console.log(t.failures ? `\n${t.failures} failure(s)` : '\nplugins-lifecycle: all checks passed');
process.exitCode = t.failures ? 1 : 0;
// Let node tear its own sockets down; a forced exit can assert in the runtime on Windows.
