import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';

test('scoped sync detects drift, updates only selected plugin, and never deletes example-only files', async () => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), 'wh-plugin-sync-'));
  try {
    await fs.mkdir(path.join(root, 'scripts/checks'), { recursive: true });
    await fs.copyFile(new URL('../scripts/checks/plugin-examples.mjs', import.meta.url), path.join(root, 'scripts/checks/plugin-examples.mjs'));
    for (const plugin of ['clock', 'other']) {
      for (const location of ['src-tauri/resources/plugins', 'docs/plugins/examples']) {
        await fs.mkdir(path.join(root, location, plugin), { recursive: true });
      }
      await fs.writeFile(path.join(root, 'src-tauri/resources/plugins', plugin, 'main.js'), 'new\n');
      await fs.writeFile(path.join(root, 'docs/plugins/examples', plugin, 'main.js'), 'old\n');
    }
    const run = (...args) => spawnSync(process.execPath, ['scripts/checks/plugin-examples.mjs', ...args], { cwd: root, encoding: 'utf8' });
    assert.equal(run('check', 'clock').status, 1);
    assert.equal(run('sync', '--all').status, 2);
    assert.equal(run('sync', '../other').status, 2);
    assert.equal(run('sync', 'clock').status, 0);
    assert.equal(run('check', 'clock').status, 0);
    assert.equal(await fs.readFile(path.join(root, 'docs/plugins/examples/other/main.js'), 'utf8'), 'old\n');
    const extra = path.join(root, 'docs/plugins/examples/clock/README.md');
    await fs.writeFile(extra, 'example documentation');
    assert.equal(run('sync', 'clock').status, 1);
    assert.equal(await fs.readFile(extra, 'utf8'), 'example documentation');
    await fs.writeFile(path.join(root, 'docs/plugins/examples/other/main.js'), 'new\r\n');
    assert.equal(run('check', 'other').status, 0);
  } finally {
    // This exact path was created by mkdtemp in the OS temporary directory.
    await fs.rm(root, { recursive: true, force: true });
  }
});
