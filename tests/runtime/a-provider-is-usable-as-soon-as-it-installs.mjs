// A model provider is only useful if the hub REGISTERS its type. The plugin can read
// `ready`, its row can offer Remove, and none of that makes a provider buildable: the
// type a user picks in the form comes from the plugin's module, loaded into the hub.
//
// It was loaded once, at boot, so a provider installed while the hub ran registered
// nothing and could not be used until a restart. This drives the real surface: install
// the real published provider through a real hub and require its type AT ONCE.
import fs from 'node:fs';
import { Hub, ReleaseServer, publishedRegistry, sleep, tally } from '../lib/hub.mjs';

const t = tally();
const reg = await publishedRegistry();
const prov = reg.plugins.find((p) => p.pluginType === 'model-provider' && p.id === 'compatible');
if (!prov) throw new Error('the published registry has no compatible model-provider');

const rel = await new ReleaseServer().start();
const asset = await rel.stage(prov);
const key = `${prov.pluginType}-${prov.id}`;
console.log(`provider: ${key}@${asset.version} from ${prov.versions[0].url}`);

const hub = new Hub();
await hub.start();
try {
  const base = `http://127.0.0.1:${hub.ep.port}`;
  const H = { authorization: `Bearer ${hub.ep.token}`, 'content-type': 'application/json' };
  const types = async () => (await (await fetch(`${base}/v1/hub/provider-types`, { headers: H })).json());

  const before = await types();
  t.check((before.types || []).length === 0, 'a hub with no provider installed offers no type', JSON.stringify(before.types));

  const ins = await hub.api().install(asset);
  t.check(!!ins.plugin, 'the published provider installs', JSON.stringify(ins).slice(0, 100));

  // The claim is about the moment AFTER install: no restart, no second request that
  // "wakes" anything. Ask the provider surface directly.
  let offered = null;
  for (let i = 0; i < 60; i++) { const now = await types(); if ((now.types || []).length) { offered = now; break; } await sleep(500); }
  const ids = (offered?.types || []).map((x) => `${x.id}@${x.version}`);
  t.check(ids.includes('custom-compatible@1'), 'its type is offered immediately, without a restart', ids.join(',') || '(none)');

  // And the type is really usable: a provider can be created that names it.
  const made = await (await fetch(`${base}/v1/hub/providers`, { method: 'POST', headers: H, body: JSON.stringify({ label: 't', url: 'http://127.0.0.1:9/v1', providerType: 'custom-compatible', providerTypeVersion: 1 }) })).json();
  t.check(made.provider && made.provider.providerTypeAvailable === true, 'a provider can be built on the freshly installed type', JSON.stringify(made).slice(0, 120));

  // Removing it takes the type away with it.
  await hub.api().remove(key);
  let gone = null;
  for (let i = 0; i < 60; i++) { const now = await types(); if (!(now.types || []).length) { gone = now; break; } await sleep(500); }
  t.check(!!gone, 'removing the provider withdraws its type', JSON.stringify(gone || (await types())).slice(0, 100));
} finally {
  await hub.stop();
  await rel.stop();
}

console.log(t.failures ? `${t.failures} failure(s)` : 'the provider is usable the moment it installs, and gone when removed');
process.exit(t.failures ? 1 : 0);
