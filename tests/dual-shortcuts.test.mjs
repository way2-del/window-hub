import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { loadTs } from './helpers/load-ts.mjs';

const dual = loadTs(fileURLToPath(new URL('../src/features/chrome/dualShortcuts.ts', import.meta.url)));
const fold = loadTs(fileURLToPath(new URL('../src/features/chrome/shortcutsFold.ts', import.meta.url)));
const geo = loadTs(fileURLToPath(new URL('../src/plugins/shortcutsGeometry.ts', import.meta.url)));

const allOn = {
  showTray: true,
  showTrayMenu: true,
  showWifi: true,
  showClock: true,
  showIme: true,
  showControlCenter: true,
};
const allOff = {
  showTray: false,
  showTrayMenu: false,
  showWifi: false,
  showClock: false,
  showIme: false,
  showControlCenter: false,
};

test('chrome rail tiers: dual / hybrid / tray', () => {
  assert.equal(dual.resolveChromeRailTier(allOff), 'dual');
  assert.equal(dual.resolveChromeRailTier({ ...allOff, showClock: true }), 'hybrid');
  assert.equal(dual.resolveChromeRailTier({ ...allOff, showWifi: true, showIme: true }), 'hybrid');
  assert.equal(dual.resolveChromeRailTier({ ...allOff, showTrayMenu: true }), 'hybrid');
  assert.equal(dual.resolveChromeRailTier({ ...allOff, showTray: true }), 'tray');
  assert.equal(dual.resolveChromeRailTier(allOn), 'tray');
  assert.equal(dual.hasRightShortcutsWing(allOff), true);
  assert.equal(dual.hasRightShortcutsWing({ ...allOff, showClock: true }), true);
  assert.equal(dual.hasRightShortcutsWing({ ...allOff, showTrayMenu: true }), true);
  assert.equal(dual.hasRightShortcutsWing({ ...allOff, showTray: true }), false);
});

test('enteredTrayTier only when crossing into tray icons', () => {
  assert.equal(dual.enteredTrayTier(allOff, { ...allOff, showClock: true }), false);
  assert.equal(dual.enteredTrayTier({ ...allOff, showClock: true }, { ...allOff, showTray: true }), true);
  assert.equal(dual.enteredTrayTier(allOn, allOn), false);
  // Dropdown alone does not enter tray tier
  assert.equal(
    dual.enteredTrayTier(allOff, { ...allOff, showTrayMenu: true }),
    false,
  );
});

test('dual helpers stay consistent with tiers', () => {
  assert.equal(dual.isDualShortcutsMode(allOff), true);
  assert.equal(dual.isDualShortcutsMode({ ...allOff, showClock: true }), false);
  assert.equal(dual.dualShortcutsTransition(allOn, allOff), 'enter');
  assert.equal(dual.countRightChromeModules(allOn), 6);
  assert.equal(dual.needsTraySubsystem({ ...allOff, showTrayMenu: true }), true);
  assert.equal(dual.needsTraySubsystem({ ...allOff, showTray: true }), true);
  assert.equal(dual.needsTraySubsystem(allOff), false);
});

test('right shortcuts zone accounts for chrome strip width', () => {
  const b = geo.computeShortcutsBoundsRight(640, 1280);
  assert.equal(b.x, 640 + geo.SHORTCUTS_ISLAND_CLEARANCE);
  assert.equal(b.x + b.maxExpandWidth, 1280 - geo.SHORTCUTS_RIGHT_INSET);
  const withStrip = geo.computeShortcutsBoundsRight(
    640,
    1280,
    geo.SHORTCUTS_ISLAND_CLEARANCE,
    120,
  );
  assert.ok(withStrip.maxExpandWidth < b.maxExpandWidth);
  assert.equal(
    withStrip.x + withStrip.maxExpandWidth,
    1280 - geo.SHORTCUTS_RIGHT_INSET - 120 - geo.SHORTCUTS_CHROME_STRIP_GAP,
  );
});

test('shortcuts fold packs toward island edges with ⋯ + gap', () => {
  const widths = { a: 40, b: 40, c: 40, d: 40 };
  const ids = (arr) => JSON.stringify([...arr]);
  // budget 140 − chip 28 − gap 7 = 105 → two chips of 40 (+gap)
  const left = fold.planShortcutsFold(['a', 'b', 'c', 'd'], widths, 140, 'left', 28, 7);
  assert.equal(ids(left.visibleIds), ids(['a', 'b']));
  assert.equal(ids(left.overflowIds), ids(['c', 'd']));
  const right = fold.planShortcutsFold(['a', 'b', 'c', 'd'], widths, 140, 'right', 28, 7);
  assert.equal(ids(right.visibleIds), ids(['c', 'd']));
  assert.equal(ids(right.overflowIds), ids(['a', 'b']));
  const fits = fold.planShortcutsFold(['a', 'b'], widths, 200, 'left', 28, 7);
  assert.equal(ids(fits.overflowIds), ids([]));
  const tight = fold.planShortcutsFold(['a', 'b', 'c'], widths, 80, 'left', 28, 7);
  assert.equal(ids(tight.visibleIds), ids(['a']));
  assert.equal(ids(tight.overflowIds), ids(['b', 'c']));
});

const trayFold = loadTs(fileURLToPath(new URL('../src/features/chrome/trayRailFold.ts', import.meta.url)));

test('tray rail fold keeps screen-edge icons and stashes island-facing ones', () => {
  const ids = (arr) => JSON.stringify([...arr]);
  // 3 slots visible from the right end
  const plan = trayFold.planTrayIconFold(['a', 'b', 'c', 'd', 'e'], 3 * trayFold.TRAY_ICON_SLOT_W);
  assert.equal(ids(plan.visibleIds), ids(['c', 'd', 'e']));
  assert.equal(ids(plan.overflowIds), ids(['a', 'b']));
  // Partial slot (≥65% of one glyph with 35% slack) still keeps one — snug under fade
  const partial = trayFold.planTrayIconFold(
    ['a', 'b'],
    Math.ceil(trayFold.TRAY_ICON_SLOT_W * 0.65),
  );
  assert.equal(ids(partial.visibleIds), ids(['b']));
  assert.equal(ids(partial.overflowIds), ids(['a']));
  const empty = trayFold.planTrayIconFold(['a', 'b'], 0);
  assert.equal(ids(empty.visibleIds), ids([]));
  assert.equal(ids(empty.overflowIds), ids(['a', 'b']));
  const nanBudget = trayFold.planTrayIconFold(['a'], Number.NaN);
  assert.equal(ids(nanBudget.overflowIds), ids(['a']));
});

test('tray rail max width shrinks as island widens', () => {
  const narrow = trayFold.computeTrayRailMaxWidth(1280, 300);
  const wide = trayFold.computeTrayRailMaxWidth(1280, 900);
  assert.ok(wide < narrow);
  assert.ok(wide < 200);
  // Snug gap keeps more room than a full fade-width clearance
  const snug = trayFold.computeTrayRailMaxWidth(1280, 560, trayFold.TRAY_ISLAND_SNUG_GAP);
  const loose = trayFold.computeTrayRailMaxWidth(1280, 560, 56);
  assert.ok(snug > loose);
});

const reorder = loadTs(fileURLToPath(new URL('../src/chromeReorder.ts', import.meta.url)));

test('fold menu spliceOverflowOrder keeps visible block and reorders overflow', () => {
  const ids = (arr) => JSON.stringify([...arr]);
  assert.equal(
    ids(reorder.spliceOverflowOrder(['a', 'b', 'c', 'd'], ['d', 'c'])),
    ids(['a', 'b', 'd', 'c']),
  );
  assert.equal(
    ids(reorder.spliceOverflowOrder(['c', 'd', 'a', 'b'], ['d', 'c'])),
    ids(['d', 'c', 'a', 'b']),
  );
  const yHint = reorder.pickDropTargetY(
    40,
    [
      { id: 'a', top: 0, height: 30 },
      { id: 'b', top: 32, height: 30 },
      { id: 'c', top: 64, height: 30 },
    ],
    'a',
  );
  assert.equal(ids([yHint?.toId, yHint?.place]), ids(['b', 'before']));
});
