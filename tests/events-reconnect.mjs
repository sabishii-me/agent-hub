// @since hub 0.1.5 / contract v1
// The hub-level event stream: it announces plugin-set changes, survives a client dropping
// and reconnecting, and a reconnect still receives later events.
import { Hub, AssetServer, adapter, client, subscribe, sleep, tally, contractStamp } from './lib/hub-harness.mjs';

const stamp = contractStamp();
console.log(`events-reconnect against contract protocol=${stamp.protocol} version=${stamp.version} sha=${stamp.contractSha}`);
const t = tally();
const check = t.check;

const assets = await new AssetServer().start();
const A = assets.asset(adapter('alpha'));
const B = assets.asset(adapter('beta'));

const hub = new Hub();
const ep = await hub.endpoint();
const c = client(ep);

console.log('a fresh subscription announces itself, then follows changes');
{
  const s = subscribe(c);
  await sleep(200);
  check('subscribing yields an immediate event', s.state.count >= 1, `count ${s.state.count}`);
  await c.install(A);
  await sleep(400);
  check('an install is announced', s.state.count >= 2, `count ${s.state.count}`);
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
  await sleep(150);
  const after2 = s2.state.count;
  check('a reconnect re-announces on subscribe', after2 >= 1, `count ${after2}`);
  await c.install(B);
  await sleep(400);
  check('a reconnect still receives later events', s2.state.count > after2, `count ${after2} -> ${s2.state.count} (first stream saw ${before1})`);
  s2.stop();
  await s2.done.catch(() => {});
}

console.log('two subscribers both see a change');
{
  const x = subscribe(c); const y = subscribe(c);
  await sleep(150);
  const bx = x.state.count, by = y.state.count;
  await c.remove('harness-adapter-alpha');
  await sleep(400);
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
