import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import ts from 'typescript';

const source = fs.readFileSync(new URL('../src/features/tray/glyphQueue.ts', import.meta.url), 'utf8');
const compiled = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 } }).outputText;
const { createGlyphQueue } = await import(`data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`);
const flush = async () => { await Promise.resolve(); await Promise.resolve(); };

test('streams bounded batches and does not starve later icons behind missing glyphs', async t => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  const requests = [], published = [];
  const queue = createGlyphQueue(async ids => {
    requests.push(ids);
    return Object.fromEntries(ids.filter(id => id !== 'missing').map(id => [id, 'png']));
  }, map => published.push(...Object.keys(map)));
  queue.update(['missing', ...Array.from({ length: 13 }, (_, i) => String(i))]);
  for (let i = 0; i < 3; i++) { t.mock.timers.tick(100); await flush(); }
  assert.equal(published.length, 13);
  assert.ok(requests.every(ids => ids.length <= 6));
  for (let i = 0; i < 5; i++) { t.mock.timers.tick(1500); await flush(); }
  assert.equal(requests.flat().filter(id => id === 'missing').length, 3);
  queue.dispose();
});

test('retains updates during an in-flight batch; removed icons and disposed results stay out', async t => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  const requests = [], published = [];
  let resolve;
  const queue = createGlyphQueue(ids => {
    requests.push(ids);
    return new Promise(done => { resolve = done; });
  }, map => published.push(map));
  queue.update(['old']);
  t.mock.timers.tick(100);
  queue.update(['new']);
  t.mock.timers.tick(1000);
  assert.equal(requests.length, 1);
  resolve({ old: 'png' });
  await flush();
  assert.deepEqual(published, []);
  t.mock.timers.tick(100);
  assert.deepEqual(requests, [['old'], ['new']]);
  queue.dispose();
  resolve({ new: 'png' });
  await flush();
  assert.deepEqual(published, []);
});
