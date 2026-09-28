import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { loadTs } from './helpers/load-ts.mjs';

const place = loadTs(
  fileURLToPath(new URL('../src/features/chrome/popupPlacement.ts', import.meta.url)),
);

const work = { left: 0, top: 0, right: 1280, bottom: 800 };

test('right-edge popup shifts left to stay on screen', () => {
  const p = place.fitPopupOrigin(1100, 40, 320, 480, work);
  assert.equal(p.x, 1280 - 8 - 320);
  assert.equal(p.y, 40);
});

test('bottom overflow flips above trigger when room allows', () => {
  // flipY = 580 - 8 - 200 = 372, which fits in the work area
  const p = place.fitPopupOrigin(100, 600, 320, 200, work, {
    flipAboveY: 580,
    flipGap: 8,
  });
  assert.equal(p.y, 580 - 8 - 200);
  assert.ok(p.y + 200 <= 580);
});

test('placePopupNearRect right-aligns when left-align would overflow', () => {
  const trigger = { left: 1200, top: 0, right: 1260, bottom: 28 };
  const p = place.placePopupNearRect(trigger, 320, 480, work, 8);
  assert.ok(p.x + 320 <= work.right - 8 + 0.01);
  assert.ok(p.x <= trigger.right);
});
