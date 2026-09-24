import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import ts from 'typescript';
import { imports, cyclicEdges, boundaryViolations } from '../scripts/checks/architecture.mjs';

const source = fs.readFileSync(new URL('../src/app/windowRouting.ts', import.meta.url), 'utf8');
const compiled = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ESNext } }).outputText;
const { resolveWindowKind } = await import(`data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`);

test('native window labels and query aliases retain their routes', () => {
  const cases = [
    ['settings', 'settings', 'settings'], ['tray-popup', 'tray', 'tray'],
    ['status-menu-popup', 'status-menu', 'status-menu'], ['input-lang-popup', 'input-lang', 'input-lang'],
    ['chrome-hover-tip', 'chrome-tip', 'chrome-tip'], ['wifi-popup', 'wifi', 'wifi'],
    ['control-center-popup', 'control-center', 'control-center'],
    ['wifi-auth-popup', 'wifi-auth', 'wifi-auth'], ['plugin-popup', 'plugin-popup', 'plugin-popup'],
    ['plugin-window', 'plugin-window', 'plugin-popup'], ['dock', 'dock', 'dock'],
    ['dock-glass', 'dock-glass', 'dock-glass'], ['island-bar-glass', 'island-bar-glass', 'island-bar-glass'],
    ['dock-icon-editor', 'dock-icon-editor', 'dock-icon-editor'],
  ];
  for (const [label, query, expected] of cases) {
    assert.equal(resolveWindowKind({}, () => label, '?window=unknown'), expected);
    assert.equal(resolveWindowKind({}, () => { throw new Error('no Tauri'); }, `?window=${query}`), expected);
  }
  assert.equal(resolveWindowKind({}, () => 'main', '?window=unknown'), 'island');
});

test('injected flags outrank native/query routes without touching Tauri', () => {
  const flags = [
    ['DOCK_ICON_EDITOR', 'dock-icon-editor'], ['DOCK_GLASS', 'dock-glass'],
    ['ISLAND_BAR_GLASS', 'island-bar-glass'], ['DOCK', 'dock'], ['PLUGIN_POPUP', 'plugin-popup'],
    ['WIFI_AUTH_POPUP', 'wifi-auth'], ['CONTROL_CENTER', 'control-center'], ['WIFI_POPUP', 'wifi'], ['CHROME_HOVER_TIP', 'chrome-tip'],
    ['INPUT_LANG_POPUP', 'input-lang'], ['STATUS_MENU_POPUP', 'status-menu'],
    ['TRAY_POPUP', 'tray'], ['SETTINGS', 'settings'],
  ];
  for (let i = 0; i < flags.length; i++) {
    const input = Object.fromEntries(flags.slice(i).map(([key]) => [`__WH_IS_${key}__`, true]));
    assert.equal(resolveWindowKind(input, () => { assert.fail('must not query Tauri'); }, '?window=dock'), flags[i][1]);
  }
  assert.equal(resolveWindowKind({__WH_IS_DOCK__: false}, () => 'settings', '?window=dock'), 'settings');
});

test('dependency scanner handles runtime imports, re-exports, dynamic imports and ignores types', () => {
  assert.deepEqual(imports(`import type { A } from './types';
    import { type B } from './types2'; export type { C } from './types3';
    import './side-effect'; import { run, type D } from './mixed';
    export { run } from './export'; const x = import('./dynamic');`, 'test.ts'),
  ['./side-effect', './mixed', './export', './dynamic']);
});

test('guards reject cycles and importing a window root into a feature', () => {
  assert.deepEqual(cyclicEdges(new Map([['a', ['b']], ['b', ['c']], ['c', []]])), []);
  assert.equal(cyclicEdges(new Map([['a', ['b']], ['b', ['a']], ['c', ['a']]])).length, 2);
  assert.deepEqual(boundaryViolations(new Map([
    ['src/main.tsx', ['src/App.tsx']], ['src/components/Widget.tsx', ['src/SettingsApp.tsx']],
  ])), ['src/components/Widget.tsx -> src/SettingsApp.tsx']);
  assert.equal(boundaryViolations(new Map([
    ['src/features/chrome/sampleStripBands.ts', ['src/islandPrefs.ts']],
  ])).length, 1);
  assert.equal(boundaryViolations(new Map([
    ['plugins/file-search/board.tsx', ['src/plugins/registry.ts']],
  ])).length, 1);
});

const tokensSource = fs.readFileSync(new URL('../src/features/chrome/tokens.ts', import.meta.url), 'utf8');
const tokensJs = ts.transpileModule(tokensSource, { compilerOptions: { module: ts.ModuleKind.ESNext } }).outputText;
const { srgbLuma, chromeTokens, chromeCssVars } = await import(`data:text/javascript;base64,${Buffer.from(tokensJs).toString('base64')}`);
test('chrome contrast and CSS variables remain independently testable', () => {
  assert.equal(srgbLuma({ r: 0, g: 0, b: 0 }), 0);
  assert.equal(srgbLuma({ r: 255, g: 255, b: 255 }), 1);
  const dark = chromeTokens({ r: 32, g: 32, b: 34 });
  const light = chromeTokens({ r: 250, g: 250, b: 250 });
  assert.equal(dark.scheme, 'dark');
  assert.equal(light.scheme, 'light');
  assert.equal(light.fg, '#000000');
  for (const side of ['left', 'center', 'right']) {
    const vars = chromeCssVars(side, dark);
    assert.equal(vars[`--chrome-${side}-fg`], dark.fg);
    assert(Object.keys(vars).every(key => key.startsWith(`--chrome-${side}-`)));
  }
});
