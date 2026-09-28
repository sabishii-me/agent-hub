// @since hub 0.1.5 / contract v1
// The hub-level event stream: it announces plugin-set changes, survives a client dropping
// and reconnecting, and a reconnect still receives later events.
import { Hub, AssetServer, release, HARNESSES, client, subscribe, sleep, until, tally, contractStamp } from '../lib/hub-harness.mjs';

const stamp = contractStamp();
console.log(`events-reconnect against contract protocol=${stamp.protocol} version=${stamp.version} sha=${stamp.contractSha}`);
const t = tally();
const check = t.check;

const assets = await new AssetServer().start();
const A = assets.asset(release(HARNESSES[0]));
const B = assets.asset(release(HARNESSES[1]));

const hub = new Hub();
const ep = await hub.endpoint();
const c = client(ep);

console.log('subscribing announces the attachment, then follows real changes');
{
  const s = subscribe(c);
  await sleep(300);
  // The contract: the stream announces on subscribe. That handshake frame names no plugin
  // (there is no single plugin it is about) - only a real change carries {id, state}.
  const handshake = s.state.events[0];
  check('subscribing yields exactly the handshake frame', s.state.events.length === 1, `count ${s.state.count}`);
  check('the handshake names no plugin and no state', handshake && !('id' in handshake) && !('state' in handshake), JSON.stringify(handshake));
  await c.install(A);
  await sleep(500);
  const change = s.state.events.find((e) => e.id === `${release(HARNESSES[0]).pluginType}-${release(HARNESSES[0]).id}`);
  check('a real change names the plugin and its state', !!change && typeof change.state === 'string', JSON.stringify(s.state.events.slice(1, 3)));
  s.stop();
  await s.done.catch(() => {});
  check('stopping the subscriber is clean', s.state.stopped);
}

console.log('a dropped stream, reconnected, still receives later events');
{
  const s1 = subscribe(c);
  await sleep(150);
  const before1 = s1.state.count;
  s1.stop();
  await s1.done.catch(() => {});
  const s2 = subscribe(c);
  await sleep(200);
  const after2 = s2.state.count;
  check('a reconnect is announced by its own handshake', after2 === 1, `count ${after2}`);
  await c.install(B);
  await sleep(500);
  check('a reconnect still receives later events', s2.state.count > after2, `count ${after2} -> ${s2.state.count} (first stream saw ${before1})`);
  s2.stop();
  await s2.done.catch(() => {});
}

console.log('two subscribers both see a change');
{
  const x = subscribe(c); const y = subscribe(c);
  await sleep(150);
  const bx = x.state.count, by = y.state.count;
  const key = `${release(HARNESSES[0]).pluginType}-${release(HARNESSES[0]).id}`;
  await c.remove(key);
  // A real removal stops processes and waits for the runtime's files to be released; wait
  // for the plugin to actually be gone rather than for a fixed delay.
  await until(async () => !(await c.plugins()).some((p) => p.id === key), { timeoutMs: 60000 });
  await sleep(200);
  check('subscriber X saw the removal', x.state.count > bx, `${bx} -> ${x.state.count}`);
  check('subscriber Y saw the removal', y.state.count > by, `${by} -> ${y.state.count}`);
  x.stop(); y.stop();
  await Promise.allSettled([x.done, y.done]);
}

await hub.stop();
await assets.stop();
console.log(t.failures ? `\n${t.failures} failure(s)` : '\nevents-reconnect: all checks passed');
process.exitCode = t.failures ? 1 : 0;
// Let node tear its own sockets down; a forced exit can assert in the runtime on Windows.
