import test from 'node:test';
import assert from 'node:assert/strict';
import { seriesFor, nearestPoint, SAMPLE_MS } from '../app/src/history-data.js';

const start = Date.parse('2026-11-27T10:00:00Z');
const reset = '2026-12-01T00:00:00Z';
const sample = (index, value, extra = {}) => ({ ts: start + index * SAMPLE_MS,
  segment: 'login-a', windows: [{ id: 'weekly', used_percent: value, resets_at: reset }], ...extra });

test('flat readings remain zero, increases are percentage points', () => {
  const points = seriesFor([sample(0, 40), sample(1, 40), sample(2, 42.5)], 'weekly');
  assert.deepEqual(points.map(p => p.delta), [null, 0, 2.5]);
  assert.deepEqual(points.map(p => p.connected), [false, true, true]);
});

test('gaps, account changes and quota resets cannot be counted as consumption', () => {
  for (const next of [sample(3, 60), sample(1, 60, { segment: 'login-b' }),
    sample(1, 60, { windows: [{ id: 'weekly', used_percent: 60, resets_at: '2026-12-08T00:00:00Z' }] }),
    sample(1, 5), sample(1, 60, { windows: [{ id: 'weekly', used_percent: 60, resets_at: null }] })]) {
    const points = seriesFor([sample(0, 40), next], 'weekly');
    assert.equal(points[1].delta, null);
    assert.equal(points[1].connected, false);
  }
});

test('absent limits and expired cycles do not produce an interpolated line', () => {
  const missing = seriesFor([sample(0, 40), sample(0.5, 0, { windows: [] }), sample(1, 45)], 'weekly');
  assert.equal(missing.length, 2);
  assert.equal(missing[1].delta, null);
  const expired = [sample(0, 40), sample(1, 45)].map(s => ({ ...s,
    windows: [{ id: 'weekly', used_percent: 40, resets_at: '2026-11-26T00:00:00Z' }] }));
  assert.equal(seriesFor(expired, 'weekly')[1].delta, null);
});

test('nearest reading uses real observation times including edge cases', () => {
  const points = seriesFor([sample(0, 40), sample(1, 42)], 'weekly');
  assert.equal(nearestPoint([], start), null);
  assert.equal(nearestPoint(points, start - 1000), points[0]);
  assert.equal(nearestPoint(points, start + SAMPLE_MS * 0.8), points[1]);
  assert.equal(nearestPoint(points, start + 10 * SAMPLE_MS), points[1]);
});
