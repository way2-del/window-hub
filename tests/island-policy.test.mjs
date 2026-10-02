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

test('pull content prioritizes available scenario, then session, then host home', () => {
  const base = {
    scenarioOwner: 'clock',
    scenarioPull: null,
    sessionOverride: 'plugin:search',
    sessionOverrideActive: true,
  };
  const enabled = (value) => (value === 'plugin:disabled' ? '' : value);
  assert.equal(resolveIslandPullContent(base, enabled), 'plugin:clock');
  assert.equal(
    resolveIslandPullContent({ ...base, scenarioPull: 'plugin:disabled' }, enabled),
    'plugin:search',
  );
  assert.equal(
    resolveIslandPullContent(
      { ...base, scenarioOwner: null, sessionOverrideActive: false },
      enabled,
    ),
    'home',
  );
  assert.equal(
    resolveIslandPullContent(
      { ...base, scenarioOwner: null, sessionOverride: 'plugin:disabled' },
      enabled,
    ),
    'home',
  );
});

test('home left card prefers bar resident weather over fallback', () => {
  const { resolveHomeDashboardLeftPluginId } = loadTs(
    fileURLToPath(new URL('../src/plugins/panelPullMode.ts', import.meta.url)),
  );
  assert.equal(
    resolveHomeDashboardLeftPluginId({
      forcedDashboardPluginId: 'com.window-hub.now-playing',
      barResidentId: 'com.window-hub.weather',
      barResidentPullMode: 'dashboard',
    }),
    'com.window-hub.now-playing',
  );
  assert.equal(
    resolveHomeDashboardLeftPluginId({
      forcedDashboardPluginId: null,
      barResidentId: 'com.window-hub.weather',
      barResidentPullMode: 'dashboard',
    }),
    'com.window-hub.weather',
  );
  assert.equal(
    resolveHomeDashboardLeftPluginId({
      forcedDashboardPluginId: null,
      barResidentId: 'com.window-hub.weather',
      barResidentPullMode: 'standalone',
    }),
    null,
  );
  assert.equal(
    resolveHomeDashboardLeftPluginId({
      forcedDashboardPluginId: null,
      barResidentId: '',
      barResidentPullMode: 'dashboard',
    }),
    null,
  );
});

test('panel pullMode defaults and listing for dashboard rail', () => {
  const { resolvePanelPullMode, listDashboardPanelProviders } = loadTs(
    fileURLToPath(new URL('../src/plugins/panelPullMode.ts', import.meta.url)),
  );
  assert.equal(
    resolvePanelPullMode({ slots: { 'island.panel': { pullMode: 'dashboard' } } }),
    'dashboard',
  );
  assert.equal(
    resolvePanelPullMode({ slots: { 'island.panel': { pullMode: 'standalone' } } }),
    'standalone',
  );
  assert.equal(
    resolvePanelPullMode({
      slots: { 'island.panel': { excludeFromPullContent: true } },
    }),
    'standalone',
  );
  assert.equal(resolvePanelPullMode({ slots: { 'island.panel': {} } }), 'dashboard');

  const listed = listDashboardPanelProviders([
    {
      id: 'a',
      name: 'A',
      entry: { panel: 'panel.html' },
      slots: { 'island.panel': { pullMode: 'dashboard' } },
    },
    {
      id: 'b',
      name: 'B',
      entry: { panel: 'panel.html' },
      slots: { 'island.panel': { excludeFromPullContent: true } },
    },
    {
      id: 'c',
      name: 'C',
      entry: { panel: 'panel.html' },
      slots: {
        'island.scenario': { order: 1 },
        'island.panel': { excludeFromPullContent: true, pullMode: 'dashboard' },
      },
    },
    {
      id: 'a__dev',
      name: 'A',
      entry: { panel: 'panel.html' },
      slots: { 'island.panel': { pullMode: 'dashboard' } },
    },
  ]);
  assert.equal(
    listed.map((p) => p.id).join(","),
    "c,a",
  );
});
