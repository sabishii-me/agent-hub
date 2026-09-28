// @since hub 0.1.6 / contract v1
// ISOLATION. The whole point: acting on one plugin must not change any other plugin's
// facts, and a single read must never contradict itself. Every check here fails if the
// hub keeps one plugin's work in a place another plugin's row can see it.
//
// A plugin row is a set of facts (id, kind, origin, path, version, artifact, ...). Those
// are what a client draws. Touching plugin A leaves B's and C's rows byte-identical -
// except for the plugin(s) the test actually acted on. See leaked()/stableFields().
import { Hub, AssetServer, release, HARNESSES, client, contradictions, leaked, stableFields, until, sleep, tally, contractStamp } from '../lib/hub-harness.mjs';

const stamp = contractStamp();
console.log(`lifecycle-isolation against contract protocol=${stamp.protocol} version=${stamp.version} sha=${stamp.contractSha} build=${stamp.buildId}`);
const t = tally();
const check = t.check;

// Slow bodies, so the in-progress state is real: an instant install would hide whether
// another plugin's row moved while it ran.
const assets = await new AssetServer({ slowMs: 500 }).start();
const A = assets.asset(release(HARNESSES[0]));
const B = assets.asset(release(HARNESSES[1]));
const C = assets.asset(release(HARNESSES[2]));
const M = (e) => `${e.pluginType}-${e.id}`;

const hub = new Hub();
const ep = await hub.endpoint();
const c = client(ep);

console.log('a plugin installs without touching the others');
{
  await Promise.all([c.install(A), c.install(B), c.install(C)]);
  await until(async () => (await c.plugins()).length === 3);
  const settled = await c.plugins();
  const betaBefore = settled.find((p) => p.id === M(release(HARNESSES[1])));
  const gammaBefore = settled.find((p) => p.id === M(release(HARNESSES[2])));

  // Install a FOURTH plugin while watching beta and gamma the whole time.
  const D = assets.asset(release('model-provider-shisa'));
  const watching = [];
  const watcher = (async () => { for (let i = 0; i < 40; i++) { watching.push(await c.plugins()); await sleep(25); } })();
  await c.install(D);
  await watcher;

  const atEnd = (await c.plugins()).find((p) => p.id === M(release(HARNESSES[1])));
  const leaks = watching.flatMap((sample, i) => {
    const beta = sample.find((p) => p.id === M(release(HARNESSES[1])));
    const gamma = sample.find((p) => p.id === M(release(HARNESSES[2])));
    const out = [];
    if (beta && stableFields(beta) !== stableFields(betaBefore)) out.push(`sample ${i}: beta moved`);
    if (gamma && stableFields(gamma) !== stableFields(gammaBefore)) out.push(`sample ${i}: gamma moved`);
    return out;
  });
  check('installing a fourth plugin never changed beta or gamma', leaks.length === 0, leaks.slice(0, 3).join(' | '));
  check('beta is still exactly as it was', stableFields(atEnd) === stableFields(betaBefore));
}

console.log('removing one plugin does not disturb another');
{
  const before = await c.plugins();
  const gammaBefore = before.find((p) => p.id === M(release(HARNESSES[2])));
  const watching = [];
  const watcher = (async () => { for (let i = 0; i < 40; i++) { watching.push(await c.plugins()); await sleep(25); } })();
  await c.remove(M(release(HARNESSES[0])));
  await watcher;
  const gammaMoved = watching.some((s) => {
    const g = s.find((p) => p.id === M(release(HARNESSES[2])));
    return g && stableFields(g) !== stableFields(gammaBefore);
  });
  check('removing alpha left gamma untouched throughout', !gammaMoved);
  check('the plugin set is still internally consistent', contradictions(await c.plugins()).length === 0);
}

console.log('a sample never contradicts itself, under load');
{
  const bad = [];
  const reading = (async () => { for (let i = 0; i < 100; i++) { bad.push(...contradictions(await c.plugins())); await sleep(10); } })();
  const E = assets.asset(release('model-provider-compatible'));
  await Promise.all([c.install(E), c.remove(M(release(HARNESSES[1])))]);
  await reading;
  check('no read ever showed a plugin in two states, or carried a second state field', bad.length === 0, bad.slice(0, 3).join(' | '));
}

console.log('a plugin is in exactly one state it has a right to be in');
{
  const final = await c.plugins();
  const states = final.map((p) => `${p.id}:${p.state}`).join(' ');
  // The test does not predict which plugin succeeds. Whether a plugin ends READY or
  // FAILED depends on whether its OWN runtime can be materialised, which is the plugin's
  // business, not the test's. What must hold for EVERY plugin is: it settled (nothing is
  // left saying installing/preparing forever), and a failure explains itself.
  const TERMINAL = new Set(['ready', 'failed', 'absent']);
  check('every plugin settled to a terminal state (nothing stuck working)', final.every((p) => TERMINAL.has(p.state)), states);
  check('a failed plugin says why', final.filter((p) => p.state === 'failed').every((p) => typeof p.detail === 'string' && p.detail.length > 0), states);
}

await hub.stop();
await assets.stop();
console.log(t.failures ? `\n${t.failures} failure(s)` : '\nlifecycle-isolation: all checks passed');
process.exitCode = t.failures ? 1 : 0;
