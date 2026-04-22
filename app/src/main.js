// Tauri 2 frontend entry — listens for usage/status events from Rust
// and renders the widget. Uses globals exposed by `withGlobalTauri: true`.

const { event, core } = window.__TAURI__;

const $ = (id) => document.getElementById(id);

let lastPayload = null;
let inFlightRefresh = false;
let refreshTimeoutId = null;

// ---------- formatting ----------
function fmtUntil(ms) {
  const diff = ms - Date.now();
  if (!Number.isFinite(diff) || diff <= 0) return 'expired';
  const totalMin = Math.floor(diff / 60000);
  const d = Math.floor(totalMin / 1440);
  const h = Math.floor((totalMin % 1440) / 60);
  const m = totalMin % 60;
  if (d > 0) return `${d}d ${h}h`;
  if (h > 0) return `${h}h ${m}m`;
  return `${m}m`;
}

function fmtUpdated(ts) {
  const diff = Date.now() - ts;
  if (diff < 45_000) return 'Updated just now';
  const totalMin = Math.floor(diff / 60_000);
  if (totalMin < 60) return `Updated ${totalMin}m ago`;
  const h = Math.floor(totalMin / 60);
  if (h < 24) return `Updated ${h}h ago`;
  const d = Math.floor(h / 24);
  return `Updated ${d}d ago`;
}

function isStale(ts) { return Date.now() - ts > 10 * 60_000; }

function pctClass(p) {
  if (p === 0) return 'zero';
  if (p >= 90) return 'crit';
  if (p >= 70) return 'warn';
  return '';
}

function clockIcon() {
  return `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="9.5"/><polyline points="12 6.5 12 12 16 14"/></svg>`;
}

const LIMIT_DEFS = [
  { key: 'five_hour',        label: 'Session (5h)' },
  { key: 'seven_day',        label: 'Weekly' },
  { key: 'seven_day_opus',   label: 'Opus weekly' },
  { key: 'seven_day_sonnet', label: 'Sonnet weekly' },
];

function renderRow(label, limit) {
  const pct = Math.max(0, Math.min(100, Number(limit.utilization) || 0));
  const resetAt = limit.resets_at ? new Date(limit.resets_at).getTime() : null;
  const cls = pctClass(pct);
  return `
    <div class="row">
      <div class="row-head">
        <span class="label">${label}</span>
        <span class="pct ${cls}">${Math.round(pct)}%</span>
        ${resetAt ? `<span class="reset" title="Resets at ${new Date(resetAt).toLocaleString()}">${clockIcon()}<span data-reset="${resetAt}">${fmtUntil(resetAt)}</span></span>` : ''}
      </div>
      <div class="bar"><div class="fill ${cls}" style="width:${pct}%"></div></div>
    </div>`;
}

function renderWidget(payload) {
  const { data, ts } = payload;
  const widget = $('widget');
  const rows = [];
  for (const def of LIMIT_DEFS) {
    const v = data[def.key];
    if (v && typeof v.utilization === 'number') rows.push(renderRow(def.label, v));
  }
  let extra = '';
  if (data.extra_usage?.is_enabled) {
    const used = Number(data.extra_usage.used_credits) || 0;
    const lim = Number(data.extra_usage.monthly_limit) || 0;
    extra = `<div class="tier"><span class="tier-label">Extra usage</span><strong>$${(used/100).toFixed(2)}</strong><span class="tier-of">of $${(lim/100).toFixed(2)}</span></div>`;
  }
  if (rows.length === 0) {
    widget.innerHTML = `<div class="empty"><div class="empty-title">No limits reported</div><p class="empty-msg">The response didn't contain any quota fields.</p></div>`;
  } else {
    widget.innerHTML = rows.join('') + extra;
  }
  $('updatedText').textContent = fmtUpdated(ts);
  $('updatedBadge').dataset.state = isStale(ts) ? 'stale' : 'fresh';
  $('refreshBtn').disabled = false;
}

function pulseLive() {
  const el = $('updatedBadge');
  el.classList.remove('pulse');
  void el.offsetWidth;
  el.classList.add('pulse');
}

function tickCountdowns() {
  document.querySelectorAll('[data-reset]').forEach(el => {
    const t = Number(el.dataset.reset);
    if (Number.isFinite(t)) el.textContent = fmtUntil(t);
  });
  if (lastPayload) {
    $('updatedText').textContent = fmtUpdated(lastPayload.ts);
    $('updatedBadge').dataset.state = isStale(lastPayload.ts) ? 'stale' : 'fresh';
  }
}

function applyStatus(status) {
  // Status events are advisory — they update the badge / empty state but
  // don't override a successful render.
  if (!status) return;
  if (status.state === 'logged_out') {
    if (!lastPayload) {
      $('emptyState')?.classList.remove('hidden');
      const t = $('emptyState')?.querySelector('.empty-title');
      const m = $('emptyMsg');
      if (t) t.textContent = 'Sign in needed';
      if (m) m.textContent = status.message || 'Sign in to claude.ai to start tracking usage.';
      $('loginBtn').classList.remove('hidden');
    }
    $('updatedBadge').dataset.state = '';
    $('updatedText').textContent = 'Not signed in';
  } else if (status.state === 'error') {
    $('updatedBadge').dataset.state = 'error';
    $('updatedText').textContent = 'Error';
    if (!lastPayload) {
      const m = $('emptyMsg');
      if (m) m.textContent = status.message || 'Something went wrong.';
    }
  } else if (status.state === 'logged_in') {
    // Successful fetch — usage-update event handles the rendering.
  }
}

// ---------- event wiring ----------
event.listen('usage-update', (e) => {
  lastPayload = e.payload;
  renderWidget(lastPayload);
  pulseLive();
  // Clear refresh-in-flight indicator when an update arrives
  inFlightRefresh = false;
  $('refreshBtn').classList.remove('spinning');
  if (refreshTimeoutId) { clearTimeout(refreshTimeoutId); refreshTimeoutId = null; }
});

event.listen('status', (e) => {
  applyStatus(e.payload);
});

$('refreshBtn').addEventListener('click', async () => {
  if (inFlightRefresh) return;
  inFlightRefresh = true;
  $('refreshBtn').classList.add('spinning');
  refreshTimeoutId = setTimeout(() => {
    inFlightRefresh = false;
    $('refreshBtn').classList.remove('spinning');
  }, 8000);
  try { await core.invoke('manual_refresh'); } catch (e) { console.warn(e); }
});

$('loginBtn').addEventListener('click', async () => {
  try { await core.invoke('open_login'); } catch (e) { console.warn(e); }
});

setInterval(tickCountdowns, 30_000);
