#!/usr/bin/env node
// Prepare every plugin's runtime from its own manifest.
//
// A plugin declares the runtime it drives (`runtime.package` + `runtime.version`)
// and the argv that starts it (`runtime.command`). The runtime itself is an
// official release installed from that package — it is a build product, not
// source, so it is never committed; this script materializes it under
// `<plugin>/runtime/`, which is the path the manifest's command is relative to.
//
// The pin the manifest states is the only version this script will install, so
// "which version runs" is answered by the manifest and nothing else.
//
// The harnesses themselves live in their own repositories, so the plugins directory
// is composed by whoever runs the hub: this script prepares whatever it finds there
// and knows no harness by name.
//
// Usage:
//   node scripts/prepare-runtimes.mjs                  # all plugins in PRTS_PLUGINS_DIR
//   node scripts/prepare-runtimes.mjs <id> [<id>...]   # named plugins only
//   node scripts/prepare-runtimes.mjs --plugins <dir>  # a different plugins directory

import fs from 'node:fs';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
let PLUGINS_DIR = process.env.PRTS_PLUGINS_DIR || path.join(HERE, '..', 'plugins');

function readManifest(id) {
  const file = path.join(PLUGINS_DIR, id, 'manifest.json');
  if (!fs.existsSync(file)) throw new Error(`no manifest for plugin '${id}'`);
  return JSON.parse(fs.readFileSync(file, 'utf8'));
}

function pluginIds() {
  if (!fs.existsSync(PLUGINS_DIR)) return [];
  return fs.readdirSync(PLUGINS_DIR)
    .filter((d) => fs.existsSync(path.join(PLUGINS_DIR, d, 'manifest.json')))
    .sort();
}

function npm(args, cwd) {
  // npm is a shell shim on Windows; spawn it through the shell so .cmd resolves.
  return execFileSync('npm', args, { cwd, stdio: 'inherit', windowsHide: true, shell: process.platform === 'win32' });
}

export function preparePlugin(id) {
  const m = readManifest(id);
  const rt = m.runtime;
  if (!rt || !rt.package || !rt.version) {
    console.log(`- ${id}: no runtime declared; nothing to prepare`);
    return false;
  }
  const dir = path.join(PLUGINS_DIR, id, 'runtime');

  // The file the command points at, resolved the same way the hosts resolve it:
  // relative paths in the manifest are relative to the PLUGIN directory, so
  // `runtime/lib/bin.js` means `<plugin>/runtime/lib/bin.js`. Used only to
  // confirm the install produced what the manifest promised.
  const pluginDir = path.join(PLUGINS_DIR, id);
  const rel = Array.isArray(rt.command) ? rt.command.slice(1).find((p) => !p.startsWith('-')) : null;
  const target = rel ? path.resolve(pluginDir, rel) : null;

  if (target && fs.existsSync(target)) {
    console.log(`- ${id}: ${rt.package}@${rt.version} already present`);
    return true;
  }

  console.log(`- ${id}: installing ${rt.package}@${rt.version} -> runtime/`);
  fs.mkdirSync(dir, { recursive: true });

  // A manifest may name a package that lives in a subdirectory. npm installs it
  // flat into this directory; the command's relative path then points into it.
  const spec = `${rt.package}@${rt.version}`;
  if (rt.mode === 'package') {
    // install as a dependency of the runtime dir itself
    if (!fs.existsSync(path.join(dir, 'package.json'))) {
      fs.writeFileSync(path.join(dir, 'package.json'), JSON.stringify({ name: `prts-runtime-${id}`, private: true }, null, 2) + '\n');
    }
    npm(['install', '--omit=dev', '--no-audit', '--no-fund', spec], dir);
  } else {
    // tarball extraction: the release's own layout becomes the runtime layout
    // npm tarballs wrap their contents in a `package/` directory; the release's
    // own layout is what the manifest's command points into, so unwrap it.
    const tgz = execFileSync('npm', ['pack', spec],
      { cwd: dir, encoding: 'utf8', windowsHide: true, shell: process.platform === 'win32' }).trim().split('\n').pop();
    execFileSync('tar', ['xzf', tgz], { cwd: dir, stdio: 'inherit', windowsHide: true });
    const inner = path.join(dir, 'package');
    if (fs.existsSync(inner)) {
      for (const entry of fs.readdirSync(inner)) {
        fs.renameSync(path.join(inner, entry), path.join(dir, entry));
      }
      fs.rmSync(inner, { recursive: true, force: true });
    }
    fs.rmSync(path.join(dir, tgz), { force: true });
    if (fs.existsSync(path.join(dir, 'package.json'))) {
      npm(['install', '--omit=dev', '--no-audit', '--no-fund'], dir);
    }
  }

  if (target && !fs.existsSync(target)) {
    throw new Error(`${id}: manifest command expects ${rel}, but the install did not produce it`);
  }
  console.log(`- ${id}: ready`);
  return true;
}

if (import.meta.url === `file://${process.argv[1]}` || process.argv[1]?.endsWith('prepare-runtimes.mjs')) {
  const argv = process.argv.slice(2);
  const flag = argv.indexOf('--plugins');
  if (flag >= 0) {
    PLUGINS_DIR = path.resolve(argv[flag + 1]);
    argv.splice(flag, 2);
  }
  console.log(`plugins directory: ${PLUGINS_DIR}`);
  const ids = argv.length ? argv : pluginIds();
  if (!ids.length) {
    console.log('no plugin directories with a manifest.json found — nothing to prepare.');
    console.log('point PRTS_PLUGINS_DIR (or --plugins <dir>) at the directory that holds them.');
  }
  for (const id of ids) preparePlugin(id);
}
