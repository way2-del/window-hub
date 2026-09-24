import fs from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../../', import.meta.url));
const source = path.join(root, 'src-tauri/resources/plugins');
const target = path.join(root, 'docs/plugins/examples');
const [mode, name, ...extra] = process.argv.slice(2);
if (!['check', 'sync'].includes(mode) || !name || extra.length ||
    (!/^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(name) && name !== '--all') || (mode === 'sync' && name === '--all')) {
  console.error('Usage: node scripts/checks/plugin-examples.mjs check <plugin|--all> | sync <plugin>');
  process.exit(2);
}

async function files(dir, prefix = '') {
  let entries;
  try { entries = await fs.readdir(dir, { withFileTypes: true }); }
  catch (error) { if (error.code === 'ENOENT') return []; throw error; }
  const result = [];
  for (const entry of entries) {
    if (entry.isSymbolicLink()) throw new Error(`Refusing symlink: ${path.join(dir, entry.name)}`);
    const relative = path.posix.join(prefix, entry.name);
    if (entry.isDirectory()) result.push(...await files(path.join(dir, entry.name), relative));
    else if (entry.isFile()) result.push(relative);
  }
  return result.sort();
}
async function plainDirectory(dir, optional = false) {
  try {
    const stat = await fs.lstat(dir);
    if (!stat.isDirectory() || stat.isSymbolicLink()) throw new Error(`Not a plain directory: ${dir}`);
  } catch (error) { if (!optional || error.code !== 'ENOENT') throw error; }
}

// No bulk writes or deletions: drift must be reviewed per plugin.
const names = name === '--all'
  ? (await fs.readdir(source, { withFileTypes: true })).filter(e => e.isDirectory()).map(e => e.name).sort()
  : [name];
let drift = 0;
for (const plugin of names) {
  const src = path.join(source, plugin), dst = path.join(target, plugin);
  await plainDirectory(src, name === '--all');
  await plainDirectory(dst, true);
  const sourceFiles = await files(src), targetFiles = await files(dst);
  for (const file of new Set([...sourceFiles, ...targetFiles])) {
    const from = path.join(src, file), to = path.join(dst, file);
    if (!sourceFiles.includes(file)) {
      console.error(`Example-only (review manually): ${plugin}/${file}`); drift++; continue;
    }
    const content = await fs.readFile(from);
    let previous = null;
    try { previous = await fs.readFile(to); } catch (e) { if (e.code !== 'ENOENT') throw e; }
    // Text line endings alone are not semantic drift; binary assets use byte comparison.
    const equal = previous && (content.equals(previous) ||
      (/\.(js|css|html|json|md|svg|txt)$/.test(file) &&
       content.toString('utf8').replaceAll('\r\n', '\n') === previous.toString('utf8').replaceAll('\r\n', '\n')));
    if (equal) continue;
    if (mode === 'sync') {
      await fs.mkdir(path.dirname(to), { recursive: true });
      await fs.writeFile(to, content);
      console.log(`Synced ${plugin}/${file}`);
    } else { console.error(`Different/missing: ${plugin}/${file}`); drift++; }
  }
}
console.log(`Plugin examples ${mode}: ${names.length} plugin(s), ${drift} unresolved difference(s).`);
process.exitCode = drift ? 1 : 0;
