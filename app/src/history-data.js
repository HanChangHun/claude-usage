// Only compare consecutive observations of the same quota cycle and login.
// Missing samples, resets and decreases must never look like consumption.
export const SAMPLE_MS = 5 * 60 * 1000;
export const MAX_GAP_MS = 8 * 60 * 1000;

export function seriesFor(samples, id) {
  let previous = null;
  return samples.flatMap(sample => {
    const row = sample.windows.find(window => window.id === id);
    if (!row || !Number.isFinite(row.used_percent)) {
      previous = null;
      return [];
    }
    const point = { ts: sample.ts, segment: sample.segment, value: row.used_percent,
      reset: row.resets_at ? Date.parse(row.resets_at) : NaN, delta: null, connected: false };
    if (previous) {
      const elapsed = point.ts - previous.ts;
      const sameCycle = Number.isFinite(point.reset) && point.reset === previous.reset
        && point.reset > point.ts;
      point.connected = elapsed > 0 && elapsed <= MAX_GAP_MS
        && point.segment === previous.segment && sameCycle && point.value >= previous.value;
      if (point.connected) point.delta = point.value - previous.value;
    }
    previous = point;
    return [point];
  });
}

export function nearestPoint(points, ts) {
  if (!points.length) return null;
  let low = 0, high = points.length - 1;
  while (low < high) {
    const middle = Math.floor((low + high) / 2);
    if (points[middle].ts < ts) low = middle + 1;
    else high = middle;
  }
  return low > 0 && ts - points[low - 1].ts < points[low].ts - ts ? points[low - 1] : points[low];
}
