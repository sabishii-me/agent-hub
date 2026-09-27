// @since hub 0.1.5 / contract v1
// Malformed and hostile requests: the hub refuses them as DATA and stays alive.
import { Hub, AssetServer, adapter, client, tally, contractStamp } from './lib/hub-harness.mjs';

const stamp = contractStamp();
console.log(`hostile-requests against contract protocol=${stamp.protocol} version=${stamp.version} sha=${stamp.contractSha}`);
const t = tally();
const check = t.check;

const assets = await new AssetServer().start();
const A = assets.asset(adapter('alpha'));

const hub = new Hub();
const ep = await hub.endpoint();
const c = client(ep);
await c.install(A);

{
  const bad = await Promise.all([
    c.installRaw('{'),                                                    // not JSON
    c.install({ ...A, sha256: 'x'.repeat(64) }),                          // tampered digest
    c.install({ ...A, id: 'alpha', url: 'ftp://nope/x.zip' }),            // non-http url
    c.removeRaw('..%2f..%2fetc'),                                         // path traversal
    c.remove('harness-adapter-does-not-exist'),                           // absent plugin
    fetch(`${c.base}/v1/hub/plugins`, { method: 'POST', headers: { authorization: c.H.authorization }, body: '{}' }).then(async (r) => ({ status: r.status })), // no content-type
  ]);
  check('a malformed body is refused as data', !!bad[0].error, JSON.stringify(bad[0]).slice(0, 60));
  check('a tampered digest is refused', !!bad[1].error, bad[1].error?.code);
  check('a non-http artifact url is refused', !!bad[2].error, bad[2].error?.code);
  check('a path-traversal id is refused', !!bad[3].error, bad[3].error?.code);
  check('removing an absent plugin is a plain not_found', bad[4].error?.code === 'not_found');
  check('a body without a declared content type is refused', bad[5].status >= 400, `status ${bad[5].status}`);
}

check('the hub is alive after all of it', Array.isArray(await c.plugins()));
check('the installed plugin is untouched by the refusals', (await c.plugins()).some((p) => p.id === 'harness-adapter-alpha'));

await hub.stop();
await assets.stop();
console.log(t.failures ? `\n${t.failures} failure(s)` : '\nhostile-requests: all checks passed');
process.exitCode = t.failures ? 1 : 0;
// Let node tear its own sockets down; a forced exit can assert in the runtime on Windows.
