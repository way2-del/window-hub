import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { beginScope, checkScope, allowedPath } from '../scripts/checks/plugin-scope.mjs';

test('plugin task allows its own files, detects host edits and deletion, preserves pre-existing dirty work', async () => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), 'wh-plugin-scope-'));
  try {
    execFileSync('git', ['init', '-q'], { cwd: root });
    await fs.mkdir(path.join(root, 'src'));
    await fs.writeFile(path.join(root, 'src/App.tsx'), 'already modified before task');
    await beginScope(root, 'clock');
    await assert.rejects(beginScope(root, 'clock'), /already active/);
    assert.deepEqual((await checkScope(root, 'clock')).outside, []);
    await fs.mkdir(path.join(root, 'src-tauri/resources/plugins/clock'), { recursive: true });
    await fs.writeFile(path.join(root, 'src-tauri/resources/plugins/clock/popup.js'), 'plugin implementation');
    assert.deepEqual((await checkScope(root, 'clock')).outside, []);
    await fs.writeFile(path.join(root, 'src/App.tsx'), 'new host edit');
    assert.deepEqual((await checkScope(root, 'clock', true)).outside, ['src/App.tsx']);
    await fs.unlink(path.join(root, 'src/App.tsx'));
    assert.deepEqual((await checkScope(root, 'clock')).outside, ['src/App.tsx']);
    await fs.writeFile(path.join(root, 'src/App.tsx'), 'already modified before task');
    assert.deepEqual((await checkScope(root, 'clock', true)).outside, []);
    await assert.rejects(checkScope(root, 'clock'), /No active scope/);
    await beginScope(root, 'clock');
    assert.equal(allowedPath('clock', 'src-tauri/src/commands.rs'), false);
    assert.equal(allowedPath('clock', 'plugins/clock/board.tsx'), true);
    assert.equal(allowedPath('clock', 'plugins/clock-other/popup.js'), false);
    assert.throws(() => allowedPath('../clock', 'src/App.tsx'));
  } finally {
    // Exact directory created above, never a caller-supplied removal target.
    await fs.rm(root, { recursive: true, force: true });
  }
});
