// @since hub 0.1.6 / contract v1
// The EVENT contract. A client that shows plugins subscribes to hub.plugins.changed and
// draws what the event says. So the event must be TRUE: the state it names must be the
// state a read returns, and an event must not name a plugin the action did not touch.
//
// This is the test a full read cannot replace: a leak that only emits a spurious event
// (telling the UI to re-read or re-draw a plugin nothing touched) is invisible to a
// read-only check, and is exactly how one plugin's work made another's card flicker.
import { Hub, AssetServer, release, HARNESSES, client, subscribe, until, sleep, tally, contractStamp } from '../lib/hub-harness.mjs';

const stamp = contractStamp();
console.log(`lifecycle-events against contract protocol=${stamp.protocol} version=${stamp.version} sha=${stamp.contractSha} build=${stamp.buildId}`);
const t = tally();
const check = t.check;

const assets = await new AssetServer({ slowMs: 500 }).start();
const A = assets.asset(release(HARNESSES[0]));
const B = assets.asset(release(HARNESSES[1]));
const C = assets.asset(release(HARNESSES[2]));
const M = (e) => `${e.pluginType}-${e.id}`;

const hub = new Hub();
const ep = await hub.endpoint();
const c = client(ep);

console.log('an event names a real plugin and a real state');
{
  await c.install(A);
  await c.install(B);
  await until(async () => (await c.plugins()).length === 2);
  const sub = subscribe(c);
  await sleep(200);
  await c.install(C);
  // Install answers when the plugin has LANDED; its runtime is materialised in the
  // background afterwards. Wait for the plugin to reach a terminal state, or the events
  // for 'ready' have simply not happened yet (the real runtimes take many seconds).
  await until(async () => {
    const p = (await c.plugins()).find((x) => x.id === M(release(HARNESSES[2])));
    return !!p && (p.state === 'ready' || p.state === 'failed');
  }, { timeoutMs: 120000 });
  sub.stop();
  const VALID = new Set(['absent', 'installing', 'removing', 'preparing', 'failed', 'ready']);
  // The frame sent on subscribe names no plugin: it is the handshake. Only frames that
  // name a plugin are changes, and each of those must carry a real state.
  const changes = sub.state.events.filter((e) => e.id);
  const bad = changes.filter((e) => !VALID.has(e.state));
  check('every event named a plugin id and one of the real states', bad.length === 0, JSON.stringify(bad.slice(0, 3)));
  // The action was ONE install of C. An event about alpha or beta - even one that names
  // the state they already have - is a spurious change: a client would redraw a card
  // nothing touched. Count them; a re-emitted 'ready' is still a lie of "this changed".
  const strays = changes.filter((e) => [M(release(HARNESSES[0])), M(release(HARNESSES[1]))].includes(e.id));
  check('the action emitted no event for a plugin it did not touch', strays.length === 0, strays.map((e) => `${e.id}:${e.state}`).join(' '));
  // And the plugin that WAS installed must have moved through its states, so the event
  // stream is not silent either.
  const gammaStates = changes.filter((e) => e.id === M(release(HARNESSES[2]))).map((e) => e.state);
  check('the installed plugin did emit its states', gammaStates.includes('installing') && gammaStates.includes('ready'), gammaStates.join(' '));
}

console.log('the events for one plugin tell a legal story, in order');
{
  const sub = subscribe(c);
  await sleep(200);
  const start = sub.state.events.filter((e) => e.id).length;
  await c.install(A);
  // Wait for the whole install AND its runtime, so the story is complete.
  await until(async () => {
    const p = (await c.plugins()).find((x) => x.id === `${A.pluginType}-${A.id}`);
    return !!p && ['ready', 'failed'].includes(p.state);
  }, { timeoutMs: 120000 });
  await until(async () => sub.state.events.filter((e) => e.id).length > start);
  await sleep(150);
  sub.stop();
  const story = sub.state.events.filter((e) => e.id === `${A.pluginType}-${A.id}`).map((e) => e.state);
  // The states an install may move through, in order. The event stream is a narrative:
  // it may repeat a state, but it must never go backwards or jump over a stage
  // (installing -> ready with no prepare is possible for a plugin with no runtime).
  const ORDER = { installing: 0, preparing: 1, ready: 2, failed: 2 };
  let legal = true;
  for (let i = 1; i < story.length; i++) {
    const a = ORDER[story[i - 1]], b = ORDER[story[i]];
    if (b < a) { legal = false; break; }               // went backwards
  }
  check("an install's events never go backwards", legal, story.join(' '));
  check('the install is announced, and reaching ready is announced', story.includes('installing') && story.includes('ready'), story.join(' '));
}

console.log('a removal ends with the plugin gone, and the last event does not claim it is ready');
{
  const sub = subscribe(c);
  await sleep(200);
  await c.remove(M(release(HARNESSES[0])));
  await until(async () => !(await c.plugins()).some((p) => p.id === M(release(HARNESSES[0]))));
  await sleep(250);
  sub.stop();
  const alphaEvents = sub.state.events.filter((e) => e.id === M(release(HARNESSES[0])));
  const last = alphaEvents[alphaEvents.length - 1];
  check('the last event for a removed plugin is removing or absent', !!last && ['removing', 'absent'].includes(last.state), JSON.stringify(last));
}

await hub.stop();
await assets.stop();
console.log(t.failures ? `\n${t.failures} failure(s)` : '\nlifecycle-events: all checks passed');
process.exitCode = t.failures ? 1 : 0;
