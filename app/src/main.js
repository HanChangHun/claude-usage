// Tauri 2 frontend entry — listens for usage/status events from Rust
// and renders the widget. Uses globals exposed by `withGlobalTauri: true`.

const { event, core } = window.__TAURI__;
// Plugin imports (loaded dynamically; available because withGlobalTauri exposes them)
const updaterApi = window.__TAURI__.updater;
const dialogApi = window.__TAURI__.dialog;
const processApi = window.__TAURI__.process;

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
  }
}

// ---------- settings panel ----------
function showSettingsStatus(message, kind = 'info') {
  const el = $('settingsStatus');
  el.textContent = message;
  el.classList.remove('hidden', 'error', 'success');
  if (kind === 'error') el.classList.add('error');
  else if (kind === 'success') el.classList.add('success');
}

function hideSettingsStatus() {
  $('settingsStatus').classList.add('hidden');
}

async function syncAutostartToggle() {
  try {
    const enabled = await core.invoke('is_autostart_enabled');
    $('autostartToggle').checked = !!enabled;
  } catch (e) {
    console.warn('autostart query failed', e);
  }
}

async function checkForUpdates(userInitiated = false) {
  if (!updaterApi) {
    if (userInitiated) showSettingsStatus('Updater plugin unavailable.', 'error');
    return;
  }
  try {
    if (userInitiated) showSettingsStatus('Checking for updates…', 'info');
    const update = await updaterApi.check();
    if (!update) {
      if (userInitiated) showSettingsStatus('You are on the latest version.', 'success');
      return;
    }
    // Update available — confirm with user
    const confirmed = await dialogApi.ask(
      `A new version (v${update.version}) is available.\n\n` +
        (update.body ? `What's new:\n${update.body}\n\n` : '') +
        `Download and install now? The app will restart.`,
      { title: 'Update available', kind: 'info', okLabel: 'Install', cancelLabel: 'Later' }
    );
    if (!confirmed) {
      hideSettingsStatus();
      return;
    }
    showSettingsStatus(`Downloading v${update.version}…`, 'info');
    await update.downloadAndInstall((event) => {
      // event types: { event: 'Started', data: { contentLength } }
      //              { event: 'Progress', data: { chunkLength } }
      //              { event: 'Finished' }
      if (event.event === 'Progress') {
        // Could show a progress bar here in future
      }
    });
    showSettingsStatus('Installed. Restarting…', 'success');
    await processApi.relaunch();
  } catch (e) {
    console.warn('update check failed', e);
    if (userInitiated) showSettingsStatus(`Update failed: ${e?.message || e}`, 'error');
  }
}

// ---------- event wiring ----------
event.listen('usage-update', (e) => {
  lastPayload = e.payload;
  renderWidget(lastPayload);
  pulseLive();
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

// Settings toggle visibility
$('settingsBtn').addEventListener('click', () => {
  const panel = $('settingsPanel');
  panel.classList.toggle('hidden');
  if (!panel.classList.contains('hidden')) {
    syncAutostartToggle();
  }
});

$('autostartToggle').addEventListener('change', async (e) => {
  try {
    const newState = await core.invoke('toggle_autostart_cmd');
    $('autostartToggle').checked = !!newState;
    showSettingsStatus(
      newState ? 'Will start with Windows.' : 'Disabled startup with Windows.',
      'success'
    );
    setTimeout(hideSettingsStatus, 2500);
  } catch (err) {
    showSettingsStatus(`Failed: ${err?.message || err}`, 'error');
    // Revert toggle
    e.target.checked = !e.target.checked;
  }
});

$('signOutBtn').addEventListener('click', async () => {
  try {
    const ok = await dialogApi.confirm(
      'This will sign out of claude.ai and clear the session in this app. ' +
        'Usage tracking will pause until you sign in again.',
      { title: 'Sign out?', kind: 'warning' }
    );
    if (!ok) return;
    await core.invoke('sign_out_cmd');
    showSettingsStatus('Signed out. Use the claude.ai window to sign in again.', 'info');
  } catch (e) {
    showSettingsStatus(`Sign out failed: ${e?.message || e}`, 'error');
  }
});

$('checkUpdateBtn').addEventListener('click', () => checkForUpdates(true));

// ---------- init ----------
setInterval(tickCountdowns, 30_000);

// Initial autostart state sync (without showing the panel)
syncAutostartToggle();

// Silent update check on startup (delay 5s so the user sees their data first)
setTimeout(() => checkForUpdates(false), 5000);
