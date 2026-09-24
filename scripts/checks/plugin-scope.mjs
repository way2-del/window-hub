import fs from 'node:fs/promises';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

function validateId(id) {
  if (!/^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(id ?? '')) throw new Error('Use a plugin folder name, e.g. world-clock.');
}
export function allowedPath(id, file) {
  validateId(id);
  return [`src-tauri/resources/plugins/${id}/`, `docs/plugins/examples/${id}/`, `plugins/${id}/`]
    .some(prefix => file.startsWith(prefix));
}

// Compare working-tree content to the start of this task, not to HEAD.
// Pre-existing unrelated changes therefore do not become this task's changes.
export async function snapshot(root) {
  const output = execFileSync('git', ['ls-files', '--cached', '--others', '--exclude-standard', '-z'], { cwd: root });
  const files = [...new Set(output.toString('utf8').split('\0').filter(Boolean))].sort();
  const result = {};
  for (const file of files) {
    if (file.startsWith('workspace/task-scopes/')) continue;
    const absolute = path.resolve(root, file);
    if (!absolute.startsWith(path.resolve(root) + path.sep)) throw new Error(`Path escaped repository: ${file}`);
    try {
      const stat = await fs.lstat(absolute);
      if (stat.isDirectory()) continue;
      const content = stat.isSymbolicLink() ? await fs.readlink(absolute) : await fs.readFile(absolute);
      result[file] = createHash('sha256').update(stat.isSymbolicLink() ? 'symlink:' : 'file:').update(content).digest('hex');
    } catch (error) { if (error.code !== 'ENOENT') throw error; }
  }
  return result;
}

const scopePath = (root, id) => path.join(root, 'workspace/task-scopes', `${id}.json`);
export async function beginScope(root, id) {
  validateId(id);
  const file = scopePath(root, id);
  let previous;
  try { previous = JSON.parse(await fs.readFile(file, 'utf8')); }
  catch (error) { if (error.code !== 'ENOENT') throw error; }
  if (previous?.status === 'active') throw new Error(`Scope ${id} is already active; check/end it before starting another task.`);
  const state = { version: 1, plugin: id, status: 'active', startedAt: new Date().toISOString(), files: await snapshot(root) };
  await fs.mkdir(path.dirname(file), { recursive: true });
  await fs.writeFile(file, JSON.stringify(state, null, 2) + '\n');
}

export async function checkScope(root, id, end = false) {
  validateId(id);
  const file = scopePath(root, id);
  const state = JSON.parse(await fs.readFile(file, 'utf8'));
  if (state.version !== 1 || state.plugin !== id || state.status !== 'active') throw new Error('No active scope for this plugin.');
  const current = await snapshot(root);
  const changed = [...new Set([...Object.keys(state.files), ...Object.keys(current)])]
    .filter(name => state.files[name] !== current[name]).sort();
  const outside = changed.filter(name => !allowedPath(id, name));
  if (end && !outside.length) {
    state.status = 'complete';
    state.finishedAt = new Date().toISOString();
    await fs.writeFile(file, JSON.stringify(state, null, 2) + '\n');
  }
  return { changed, outside };
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [mode, id, ...extra] = process.argv.slice(2);
  try {
    if (!['begin', 'check', 'end'].includes(mode) || extra.length) throw new Error('Usage: npm run plugins:scope -- <begin|check|end> <plugin>');
    const root = fileURLToPath(new URL('../../', import.meta.url));
    if (mode === 'begin') {
      await beginScope(root, id);
      console.log(`Plugin scope started: ${id}. Allowed: this plugin's runtime, example, and authoring directories only.`);
    } else {
      const { changed, outside } = await checkScope(root, id, mode === 'end');
      console.log(`Plugin scope: ${changed.length} changed file(s), ${outside.length} outside scope.`);
      for (const file of outside) console.error(`Outside plugin task: ${file}`);
      process.exitCode = outside.length ? 1 : 0;
    }
  } catch (error) { console.error(error.message); process.exitCode = 1; }
}
