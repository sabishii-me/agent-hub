// Refresh the fixtures from the hub's own registry. The registry is the authority for
// what is published; this downloads exactly those artifacts and verifies the digests it
// already states. Nothing here invents a plugin - a fixture is the released bytes.
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { fileURLToPath } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const registry = JSON.parse(fs.readFileSync(path.resolve(HERE, '..', '..', 'registry.json'), 'utf8'));

const meta = [];
for (const plugin of registry.plugins) {
  const version = plugin.versions[0];
  const file = `${plugin.pluginType}-${plugin.id}-${version.version}.zip`;
  const dest = path.join(HERE, file);
  process.stdout.write(`${plugin.pluginType}-${plugin.id} ${version.version}: `);
  const answer = await fetch(version.url, { redirect: 'follow' });
  if (!answer.ok) throw new Error(`${version.url} answered ${answer.status}`);
  const bytes = Buffer.from(await answer.arrayBuffer());
  const sha = crypto.createHash('sha256').update(bytes).digest('hex');
  // The registry states the digest; a download that does not match is not that release.
  if (sha !== version.sha256) throw new Error(`${file}: downloaded sha256 ${sha} is not the registry's ${version.sha256}`);
  fs.writeFileSync(dest, bytes);
  meta.push({ id: plugin.id, pluginType: plugin.pluginType, version: version.version, file, url: version.url, sha256: sha, size: bytes.length });
  console.log(`${bytes.length} bytes ok`);
}
fs.writeFileSync(path.join(HERE, 'registry.json'), JSON.stringify(meta, null, 2) + '\n');
console.log(`fixtures: ${meta.length} artifact(s)`);
