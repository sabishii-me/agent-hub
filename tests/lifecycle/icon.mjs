// @since hub 0.1.6 / contract v1
// An UNINSTALLED plugin still shows a mark. Its directory does not exist yet, so its
// icon cannot come from disk - it comes from the URL its catalog entry names. This is
// the shape that broke silently: a route addressed by the plugin's storage key
// (`<kind>-<id>`) looked the entry up by its `id` alone (the plugin's own name), found
// nothing, and answered 404 for every mark a person had not installed yet.
import http from 'node:http';
import { Hub, release, releaseRegistry, client, sleep, tally, contractStamp } from '../lib/hub-harness.mjs';

const stamp = contractStamp();
console.log(`lifecycle-icon against contract protocol=${stamp.protocol} version=${stamp.version} sha=${stamp.contractSha} build=${stamp.buildId}`);
const t = tally();
const check = t.check;

// A tiny server that stands in for a release host: it serves the icon bytes the registry
// names. The URL in the registry entry points HERE, so the test exercises the hub's own
// fetch-and-relay without touching the network.
const ICON = Buffer.from('<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"></svg>');
const icons = http.createServer((req, res) => {
  if (!req.url.endsWith('.svg')) { res.writeHead(404); return res.end(); }
  res.writeHead(200, { 'content-type': 'image/svg+xml', 'content-length': ICON.length });
  res.end(ICON);
});
await new Promise((r) => icons.listen(0, '127.0.0.1', r));
const iconBase = `http://127.0.0.1:${icons.address().port}`;

// The hub's registry, with each real entry's icon URL repointed at this local server -
// the only change; the ids, kinds and structure are the published ones.
const registry = releaseRegistry().map((e) => ({
  id: e.id,
  pluginType: e.pluginType,
  name: e.id,
  icon: { light: `${iconBase}/${e.pluginType}-${e.id}.svg`, dark: `${iconBase}/${e.pluginType}-${e.id}.svg` },
  versions: [{ version: e.version, url: `${iconBase}/${e.file}`, sha256: e.sha256, size: e.size }],
}));

const hub = new Hub(undefined, { registry });
const ep = await hub.endpoint();
const c = client(ep);

const KEY = (e) => `${e.pluginType}-${e.id}`;

console.log('an uninstalled plugin still shows its mark');
{
  const installed = await c.plugins();
  check('nothing is installed yet', installed.length === 0, `${installed.length}`);
  for (const e of releaseRegistry()) {
    const r = await fetch(`${c.base}/v1/hub/plugins/${encodeURIComponent(KEY(e))}/icon/light`, { headers: c.H });
    check(`${KEY(e)}: the icon is served from its catalog entry`, r.status === 200 && (r.headers.get('content-type') || '').includes('svg'), `status ${r.status}`);
  }
}

console.log('an unknown plugin has no mark, and says so as a 404');
{
  const r = await fetch(`${c.base}/v1/hub/plugins/no-such-plugin/icon/light`, { headers: c.H });
  check('an id nobody knows is a 404', r.status === 404, `status ${r.status}`);
}

console.log('once installed, the mark comes from the plugin on disk');
{
  const e = releaseRegistry()[0];
  await c.install({ id: e.id, pluginType: e.pluginType, version: e.version, url: `${iconBase}/${e.file}`, sha256: e.sha256, size: e.size });
  // The real manifest declares its own icons (logo.svg beside it); let the runtime settle.
  await sleep(300);
  const r = await fetch(`${c.base}/v1/hub/plugins/${encodeURIComponent(KEY(e))}/icon/light`, { headers: c.H });
  check('the installed plugin still serves a mark', r.status === 200, `status ${r.status}`);
}

await hub.stop();
icons.close();
console.log(t.failures ? `\n${t.failures} failure(s)` : '\nlifecycle-icon: all checks passed');
process.exitCode = t.failures ? 1 : 0;
