import { seriesFor, nearestPoint, SAMPLE_MS } from './history-data.js';

const $ = id => document.getElementById(id);
const NS = 'http://www.w3.org/2000/svg';
const fmt = value => new Intl.NumberFormat(undefined, { maximumFractionDigits: 2 }).format(value);
const dateTime = ts => new Date(ts).toLocaleString(undefined, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit', second: '2-digit' });
let provider = localStorage.getItem('codex-usage-enabled') === 'true' ? 'codex' : 'claude';
let mode = 'usage', data = null, points = [], sequence = 0;
let xScale, yScale, selected = null;

function svg(tag, attrs, text) {
  const node = document.createElementNS(NS, tag);
  Object.entries(attrs).forEach(([key, value]) => node.setAttribute(key, value));
  if (text != null) node.textContent = text;
  return node;
}

function setPressed(attribute, value) {
  document.querySelectorAll(`button[data-${attribute}]`).forEach(button => {
    button.setAttribute('aria-pressed', String(button.dataset[attribute] === value));
  });
}

function showDetail(point) {
  selected?.replaceChildren();
  if (!point) return;
  const change = point.delta == null ? 'Change unavailable across a break' : `+${fmt(point.delta)} pp since previous reading`;
  const reset = Number.isFinite(point.reset) ? ` · Resets ${dateTime(point.reset)}` : '';
  $('historyDetail').textContent = `${dateTime(point.ts)} · ${fmt(point.value)}% used · ${change}${reset}`;
  const index = points.indexOf(point);
  $('historyScrubber').value = index;
  $('historyScrubber').setAttribute('aria-valuetext', `${dateTime(point.ts)}, ${fmt(point.value)} percent used, ${change}`);
  if (!selected) return;
  const x = xScale(point.ts), value = mode === 'usage' ? point.value : point.delta;
  selected.append(svg('line', { x1: x, x2: x, y1: 12, y2: 222, class: 'chart-cursor' }));
  if (value != null) selected.append(svg('circle', { cx: x, cy: yScale(value), r: 4, class: 'chart-selected' }));
}

function render() {
  if (!data) return;
  document.body.dataset.provider = provider;
  setPressed('provider', provider);
  setPressed('mode', mode);
  points = seriesFor(data.samples, $('historyLimit').value);
  const latest = points.at(-1);
  const comparable = points.filter(point => point.delta != null);
  $('historyLatest').textContent = latest ? `${fmt(latest.value)}%` : '—';
  $('historyLastTime').textContent = latest ? dateTime(latest.ts) : 'Waiting for a reading';
  $('historyRise').textContent = comparable.length ? `+${fmt(comparable.reduce((sum, point) => sum + point.delta, 0))} pp` : '—';
  $('historyCount').textContent = fmt(points.length);
  $('historyChartTitle').textContent = mode === 'usage' ? 'Quota used (%)' : 'Rise between readings (percentage points)';
  const chart = $('historyChart');
  chart.replaceChildren();
  const top = 12, bottom = 222, left = 48, right = 808;
  const ceiling = mode === 'usage' ? Math.max(100, ...points.map(p => p.value))
    : Math.max(5, ...comparable.map(p => Math.ceil(p.delta / 5) * 5));
  xScale = ts => left + (ts - data.start) / (data.end - data.start) * (right - left);
  yScale = value => bottom - value / ceiling * (bottom - top);
  for (let i = 0; i <= 4; i++) {
    const value = ceiling * i / 4, y = yScale(value);
    chart.append(svg('line', { x1: left, x2: right, y1: y, y2: y, class: 'chart-grid' }));
    chart.append(svg('text', { x: left - 9, y: y + 4, 'text-anchor': 'end' }, fmt(value)));
  }
  for (let i = 0; i <= 4; i++) {
    const ts = data.start + (data.end - data.start) * i / 4;
    const label = new Date(ts).toLocaleString(undefined, Number($('historyRange').value) <= 24
      ? { hour: '2-digit', minute: '2-digit' } : { month: 'short', day: 'numeric' });
    chart.append(svg('text', { x: xScale(ts), y: 248, 'text-anchor': i === 0 ? 'start' : i === 4 ? 'end' : 'middle' }, label));
  }
  if (mode === 'usage') {
    let path = '';
    points.forEach((point, index) => {
      const x = xScale(point.ts), y = yScale(point.value);
      // Step plot: no invented gradual rise or smoothed peaks between observations.
      path += point.connected ? `H${x}V${y}` : `M${x},${y}`;
      if (points.length <= 288 || (!point.connected && !points[index + 1]?.connected)) {
        chart.append(svg('circle', { cx: x, cy: y, r: 2.2, class: 'chart-point' }));
      }
    });
    chart.append(svg('path', { d: path, class: 'chart-line' }));
  } else {
    const width = Math.max(1, Math.min(12, SAMPLE_MS / (data.end - data.start) * (right - left) * 0.75));
    comparable.forEach(point => chart.append(svg('rect', { x: xScale(point.ts) - width / 2,
      y: yScale(point.delta), width, height: Math.max(1, bottom - yScale(point.delta)), class: 'chart-bar' })));
  }
  selected = svg('g', { 'aria-hidden': 'true' });
  chart.append(selected);
  chart.setAttribute('aria-label', `${provider}, ${$('historyLimit').selectedOptions[0]?.textContent || 'quota'}, ${points.length} readings. ${mode === 'usage' ? 'Percentage used' : 'Increase in percentage points'}. Use the slider for details.`);
  const empty = $('historyEmpty');
  empty.classList.toggle('hidden', points.length > 0 && (mode === 'usage' || comparable.length > 0));
  empty.textContent = points.length ? 'A change needs two readings from the same quota cycle, without a gap.'
    : `No ${provider === 'codex' ? 'Codex' : 'Claude'} readings in this range yet. Recording starts with a successful quota refresh.${provider === 'codex' ? ' Enable Show Codex usage in Settings.' : ''}`;
  $('historyScrubber').disabled = !points.length;
  $('historyScrubber').max = Math.max(0, points.length - 1);
  $('historyDetail').textContent = 'Move over the chart or use the slider to inspect a reading.';
  showDetail(latest);
}

async function refresh() {
  const request = ++sequence;
  $('historyRefresh').disabled = true;
  try {
    const result = await window.__TAURI__.core.invoke('read_usage_history', { provider, hours: Number($('historyRange').value) });
    if (request !== sequence) return;
    data = result;
    const limit = $('historyLimit'), previous = limit.value;
    const windows = new Map();
    result.samples.forEach(sample => sample.windows.forEach(row => windows.set(row.id, row.label)));
    limit.replaceChildren(...Array.from(windows, ([id, label]) => {
      const option = document.createElement('option'); option.value = id; option.textContent = label; return option;
    }));
    if (windows.has(previous)) limit.value = previous;
    else if (windows.has(provider === 'codex' ? 'weekly' : 'weekly_all')) limit.value = provider === 'codex' ? 'weekly' : 'weekly_all';
    limit.disabled = !windows.size;
    $('historyWarning').textContent = result.warning || '';
    $('historyWarning').classList.toggle('hidden', !result.warning);
    render();
  } catch (error) {
    if (request !== sequence) return;
    $('historyWarning').textContent = `Could not load history. ${String(error)}`;
    $('historyWarning').classList.remove('hidden');
  } finally {
    if (request === sequence) $('historyRefresh').disabled = false;
  }
}

document.querySelectorAll('button[data-provider]').forEach(button => button.addEventListener('click', () => {
  provider = button.dataset.provider;
  data = null;
  // Clear the old provider while loading so it cannot be mistaken for the new one.
  $('historyChart').replaceChildren();
  $('historyLatest').textContent = $('historyRise').textContent = '—';
  $('historyLastTime').textContent = 'Loading…';
  $('historyCount').textContent = '0';
  $('historyScrubber').disabled = true;
  $('historyDetail').textContent = '';
  $('historyEmpty').classList.remove('hidden');
  $('historyEmpty').textContent = 'Loading history…';
  setPressed('provider', provider);
  void refresh();
}));
document.querySelectorAll('[data-mode]').forEach(button => button.addEventListener('click', () => { mode = button.dataset.mode; render(); }));
$('historyLimit').addEventListener('change', render);
$('historyRange').addEventListener('change', refresh);
$('historyRefresh').addEventListener('click', refresh);
$('historyScrubber').addEventListener('input', event => showDetail(points[Number(event.target.value)]));
$('historyChart').addEventListener('pointermove', event => {
  if (!data || !points.length) return;
  const rect = $('historyChart').getBoundingClientRect();
  const x = (event.clientX - rect.left) / rect.width * 820;
  const ts = data.start + (x - 48) / 760 * (data.end - data.start);
  showDetail(nearestPoint(points, ts));
});
setPressed('provider', provider);
void refresh();
setInterval(() => { if (!document.hidden) void refresh(); }, 60_000);
document.addEventListener('visibilitychange', () => { if (!document.hidden) void refresh(); });
