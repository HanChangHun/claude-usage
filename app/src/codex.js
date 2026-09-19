// Codex reports up to two windows per bucket: a 5-hour session window (Plus
// and similar plans) and the 7-day weekly window. A row is shown only for a
// window the account actually reports, so plans without a session limit keep
// the single weekly row.
const WINDOWS = [
  { minutes: 300, label: 'Codex session (5h)' },
  { minutes: 10080, label: 'Codex weekly' },
];

export function codexLimits(usage) {
  const bucket = usage.rateLimitsByLimitId == null
    ? usage.rateLimits
    : usage.rateLimitsByLimitId.codex;
  if (!bucket || (bucket.limitId && bucket.limitId !== 'codex')) return [];
  const windows = [bucket.primary, bucket.secondary]
    .filter(window => Number.isFinite(window?.usedPercent));
  return WINDOWS.flatMap(({ minutes, label }) => {
    const window = windows.find(window => window.windowDurationMins === minutes);
    return window ? [{
      label,
      utilization: Math.max(0, Math.min(100, window.usedPercent)),
      resets_at: window.resetsAt == null ? null : new Date(window.resetsAt * 1000).toISOString(),
    }] : [];
  });
}

export async function initCodex({ renderRow }) {
  const get = id => document.getElementById(id);
  const toggle = get('codexToggle');
  const status = get('codexStatus');
  const rows = get('codexRows');
  let enabled = localStorage.getItem('codex-usage-enabled') === 'true';
  let generation = 0;
  let inFlight = false;

  async function refresh() {
    if (!enabled || inFlight) return;
    inFlight = true;
    const requestGeneration = generation;
    status.textContent = 'Connecting to Codex…';
    status.classList.toggle('hidden', rows.childElementCount > 0);
    status.dataset.state = '';
    try {
      const usage = await window.__TAURI__.core.invoke('read_codex_usage');
      if (!enabled || requestGeneration !== generation) return;
      const limits = codexLimits(usage);
      rows.innerHTML = limits.map(limit => renderRow(limit.label, limit)).join('');
      status.classList.toggle('hidden', limits.length > 0);
      status.textContent = 'No Codex limits reported. Check your ChatGPT login in Codex CLI, then use the top refresh button.';
    } catch (error) {
      if (!enabled || requestGeneration !== generation) return;
      rows.replaceChildren();
      status.classList.remove('hidden');
      status.dataset.state = 'error';
      status.textContent = typeof error === 'string' ? error : 'Could not refresh Codex. Please retry.';
    } finally {
      inFlight = false;
      if (enabled && requestGeneration !== generation) void refresh();
    }
  }

  toggle.checked = enabled;
  get('codexSection').classList.toggle('hidden', !enabled);
  toggle.addEventListener('change', () => {
    enabled = toggle.checked;
    generation += 1;
    localStorage.setItem('codex-usage-enabled', String(enabled));
    get('codexSection').classList.toggle('hidden', !enabled);
    rows.replaceChildren();
    if (enabled) void refresh();
  });
  get('settingsBtn').addEventListener('click', () => {
    const panel = get('settingsPanel');
    if (!panel.classList.contains('hidden')) panel.scrollIntoView({ block: 'nearest' });
  });
  await window.__TAURI__.event.listen('codex-refresh', refresh);
  void refresh();
}
