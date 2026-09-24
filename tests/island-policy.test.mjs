import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { loadTs } from './helpers/load-ts.mjs';

const load = name => loadTs(fileURLToPath(new URL(`../src/features/island/${name}.ts`, import.meta.url)));
const motion = load('motion');
const { createIslandGeometry } = load('geometry');
const { resolveIslandPullContent } = load('pullContent');

test('gesture progress and animation channels stay clamped, monotonic, and reach endpoints', () => {
  assert.equal(motion.pullProgress(-20), 0);
  assert.equal(motion.pullProgress(210), 1);
  assert.equal(motion.pullProgress(1000), 1);
  assert.equal(motion.channelEase(0, 0.2, 0.8), 0);
  assert.equal(motion.channelEase(1, 0.2, 0.8), 1);
  let previous = 0;
  for (let step = 0; step <= 210; step++) {
    const current = motion.pullProgress(step);
    assert(current >= previous && current <= 1);
    previous = current;
  }
  assert.equal(motion.lerp(28, 220, 0), 28);
  assert.equal(motion.lerp(28, 220, 1), 220);
});

test('geometry preserves short-panel rounding and screen-edge bleed without depending on preferences', () => {
  const g = createIslandGeometry(152);
  assert.equal(g.islandBottomRadius(560, 152), 18);
  assert.equal(g.islandBottomRadius(560, 220), 32);
  assert.equal(createIslandGeometry(220).islandBottomRadius(560, 220), 18);
  for (const [w, h] of [[0, 0], [300, 28], [560, 152], [380, 220]]) {
    assert(!g.islandPath(w, h).includes('NaN'));
    assert(g.islandPath(w, h, 1, 1).startsWith('M 0 -1 L '));
    assert(g.islandPath(w, h).endsWith('Z'));
    assert(g.islandNotifyInnerStrokePath(w, h).startsWith('M -8 0'));
    assert(!g.islandNotifyInnerStrokePath(w, h).endsWith('Z'));
    assert(g.islandNotifyClipSilhouette(w, h, 8, 1).startsWith('M -8 -1'));
    assert(g.islandNotifyClipSilhouette(w, h).endsWith('Z'));
  }
});

test('pull content prioritizes available scenario, then session, then user preference', () => {
  const base = { scenarioOwner: 'clock', scenarioPull: null, sessionOverride: 'plugin:search', sessionOverrideActive: true, pullContent: 'plugin:weather' };
  const enabled = value => value === 'plugin:disabled' ? '' : value;
  assert.equal(resolveIslandPullContent(base, enabled), 'plugin:clock');
  assert.equal(resolveIslandPullContent({ ...base, scenarioPull: 'plugin:disabled' }, enabled), 'plugin:search');
  assert.equal(resolveIslandPullContent({ ...base, scenarioOwner: null, sessionOverrideActive: false }, enabled), 'plugin:weather');
  assert.equal(resolveIslandPullContent({ ...base, scenarioOwner: null, sessionOverride: 'plugin:disabled' }, enabled), 'plugin:weather');
});
