const STORAGE_KEY = 'antigravity-usage-enabled';

const WINDOWS = [
  { key: '5h', label: 'Gemini session (5h)' },
  { key: 'weekly', label: 'Gemini weekly' },
];

function normalizeResetAt(value) {
  if (typeof value !== 'string' || !value.trim()) return null;
  const ms = Date.parse(value);
  // agy reports Unix epoch (1970-01-01T00:00:00Z) when a quota window has not started.
  if (!Number.isFinite(ms) || ms <= 86_400_000) return null;
  return new Date(ms).toISOString();
}

function isGeminiGroup(group) {
  const name = typeof group?.name === 'string' ? group.name : '';
  if (/gemini/i.test(name)) return true;
  const buckets = Array.isArray(group?.buckets) ? group.buckets : [];
  return buckets.some(bucket => String(bucket?.id || '').toLowerCase().startsWith('gemini-'));
}

export function antigravityLimits(usage) {
  const groups = Array.isArray(usage?.groups) ? usage.groups : [];
  const gemini = groups.find(isGeminiGroup);
  if (!gemini || !Array.isArray(gemini.buckets)) return [];
  const buckets = gemini.buckets.filter(bucket => Number.isFinite(bucket?.remainingFraction));
  return WINDOWS.flatMap(({ key, label }) => {
    const bucket = buckets.find(
      item => String(item?.window || '').trim().toLowerCase() === key
    );
    if (!bucket) return [];
    const remaining = Math.max(0, Math.min(1, bucket.remainingFraction));
    return [{
      label,
      utilization: (1 - remaining) * 100,
      resets_at: normalizeResetAt(bucket.resetTime),
    }];
  });
}

export async function initAntigravity({ renderRow }) {
  const get = id => document.getElementById(id);
  const toggle = get('antigravityToggle');
  const status = get('antigravityStatus');
  const rows = get('antigravityRows');
  const section = get('antigravitySection');
  let enabled = localStorage.getItem(STORAGE_KEY) === 'true';
  let generation = 0;
  let inFlight = false;
  // Tell the local API whether Antigravity is tracked ("disabled" vs. "pending").
  const track = on => window.__TAURI__.core
    .invoke('set_provider_tracking', { provider: 'antigravity', enabled: on })
    .catch(error => console.warn('antigravity tracking', error));

  async function refresh() {
    if (!enabled || inFlight) return;
    inFlight = true;
    const requestGeneration = generation;
    status.textContent = 'Connecting to Antigravity…';
    status.classList.toggle('hidden', rows.childElementCount > 0);
    status.dataset.state = '';
    try {
      const usage = await window.__TAURI__.core.invoke('read_antigravity_usage');
      if (!enabled || requestGeneration !== generation) return;
      const limits = antigravityLimits(usage);
      rows.innerHTML = limits.map(limit => renderRow(limit.label, limit)).join('');
      status.classList.toggle('hidden', limits.length > 0);
      status.textContent = 'No Antigravity limits reported. Check your Google login in agy, then use the top refresh button.';
    } catch (error) {
      if (!enabled || requestGeneration !== generation) return;
      rows.replaceChildren();
      status.classList.remove('hidden');
      status.dataset.state = 'error';
      status.textContent = typeof error === 'string' ? error : 'Could not refresh Antigravity. Please retry.';
    } finally {
      inFlight = false;
      if (enabled && requestGeneration !== generation) void refresh();
    }
  }

  toggle.checked = enabled;
  section.classList.toggle('hidden', !enabled);
  toggle.addEventListener('change', () => {
    enabled = toggle.checked;
    generation += 1;
    localStorage.setItem(STORAGE_KEY, String(enabled));
    void track(enabled);
    section.classList.toggle('hidden', !enabled);
    rows.replaceChildren();
    status.textContent = '';
    status.classList.add('hidden');
    status.dataset.state = '';
    if (enabled) void refresh();
  });
  await track(enabled);
  await window.__TAURI__.event.listen('antigravity-refresh', refresh);
  if (enabled) void refresh();
}
